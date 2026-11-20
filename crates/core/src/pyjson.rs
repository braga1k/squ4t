//! JSON as Python's `json` module reads and writes it.
//!
//! The ledger must stay byte-compatible with the one store.py writes, including fields
//! this program does not know, so values keep Python's types: an integer stays an
//! integer of any size, `1.0` stays a float, `NaN` is accepted, and strings may hold
//! the lone surrogates Python allows. Strings are kept as WTF-8, whose byte order is
//! Python's code-point order, which is how `sort_keys` orders keys.

use std::cmp::Ordering;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    /// An integer that fits in i128.
    Int(i128),
    /// A larger integer, as its canonical decimal text. No field this program reads accepts one.
    BigInt(String),
    Float(f64),
    Str(Vec<u8>),
    Array(Vec<Value>),
    /// Keys in first-seen order; a repeated key keeps its first place and its last value, as a dict does.
    Object(Vec<(Vec<u8>, Value)>),
}

/// Why text could not be read. Python reports these with different exception types;
/// store.py turns all of them into the same refusal.
#[derive(Debug, PartialEq, Eq)]
pub struct DecodeError;

const MAX_DEPTH: usize = 900; // Python raises RecursionError at about 1000 levels.
const MAX_INT_DIGITS: usize = 4300; // sys.int_info.default_max_str_digits

impl Value {
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Object(items) => items.iter().find(|(k, _)| k == key.as_bytes()).map(|(_, v)| v),
            _ => None,
        }
    }
    pub fn get_mut(&mut self, key: &str) -> Option<&mut Value> {
        match self {
            Value::Object(items) => items.iter_mut().find(|(k, _)| k == key.as_bytes()).map(|(_, v)| v),
            _ => None,
        }
    }
    /// `d[key] = value`: replaces in place, or appends.
    pub fn set(&mut self, key: &[u8], value: Value) {
        if let Value::Object(items) = self {
            match items.iter_mut().find(|(k, _)| k == key) {
                Some(slot) => slot.1 = value,
                None => items.push((key.to_vec(), value)),
            }
        }
    }
    pub fn str(text: &str) -> Value {
        Value::Str(text.as_bytes().to_vec())
    }
    pub fn object(items: Vec<(&str, Value)>) -> Value {
        let mut object = Value::Object(Vec::new());
        for (k, v) in items {
            object.set(k.as_bytes(), v);
        }
        object
    }
}

// ---------------------------------------------------------------- reading

/// `json.loads(text)`, on bytes that Python first decoded as strict UTF-8.
pub fn loads(bytes: &[u8]) -> Result<Value, DecodeError> {
    let text = std::str::from_utf8(bytes).map_err(|_| DecodeError)?;
    let chars: Vec<char> = text.chars().collect();
    let mut parser = Parser { s: &chars, i: 0 };
    parser.skip_ws();
    let value = parser.value(0)?;
    parser.skip_ws();
    if parser.i != chars.len() {
        return Err(DecodeError); // "Extra data", including a leading BOM's leftovers
    }
    Ok(value)
}

