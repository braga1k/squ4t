//! The squat ledger: a port of `store.py` (omarchy-squat-break bff0771), function for function.
//!
//! No I/O here. [`run`] receives the ledger file as it is on disk (or `None` when there is
//! none) and returns what to show and, when the state changed, the exact bytes to write.
//! The platform layer adds the lock and the atomic replacement.

use crate::pyjson::{self, dumps, has_surrogate, loads, Num, Value};
use crate::unicode_tables::{DIGIT_ZEROS, SPACES};

pub const MAX_TOTAL: i128 = 9_000_000_000_000;
pub const MAX_LEDGER_BYTES: u64 = 2_000_000;
pub const ACTIONS: [&str; 8] = ["tick", "status", "add", "undo", "pause", "resume", "snooze", "settings"];

/// What store.py prints on success.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    pub today: i128,
    pub total: i128,
    pub interval: i128,
    pub suggested: i128,
    pub paused: bool,
    pub due: bool,
    pub remaining: i128,
    pub alert: bool,
    pub can_undo: bool,
    /// The seven most recent days, newest first.
    pub history: Vec<(String, i128)>,
}

impl Report {
    /// The line `print(json.dumps(result))` produces, keys in store.py's order.
    pub fn to_json(&self) -> String {
        let history: Vec<String> = self.history.iter().map(|(d, c)| format!("{{\"day\": \"{d}\", \"count\": {c}}}")).collect();
        format!(
            "{{\"today\": {}, \"total\": {}, \"interval\": {}, \"suggested\": {}, \"paused\": {}, \"due\": {}, \"remaining\": {}, \"alert\": {}, \"canUndo\": {}, \"history\": [{}]}}",
            self.today,
            self.total,
            self.interval,
            self.suggested,
            self.paused,
            self.due,
            self.remaining,
            self.alert,
            self.can_undo,
            history.join(", ")
        )
    }
}

/// store.py refused the operation. It prints the same message for every cause,
/// so the cause is only for the log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refused(pub &'static str);

pub const REFUSED_MESSAGE: &str = "{\"error\": \"Could not read or save squat ledger\"}";

type Step<T> = Result<T, Refused>;

fn integer(value: Option<&Value>, low: i128, high: i128) -> Step<i128> {
    match value {
        Some(Value::Int(v)) if (low..=high).contains(v) => Ok(*v),
        Some(_) => Err(Refused("Invalid integer")),
        None => Err(Refused("missing key")),
    }
}

fn timestamp(value: Option<&Value>) -> Step<Num> {
    let ok = |n: Num| Num::Int(0).py_cmp(n).is_some_and(|o| o.is_le()) && n.py_cmp(Num::Float(1e13)).is_some_and(|o| o.is_le());
    match value {
        Some(Value::Int(i)) if ok(Num::Int(*i)) => Ok(Num::Int(*i)),
        Some(Value::Float(f)) if f.is_finite() && ok(Num::Float(*f)) => Ok(Num::Float(*f)),
        Some(_) => Err(Refused("Invalid timestamp")),
        None => Err(Refused("missing key")),
    }
}

/// `dt.date.fromisoformat(day).isoformat() == day`: exactly YYYY-MM-DD, a real date, year 1-9999.
pub fn is_iso_day(day: &[u8]) -> bool {
    if day.len() != 10 || day[4] != b'-' || day[7] != b'-' {
        return false;
    }
    let num = |r: std::ops::Range<usize>| -> Option<u32> { day[r].iter().try_fold(0u32, |acc, &b| b.is_ascii_digit().then(|| acc * 10 + (b - b'0') as u32)) };
    let (Some(y), Some(m), Some(d)) = (num(0..4), num(5..7), num(8..10)) else { return false };
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let days = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    y >= 1 && (1..=days).contains(&d)
}

fn days_of(s: &Value) -> Step<&Vec<(Vec<u8>, Value)>> {
    match s.get("days") {
        Some(Value::Object(items)) => Ok(items),
        Some(_) => Err(Refused("Invalid daily ledger")),
        None => Err(Refused("missing key")),
    }
}

