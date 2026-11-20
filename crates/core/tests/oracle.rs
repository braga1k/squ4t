//! Replays every step store.py was recorded taking (oracle/golden/ledger.jsonl.gz) through the
//! Rust ledger and requires the same answer: refused or not, the same report, and the same
//! ledger bytes on disk afterwards.
//!
//! The only difference allowed is the kind of refusal: store.py's main() prints one message
//! for every exception it catches, so the Python exception type is not compared.

use std::io::{BufRead, BufReader};

use serde_json::Value as J;
use squ4t_core::ledger::{run, Report};
use squ4t_core::pyjson::Num;

const BASE_LEDGER: &str =
    r#"{"days": {"2027-01-15": 40}, "due": false, "interval": 60, "last": null, "next_at": 1800003600, "paused": false, "remaining": 3600, "suggested": 20, "version": 1}"#;

fn golden() -> Box<dyn BufRead> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../oracle/golden");
    let gz = dir.join("ledger.jsonl.gz");
    let file = std::fs::File::open(&gz).unwrap_or_else(|e| panic!("{}: {e}", gz.display()));
    Box::new(BufReader::new(flate2::read::GzDecoder::new(file)))
}

fn report_json(r: &Report) -> J {
    serde_json::from_str(&r.to_json()).expect("report is JSON")
}

fn now_of(v: &J) -> Num {
    match v.as_i64() {
        Some(i) if !v.is_f64() => Num::Int(i as i128),
        _ => Num::Float(v.as_f64().expect("number")),
    }
}

#[test]
fn rust_ledger_matches_store_py() {
    let (mut cases, mut steps, mut refused, mut written) = (0, 0, 0, 0);
    let mut failures = Vec::new();
    for line in golden().lines() {
        let case: J = serde_json::from_str(&line.unwrap()).unwrap();
        cases += 1;
        let id = case["id"].as_i64().unwrap();
        let mut file: Option<Vec<u8>> = match &case["initial"] {
            J::Null => None,
            J::String(s) => Some(s.as_bytes().to_vec()),
            J::Object(o) => {
                let size = o["padded_to"].as_u64().unwrap() as usize;
                let mut text = format!("{:<width$}", BASE_LEDGER, width = size - 1);
                text.push('\n');
                Some(text.into_bytes())
            }
            other => panic!("initial {other}"),
        };
        for (n, step) in case["steps"].as_array().unwrap().iter().enumerate() {
            steps += 1;
            let action = step["action"].as_str().unwrap();
            let args: Vec<String> = step["args"].as_array().unwrap().iter().map(|a| a.as_str().unwrap().to_string()).collect();
            let now = now_of(&step["now"]);
            let today = step["today"].as_str().unwrap();
            let got = run(file.as_deref(), action, &args, now, today);
            let expected_error = step.get("error").and_then(J::as_str);
            let mut problem = None;
            match (&got, expected_error) {
                (Err(_), Some(_)) => refused += 1,
                (Ok((report, write)), None) => {
                    if report_json(report) != step["result"] {
                        problem = Some(format!("report {} != {}", report.to_json(), step["result"]));
                    }
                    if let Some(bytes) = write {
                        written += 1;
                        file = Some(bytes.clone());
                    }
                }
                (Err(e), None) => problem = Some(format!("refused ({}) where store.py answered {}", e.0, step["result"])),
                (Ok((report, _)), Some(err)) => problem = Some(format!("answered {} where store.py raised {err}", report.to_json())),
            }
            // The file on disk after the step: unchanged unless the recording says otherwise.
            if problem.is_none() {
                let expected_file = step.get("ledger");
                let actual = file.as_deref().map(|b| String::from_utf8_lossy(b).into_owned());
                match expected_file {
                    Some(J::String(text)) if actual.as_deref() != Some(text.as_str()) => problem = Some(format!("ledger {:?} != {:?}", actual, text)),
                    Some(J::Null) if actual.is_some() => problem = Some("ledger should be gone".into()),
                    _ => {}
                }
            }
            if let Some(p) = problem {
                failures.push(format!("case {id} step {n} {action} {args:?} now={now:?}: {p}"));
                break; // later steps of this case start from a different file
            }
        }
    }
    println!("oracle ledger: {cases} cases, {steps} steps; {refused} refused by both; {written} writes; unexplained {}", failures.len());
    for f in failures.iter().take(25) {
        println!("  {f}");
    }
    assert!(cases > 2000, "the recording is incomplete");
    assert!(failures.is_empty(), "{} steps differ from store.py", failures.len());
}