struct Parser<'a> {
    s: &'a [char],
    i: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<char> {
        self.s.get(self.i).copied()
    }
    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(' ' | '\t' | '\n' | '\r')) {
            self.i += 1;
        }
    }
    fn literal(&mut self, word: &str) -> bool {
        let w: Vec<char> = word.chars().collect();
        if self.s[self.i..].starts_with(&w) {
            self.i += w.len();
            true
        } else {
            false
        }
    }
    fn value(&mut self, depth: usize) -> Result<Value, DecodeError> {
        if depth > MAX_DEPTH {
            return Err(DecodeError);
        }
        match self.peek().ok_or(DecodeError)? {
            '"' => {
                self.i += 1;
                Ok(Value::Str(self.string()?))
            }
            '{' => {
                self.i += 1;
                self.object(depth)
            }
            '[' => {
                self.i += 1;
                self.array(depth)
            }
            'n' if self.literal("null") => Ok(Value::Null),
            't' if self.literal("true") => Ok(Value::Bool(true)),
            'f' if self.literal("false") => Ok(Value::Bool(false)),
            'N' if self.literal("NaN") => Ok(Value::Float(f64::NAN)),
            'I' if self.literal("Infinity") => Ok(Value::Float(f64::INFINITY)),
            '-' if self.s.get(self.i + 1) == Some(&'I') => {
                if self.literal("-Infinity") {
                    Ok(Value::Float(f64::NEG_INFINITY))
                } else {
                    Err(DecodeError)
                }
            }
            _ => self.number(),
        }
    }
    fn number(&mut self) -> Result<Value, DecodeError> {
        let start = self.i;
        let digit = |c: Option<char>| matches!(c, Some('0'..='9'));
        if self.peek() == Some('-') {
            self.i += 1;
        }
        match self.peek() {
            Some('0') => self.i += 1,
            Some('1'..='9') => {
                while digit(self.peek()) {
                    self.i += 1;
                }
            }
            _ => return Err(DecodeError),
        }
        let mut float = false;
        if self.peek() == Some('.') && digit(self.s.get(self.i + 1).copied()) {
            float = true;
            self.i += 1;
            while digit(self.peek()) {
                self.i += 1;
            }
        }
        if matches!(self.peek(), Some('e' | 'E')) {
            let save = self.i;
            self.i += 1;
            if matches!(self.peek(), Some('+' | '-')) {
                self.i += 1;
            }
            if digit(self.peek()) {
                float = true;
                while digit(self.peek()) {
                    self.i += 1;
                }
            } else {
                self.i = save; // "1e" is the number 1 followed by text that is not JSON
            }
        }
        let text: String = self.s[start..self.i].iter().collect();
        if float {
            text.parse::<f64>().map(Value::Float).map_err(|_| DecodeError)
        } else {
            int_value(&text)
        }
    }
    fn hex4(&self, at: usize) -> Result<u32, DecodeError> {
        let mut v = 0u32;
        for k in 0..4 {
            let c = self.s.get(at + k).ok_or(DecodeError)?;
            v = v * 16 + c.to_digit(16).ok_or(DecodeError)?;
        }
        Ok(v)
    }
    /// After the opening quote. Mirrors `scanstring_unicode` with strict=True.
    fn string(&mut self) -> Result<Vec<u8>, DecodeError> {
        let mut out = Vec::new();
        loop {
            let c = self.peek().ok_or(DecodeError)?;
            self.i += 1;
            match c {
                '"' => return Ok(out),
                '\\' => {
                    let e = self.peek().ok_or(DecodeError)?;
                    self.i += 1;
                    let code = match e {
                        '"' => '"' as u32,
                        '\\' => '\\' as u32,
                        '/' => '/' as u32,
                        'b' => 8,
                        'f' => 12,
                        'n' => 10,
                        'r' => 13,
                        't' => 9,
                        'u' => {
                            let mut c = self.hex4(self.i)?;
                            self.i += 4;
                            // `end + 6 < len`: room for "\uXXXX" and at least one more character.
                            if (0xD800..=0xDBFF).contains(&c) && self.i + 6 < self.s.len() && self.s[self.i] == '\\' && self.s[self.i + 1] == 'u' {
                                let c2 = self.hex4(self.i + 2)?;
                                if (0xDC00..=0xDFFF).contains(&c2) {
                                    c = 0x10000 + ((c - 0xD800) << 10) + (c2 - 0xDC00);
                                    self.i += 6;
                                }
                            }
                            c
                        }
                        _ => return Err(DecodeError),
                    };
                    push_wtf8(&mut out, code);
                }
                c if (c as u32) < 0x20 => return Err(DecodeError),
                c => push_wtf8(&mut out, c as u32),
            }
        }
    }
    fn object(&mut self, depth: usize) -> Result<Value, DecodeError> {
        let mut object = Value::Object(Vec::new());
        self.skip_ws();
        if self.peek() == Some('}') {
            self.i += 1;
            return Ok(object);
        }
        loop {
            if self.peek() != Some('"') {
                return Err(DecodeError);
            }
            self.i += 1;
            let key = self.string()?;
            self.skip_ws();
            if self.peek() != Some(':') {
                return Err(DecodeError);
            }
            self.i += 1;
            self.skip_ws();
            let value = self.value(depth + 1)?;
            object.set(&key, value);
            self.skip_ws();
            match self.peek() {
                Some('}') => {
                    self.i += 1;
                    return Ok(object);
                }
                Some(',') => {
                    self.i += 1;
                    self.skip_ws();
                }
                _ => return Err(DecodeError),
            }
        }
    }
    fn array(&mut self, depth: usize) -> Result<Value, DecodeError> {
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(']') {
            self.i += 1;
            return Ok(Value::Array(items));
        }
        loop {
            items.push(self.value(depth + 1)?);
            self.skip_ws();
            match self.peek() {
                Some(']') => {
                    self.i += 1;
                    return Ok(Value::Array(items));
                }
                Some(',') => {
                    self.i += 1;
                    self.skip_ws();
                }
                _ => return Err(DecodeError),
            }
        }
    }
}