fn sum_days(s: &Value) -> i128 {
    match s.get("days") {
        Some(Value::Object(items)) => items.iter().map(|(_, v)| if let Value::Int(i) = v { *i } else { 0 }).sum(),
        _ => 0,
    }
}

pub fn validate(s: &Value) -> Step<()> {
    let version_is_one = matches!(s.get("version"), Some(Value::Int(1) | Value::Bool(true))) || matches!(s.get("version"), Some(Value::Float(f)) if *f == 1.0);
    if !matches!(s, Value::Object(_)) || !version_is_one {
        return Err(Refused("Unsupported ledger"));
    }
    integer(s.get("interval"), 1, 1440)?;
    integer(s.get("suggested"), 1, 1000)?;
    let paused = s.get("paused").ok_or(Refused("missing key"))?;
    if !matches!(paused, Value::Bool(_)) {
        return Err(Refused("Invalid timer state"));
    }
    if !matches!(s.get("due").ok_or(Refused("missing key"))?, Value::Bool(_)) {
        return Err(Refused("Invalid timer state"));
    }
    timestamp(s.get("next_at"))?;
    timestamp(s.get("remaining"))?;
    let days = days_of(s)?;
    if days.len() > 36600 {
        return Err(Refused("Invalid daily ledger"));
    }
    for (day, count) in days {
        if !is_iso_day(day) {
            return Err(Refused("Invalid day"));
        }
        integer(Some(count), 0, MAX_TOTAL)?;
    }
    if sum_days(s) > MAX_TOTAL {
        return Err(Refused("Counter limit exceeded"));
    }
    let last = s.get("last").ok_or(Refused("missing key"))?;
    if *last != Value::Null {
        let Value::Object(_) = last else { return Err(Refused("Invalid last entry")) };
        let day = last.get("day").ok_or(Refused("missing key"))?;
        let entry = match day {
            Value::Str(day) => days.iter().find(|(k, _)| k == day),
            Value::Array(_) | Value::Object(_) => return Err(Refused("unhashable day")),
            _ => None,
        };
        let Some((_, Value::Int(on_day))) = entry else { return Err(Refused("Invalid last entry")) };
        let count = integer(last.get("count"), 1, 1000)?;
        if count > *on_day {
            return Err(Refused("Invalid undo entry"));
        }
    }
    Ok(())
}

pub fn initial(now: Num) -> Value {
    Value::object(vec![
        ("version", Value::Int(1)),
        ("interval", Value::Int(60)),
        ("suggested", Value::Int(20)),
        ("paused", Value::Bool(false)),
        ("due", Value::Bool(false)),
        ("next_at", now.py_add(Num::Int(3600)).to_value()),
        ("remaining", Value::Int(3600)),
        ("days", Value::Object(Vec::new())),
        ("last", Value::Null),
    ])
}

/// Python's `int(text)` for a command-line argument.
pub fn py_int(text: &str) -> Step<i128> {
    let bad = Refused("invalid literal for int()");
    // Unicode spaces become ' ' and Unicode decimal digits become ASCII, as CPython does first.
    let mut ascii = String::with_capacity(text.len());
    for c in text.chars() {
        let u = c as u32;
        if SPACES.contains(&u) {
            ascii.push(' ');
        } else if let Some(zero) = DIGIT_ZEROS.iter().find(|&&z| (z..z + 10).contains(&u)) {
            ascii.push(char::from(b'0' + (u - zero) as u8));
        } else if c.is_ascii() {
            ascii.push(c);
        } else {
            return Err(bad);
        }
    }
    let t = ascii.trim_matches(|c: char| c == ' ' || ('\t'..='\r').contains(&c));
    let (negative, body) = match t.as_bytes().first() {
        Some(b'-') => (true, &t[1..]),
        Some(b'+') => (false, &t[1..]),
        _ => (false, t),
    };
    let bytes = body.as_bytes();
    if bytes.is_empty() || !bytes[0].is_ascii_digit() || !bytes[bytes.len() - 1].is_ascii_digit() {
        return Err(bad);
    }
    let mut digits = String::new();
    for (n, &b) in bytes.iter().enumerate() {
        match b {
            b'0'..=b'9' => digits.push(b as char),
            b'_' if bytes[n + 1] != b'_' => {}
            _ => return Err(bad),
        }
    }
    let digits = digits.trim_start_matches('0');
    if digits.len() > 4300 {
        return Err(bad);
    }
    if digits.len() > 30 {
        return Ok(if negative { i128::MIN } else { i128::MAX }); // outside every range checked
    }
    let v: i128 = if digits.is_empty() { 0 } else { digits.parse().map_err(|_| Refused("invalid literal for int()"))? };
    Ok(if negative { -v } else { v })
}

