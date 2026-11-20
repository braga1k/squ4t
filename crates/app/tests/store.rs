//! tests/test_store.py of omarchy-squat-break, test for test, against the Windows ledger store:
//! real files, the real lock, the real atomic replacement.

use std::path::PathBuf;

use squ4t::store::{Failure, Store};
use squ4t_core::ledger::{self, Report};
use squ4t_core::pyjson::Num;

const NOW: i128 = 1_800_000_000;
const DAY: &str = "2026-11-12";

struct Ledger {
    store: Store,
    dir: PathBuf,
}

impl Ledger {
    fn new(name: &str) -> Ledger {
        let dir = std::env::temp_dir().join(format!("squ4t-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Ledger { store: Store::new(dir.clone()), dir }
    }
    fn run(&self, action: &str, args: &[&str]) -> Result<Report, Failure> {
        self.at(action, args, NOW, DAY)
    }
    fn at(&self, action: &str, args: &[&str], now: i128, day: &str) -> Result<Report, Failure> {
        let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        self.store.run_at(action, &args, Num::Int(now), day)
    }
    fn ok(&self, action: &str, args: &[&str]) -> Report {
        self.run(action, args).unwrap()
    }
    fn tick(&self, now: i128) -> Report {
        self.at("tick", &[], now, DAY).unwrap()
    }
    fn bytes(&self) -> Vec<u8> {
        std::fs::read(self.dir.join("ledger.json")).unwrap()
    }
}

impl Drop for Ledger {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn initial_counts_and_persistence() {
    let l = Ledger::new("persist");
    assert_eq!(l.ok("status", &[]).total, 0);
    assert_eq!(l.ok("add", &["20"]).total, 20);
    let r = l.ok("add", &["13"]);
    assert_eq!((r.today, r.total), (33, 33));
    assert_eq!(l.ok("status", &[]).total, 33);
    // store.py checks mode 0600. Windows: the file lives in the user's profile, whose ACL keeps
    // other users out; the test checks the folder is the one given and nothing else is left there.
    let mut names: Vec<String> = std::fs::read_dir(&l.dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    assert_eq!(names, ["ledger.json", "ledger.lock"]);
}

#[test]
fn midnight_and_undo_previous_day() {
    let l = Ledger::new("midnight");
    l.ok("add", &["20"]);
    let r = l.at("status", &[], NOW, "2026-11-13").unwrap();
    assert_eq!((r.today, r.total), (0, 20));
    let r = l.at("undo", &[], NOW, "2026-11-13").unwrap();
    assert_eq!((r.today, r.total, r.can_undo), (0, 0, false));
    l.ok("undo", &[]);
    assert_eq!(l.ok("status", &[]).total, 0);
}

#[test]
fn recurring_alerts_no_counts_or_catchup_spam() {
    let l = Ledger::new("alerts");
    l.ok("status", &[]);
    assert!(!l.tick(NOW + 3599).alert);
    assert!(l.tick(NOW + 3600).alert);
    assert!(!l.tick(NOW + 3600).alert);
    let r = l.tick(NOW + 36000);
    assert!(r.alert);
    assert_eq!(r.total, 0);
    assert!(!l.tick(NOW + 36001).alert);
    assert!(l.tick(NOW + 39600).alert);
}

#[test]
fn pause_resume_persists_remaining() {
    let l = Ledger::new("pause");
    l.ok("status", &[]);
    l.at("pause", &[], NOW + 1000, DAY).unwrap();
    assert!(!l.tick(NOW + 10000).alert);
    assert_eq!(l.at("resume", &[], NOW + 10000, DAY).unwrap().remaining, 2600);
    assert!(!l.tick(NOW + 12599).alert);
    assert!(l.tick(NOW + 12600).alert);
}

#[test]
fn snooze_and_completed_set_restart_timer() {
    let l = Ledger::new("snooze");
    l.ok("status", &[]);
    l.ok("snooze", &[]);
    assert!(!l.tick(NOW + 299).alert);
    assert!(l.tick(NOW + 300).alert);
    let r = l.at("add", &["20"], NOW + 301, DAY).unwrap();
    assert!(!r.due);
    assert_eq!(r.remaining, 3600);
}

#[test]
fn settings_validate_and_do_not_count_reps() {
    let l = Ledger::new("settings");
    let r = l.ok("settings", &["45", "15"]);
    assert_eq!((r.interval, r.suggested, r.total), (45, 15, 0));
    assert_eq!(l.at("settings", &["45", "10"], NOW + 100, DAY).unwrap().remaining, 2600);
    let before = l.bytes();
    for (action, args) in [("settings", &["0", "20"][..]), ("add", &["-1"]), ("add", &["1001"]), ("add", &["<img src=x>"])] {
        assert!(matches!(l.run(action, args), Err(Failure::Refused(_))), "{action} {args:?}");
        assert_eq!(l.bytes(), before);
    }
}

#[test]
fn corrupt_ledger_is_never_overwritten() {
    let l = Ledger::new("corrupt");
    let path = l.dir.join("ledger.json");
    let initial = String::from_utf8(ledger::run(None, "status", &[], Num::Int(NOW), DAY).unwrap().1.unwrap()).unwrap();
    let bad_day = initial.replace("\"days\": {}", "\"days\": {\"<img src=x>\": 20}");
    for content in ["{broken", "{\"version\": 42}", bad_day.as_str()] {
        std::fs::write(&path, content).unwrap();
        assert!(l.run("add", &["20"]).is_err(), "{content}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), content);
    }
}

#[test]
fn parallel_writers_cannot_double_alert_or_lose_counts() {
    let l = Ledger::new("parallel");
    l.ok("status", &[]);
    let store = l.store.clone();
    let alerts: usize = std::thread::scope(|s| {
        let handles: Vec<_> = (0..16).map(|_| s.spawn(|| store.run_at("tick", &[], Num::Int(NOW + 3600), DAY).unwrap().alert as usize)).collect();
        handles.into_iter().map(|h| h.join().unwrap()).sum()
    });
    assert_eq!(alerts, 1);
    std::thread::scope(|s| {
        for _ in 0..32 {
            s.spawn(|| store.run_at("add", &["10".to_string()], Num::Int(NOW), DAY).unwrap());
        }
    });
    assert_eq!(l.ok("status", &[]).total, 320);
}