/// Python's int() of already-validated ASCII: optional '-', decimal digits.
fn int_value(text: &str) -> Result<Value, DecodeError> {
    let (negative, digits) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let digits = digits.trim_start_matches('0');
    if digits.len() > MAX_INT_DIGITS {
        return Err(DecodeError);
    }
    if digits.is_empty() {
        return Ok(Value::Int(0));
    }
    let canonical = if negative { format!("-{digits}") } else { digits.to_string() };
    Ok(match canonical.parse::<i128>() {
        Ok(v) => Value::Int(v),
        Err(_) => Value::BigInt(canonical),
    })
}

pub fn push_wtf8(out: &mut Vec<u8>, c: u32) {
    match c {
        0..=0x7F => out.push(c as u8),
        0x80..=0x7FF => out.extend([0xC0 | (c >> 6) as u8, 0x80 | (c & 0x3F) as u8]),
        0x800..=0xFFFF => out.extend([0xE0 | (c >> 12) as u8, 0x80 | ((c >> 6) & 0x3F) as u8, 0x80 | (c & 0x3F) as u8]),
        _ => out.extend([0xF0 | (c >> 18) as u8, 0x80 | ((c >> 12) & 0x3F) as u8, 0x80 | ((c >> 6) & 0x3F) as u8, 0x80 | (c & 0x3F) as u8]),
    }
}

/// True when the WTF-8 text holds a lone surrogate, which UTF-8 cannot encode.
pub fn has_surrogate(bytes: &[u8]) -> bool {
    bytes.windows(2).any(|w| w[0] == 0xED && w[1] >= 0xA0)
}

// ---------------------------------------------------------------- writing

/// `json.dumps(value, ensure_ascii=False, sort_keys=True)` as WTF-8.
/// Encoding it as UTF-8 fails, as in Python, when it holds a lone surrogate.
pub fn dumps(value: &Value) -> Vec<u8> {
    let mut out = Vec::new();
    write_value(&mut out, value);
    out
}

fn write_value(out: &mut Vec<u8>, value: &Value) {
    match value {
        Value::Null => out.extend_from_slice(b"null"),
        Value::Bool(true) => out.extend_from_slice(b"true"),
        Value::Bool(false) => out.extend_from_slice(b"false"),
        Value::Int(v) => out.extend_from_slice(v.to_string().as_bytes()),
        Value::BigInt(text) => out.extend_from_slice(text.as_bytes()),
        Value::Float(f) => out.extend_from_slice(float_repr(*f).as_bytes()),
        Value::Str(s) => write_str(out, s),
        Value::Array(items) => {
            out.push(b'[');
            for (n, item) in items.iter().enumerate() {
                if n > 0 {
                    out.extend_from_slice(b", ");
                }
                write_value(out, item);
            }
            out.push(b']');
        }
        Value::Object(items) => {
            let mut sorted: Vec<&(Vec<u8>, Value)> = items.iter().collect();
            sorted.sort_by(|a, b| a.0.cmp(&b.0));
            out.push(b'{');
            for (n, (k, v)) in sorted.into_iter().enumerate() {
                if n > 0 {
                    out.extend_from_slice(b", ");
                }
                write_str(out, k);
                out.extend_from_slice(b": ");
                write_value(out, v);
            }
            out.push(b'}');
        }
    }
}