fn num(s: &Value, key: &str) -> Num {
    s.get(key).and_then(Num::from_value).expect("validated")
}
fn int(s: &Value, key: &str) -> i128 {
    match s.get(key) {
        Some(Value::Int(i)) => *i,
        _ => unreachable!("validated"),
    }
}
fn flag(s: &Value, key: &str) -> bool {
    matches!(s.get(key), Some(Value::Bool(true)))
}

/// `operate(s, action, args, now, today)`. `s` has passed [`validate`].
pub fn operate(s: &mut Value, action: &str, args: &[String], now: Num, today: &str) -> Step<Report> {
    let mut alert = false;
    let interval = int(s, "interval");
    let paused = flag(s, "paused");
    let has_last = !matches!(s.get("last"), Some(Value::Null));
    match action {
        "tick" if !paused && now.ge(num(s, "next_at")) => {
            // A resume/restart sends at most one reminder, never a backlog.
            s.set(b"due", Value::Bool(true));
            s.set(b"next_at", now.py_add(Num::Int(interval * 60)).to_value());
            alert = true;
        }
        "add" => {
            let count = py_int(&args[0])?;
            if !(1..=1000).contains(&count) {
                return Err(Refused("Invalid integer"));
            }
            if sum_days(s) + count > MAX_TOTAL {
                return Err(Refused("Counter limit exceeded"));
            }
            let days = s.get_mut("days").expect("validated");
            let before = match days.get(today) {
                Some(Value::Int(i)) => *i,
                _ => 0,
            };
            days.set(today.as_bytes(), Value::Int(before + count));
            s.set(b"last", Value::object(vec![("day", Value::str(today)), ("count", Value::Int(count))]));
            s.set(b"due", Value::Bool(false));
            s.set(b"next_at", now.py_add(Num::Int(interval * 60)).to_value());
            s.set(b"remaining", Value::Int(interval * 60));
        }
        "undo" if has_last => {
            let last = s.get("last").cloned().expect("validated");
            let Some(Value::Str(day)) = last.get("day").cloned() else { unreachable!("validated") };
            let Some(Value::Int(count)) = last.get("count") else { unreachable!("validated") };
            let days = s.get_mut("days").expect("validated");
            if let Value::Object(items) = days {
                if let Some((_, Value::Int(v))) = items.iter_mut().find(|(k, _)| *k == day) {
                    *v -= count;
                }
            }
            s.set(b"last", Value::Null);
        }
        "pause" if !paused => {
            s.set(b"remaining", num(s, "next_at").py_sub(now).max0().to_value());
            s.set(b"paused", Value::Bool(true));
        }
        "resume" if paused => {
            s.set(b"next_at", now.py_add(num(s, "remaining")).to_value());
            s.set(b"paused", Value::Bool(false));
        }
        "snooze" => {
            s.set(b"next_at", now.py_add(Num::Int(300)).to_value());
            s.set(b"remaining", Value::Int(300));
            s.set(b"due", Value::Bool(false));
        }
        "settings" => {
            let new_interval = py_int(&args[0])?;
            if !(1..=1440).contains(&new_interval) {
                return Err(Refused("Invalid integer"));
            }
            let suggested = py_int(&args[1])?;
            if !(1..=1000).contains(&suggested) {
                return Err(Refused("Invalid integer"));
            }
            // Editing the suggested count alone must not delay the reminder.
            if new_interval != interval {
                s.set(b"next_at", now.py_add(Num::Int(new_interval * 60)).to_value());
                s.set(b"remaining", Value::Int(new_interval * 60));
                s.set(b"due", Value::Bool(false));
            }
            s.set(b"interval", Value::Int(new_interval));
            s.set(b"suggested", Value::Int(suggested));
        }
        _ => {}
    }
    validate(s)?;
    let paused = flag(s, "paused");
    let remaining = if paused { num(s, "remaining") } else { num(s, "next_at").py_sub(now).max0() };
    let Ok(days) = days_of(s) else { unreachable!("validated") };
    let mut history: Vec<(String, i128)> = days.iter().map(|(d, c)| (String::from_utf8(d.clone()).expect("ISO day"), if let Value::Int(i) = c { *i } else { 0 })).collect();
    history.sort_by(|a, b| b.cmp(a));
    history.truncate(7);
    Ok(Report {
        today: match s.get("days").and_then(|d| d.get(today)) {
            Some(Value::Int(i)) => *i,
            _ => 0,
        },
        total: sum_days(s),
        interval: int(s, "interval"),
        suggested: int(s, "suggested"),
        paused,
        due: flag(s, "due"),
        remaining: remaining.ceil(),
        alert,
        can_undo: !matches!(s.get("last"), Some(Value::Null)),
        history,
    })
}

/// One run of `store.py <action> <args>` against the ledger file's bytes.
///
/// Returns the report and, when the ledger must be written, its new bytes.
pub fn run(existing: Option<&[u8]>, action: &str, args: &[String], now: Num, today: &str) -> Step<(Report, Option<Vec<u8>>)> {
    if !ACTIONS.contains(&action) {
        return Err(Refused("unknown action"));
    }
    let expected = match action {
        "add" => 1,
        "settings" => 2,
        _ => 0,
    };
    if args.len() != expected {
        return Err(Refused("Wrong argument count"));
    }
    let mut s = match existing {
        Some(bytes) => {
            if bytes.len() as u64 > MAX_LEDGER_BYTES {
                return Err(Refused("Ledger too large"));
            }
            let s = loads(bytes).map_err(|_| Refused("not JSON"))?;
            validate(&s)?;
            s
        }
        None => initial(now),
    };
    let before = dumps(&s);
    let report = operate(&mut s, action, args, now, today)?;
    let after = dumps(&s);
    if existing.is_some() && after == before {
        return Ok((report, None));
    }
    if has_surrogate(&after) {
        return Err(Refused("lone surrogate cannot be written as UTF-8"));
    }
    let mut bytes = after;
    bytes.push(b'\n');
    Ok((report, Some(bytes)))
}

pub use pyjson::Num as Clock;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn python_int_forms() {
        for (text, v) in [("1_0", 10), (" 7 ", 7), ("+5", 5), ("007", 7), ("\t12\n", 12), ("+1_0", 10), ("\u{661}\u{662}", 12), ("-0", 0)] {
            assert_eq!(py_int(text), Ok(v), "{text:?}");
        }
        for text in ["", " ", "1.5", "0x10", "1e3", "ten", "--5", "5-", "1__0", "_5", "5_", "None", "<img src=x>"] {
            assert!(py_int(text).is_err(), "{text:?}");
        }
    }

    #[test]
    fn iso_days() {
        assert!(is_iso_day(b"2028-02-29") && is_iso_day(b"2000-02-29") && is_iso_day(b"0001-01-01"));
        assert!(!is_iso_day(b"1900-02-29") && !is_iso_day(b"0000-01-01") && !is_iso_day(b"20270115") && !is_iso_day(b"2027-04-31"));
    }
}