fn write_str(out: &mut Vec<u8>, s: &[u8]) {
    out.push(b'"');
    for &b in s {
        match b {
            b'"' => out.extend_from_slice(b"\\\""),
            b'\\' => out.extend_from_slice(b"\\\\"),
            b'\n' => out.extend_from_slice(b"\\n"),
            b'\r' => out.extend_from_slice(b"\\r"),
            b'\t' => out.extend_from_slice(b"\\t"),
            8 => out.extend_from_slice(b"\\b"),
            12 => out.extend_from_slice(b"\\f"),
            0..=0x1F => out.extend_from_slice(format!("\\u{b:04x}").as_bytes()),
            _ => out.push(b),
        }
    }
    out.push(b'"');
}

/// `repr(float)` as `json` writes it: the shortest text that reads back as the same value.
pub fn float_repr(f: f64) -> String {
    if f.is_nan() {
        return "NaN".into();
    }
    if f.is_infinite() {
        return if f > 0.0 { "Infinity".into() } else { "-Infinity".into() };
    }
    if f == 0.0 {
        return if f.is_sign_negative() { "-0.0".into() } else { "0.0".into() };
    }
    let sci = format!("{:e}", f.abs()); // shortest round-trip digits, e.g. "1.5e-10"
    let (mantissa, exponent) = sci.split_once('e').expect("LowerExp has an exponent");
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let decpt = exponent.parse::<i32>().expect("integer exponent") + 1;
    let n = digits.len() as i32;
    let sign = if f < 0.0 { "-" } else { "" };
    if -4 < decpt && decpt <= 16 {
        let body = if decpt <= 0 {
            format!("0.{}{}", "0".repeat((-decpt) as usize), digits)
        } else if decpt >= n {
            format!("{}{}.0", digits, "0".repeat((decpt - n) as usize))
        } else {
            format!("{}.{}", &digits[..decpt as usize], &digits[decpt as usize..])
        };
        format!("{sign}{body}")
    } else {
        let mut body = digits[..1].to_string();
        if n > 1 {
            body.push('.');
            body.push_str(&digits[1..]);
        }
        let e = decpt - 1;
        format!("{sign}{body}e{}{:02}", if e < 0 { '-' } else { '+' }, e.abs())
    }
}

// ---------------------------------------------------------------- numbers

/// A Python int or float, with Python's mixed arithmetic and exact comparisons.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Num {
    Int(i128),
    Float(f64),
}

impl Num {
    pub fn from_value(v: &Value) -> Option<Num> {
        match v {
            Value::Int(i) => Some(Num::Int(*i)),
            Value::Float(f) => Some(Num::Float(*f)),
            _ => None,
        }
    }
    pub fn to_value(self) -> Value {
        match self {
            Num::Int(i) => Value::Int(i),
            Num::Float(f) => Value::Float(f),
        }
    }
    pub fn py_add(self, other: Num) -> Num {
        match (self, other) {
            (Num::Int(a), Num::Int(b)) => Num::Int(a + b),
            (a, b) => Num::Float(a.as_f64() + b.as_f64()),
        }
    }
    pub fn py_sub(self, other: Num) -> Num {
        match (self, other) {
            (Num::Int(a), Num::Int(b)) => Num::Int(a - b),
            (a, b) => Num::Float(a.as_f64() - b.as_f64()),
        }
    }
    /// int to float as Python converts it: rounded to nearest, ties to even.
    pub fn as_f64(self) -> f64 {
        match self {
            Num::Int(i) => i as f64,
            Num::Float(f) => f,
        }
    }
    /// Python's exact comparison between ints and floats.
    pub fn py_cmp(self, other: Num) -> Option<Ordering> {
        match (self, other) {
            (Num::Int(a), Num::Int(b)) => Some(a.cmp(&b)),
            (Num::Float(a), Num::Float(b)) => a.partial_cmp(&b),
            (Num::Int(a), Num::Float(b)) => int_float_cmp(a, b),
            (Num::Float(a), Num::Int(b)) => int_float_cmp(b, a).map(Ordering::reverse),
        }
    }
    pub fn ge(self, other: Num) -> bool {
        matches!(self.py_cmp(other), Some(Ordering::Greater | Ordering::Equal))
    }
    pub fn gt(self, other: Num) -> bool {
        matches!(self.py_cmp(other), Some(Ordering::Greater))
    }
    /// `max(0, x)`: the int 0 unless x is larger.
    pub fn max0(self) -> Num {
        if self.gt(Num::Int(0)) {
            self
        } else {
            Num::Int(0)
        }
    }
    /// `math.ceil(x)`, an int.
    pub fn ceil(self) -> i128 {
        match self {
            Num::Int(i) => i,
            Num::Float(f) => f.ceil() as i128,
        }
    }
}

fn int_float_cmp(i: i128, f: f64) -> Option<Ordering> {
    if f.is_nan() {
        return None;
    }
    if f.is_infinite() {
        return Some(if f > 0.0 { Ordering::Less } else { Ordering::Greater });
    }
    const EXACT: i128 = 1 << 53;
    if (-EXACT..=EXACT).contains(&i) {
        return (i as f64).partial_cmp(&f);
    }
    if f.abs() >= 1.7e38 {
        return Some(if f > 0.0 { Ordering::Less } else { Ordering::Greater });
    }
    // Both are integers at this magnitude, or the float has a fraction below an integer part that fits.
    let floor = f.floor();
    let whole = floor as i128;
    match i.cmp(&whole) {
        Ordering::Equal if f > floor => Some(Ordering::Less),
        other => Some(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn float_repr_matches_python() {
        for (f, text) in [
            (1e16, "1e+16"),
            (1e15, "1000000000000000.0"),
            (1.5e-10, "1.5e-10"),
            (1e-5, "1e-05"),
            (0.0001, "0.0001"),
            (1800007200.5, "1800007200.5"),
            (3600.0, "3600.0"),
            (-0.0, "-0.0"),
            (5e-324, "5e-324"),
            (1e22, "1e+22"),
            (123456789.12345679, "123456789.12345679"),
            (1.2345678901234568e18, "1.2345678901234568e+18"),
        ] {
            assert_eq!(float_repr(f), text, "{f:e}");
        }
    }

    #[test]
    fn round_trip_keeps_python_types() {
        let text =
            concat!(r#"{"b": [1, 2.0, null, true], "a": 1e16, "big": 123456789012345678901234567890123456789012, "k": ""#, "\\ud83c", "\\udfcb", "\\u00e9", r#""}"#).as_bytes();
        let v = loads(text).unwrap();
        assert_eq!(
            String::from_utf8(dumps(&v)).unwrap(),
            "{\"a\": 1e+16, \"b\": [1, 2.0, null, true], \"big\": 123456789012345678901234567890123456789012, \"k\": \"\u{1F3CB}\u{e9}\"}"
        );
    }

    #[test]
    fn python_rejections() {
        for text in ["", "{broken", "[1,]", "{\"a\":1,}", "060", "1.", ".5", "+1", "\u{feff}{}", "{\"a\":\"\t\"}", "{'a':1}", "\u{a0}{}"] {
            assert!(loads(text.as_bytes()).is_err(), "{text:?}");
        }
        assert_eq!(loads(b"-0").unwrap(), Value::Int(0));
        assert!(matches!(loads(b"1e999").unwrap(), Value::Float(f) if f.is_infinite()));
        assert!(has_surrogate(&dumps(&loads(br#""\ud800""#).unwrap())));
    }
}
