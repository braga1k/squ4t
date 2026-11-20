//! Compares the Rust elastic motion with what 1nstall's own motion.cs computes
//! (oracle/golden/motion.json, recorded by oracle/motion/record.ps1).

use serde_json::Value as J;
use squ4t_core::motion::{flight, spring, Feel, Pose};

/// Same operations in the same order; only the C runtime's exp/sin/cos may differ in the last bit.
const TOLERANCE: f64 = 1e-9;

fn n(v: &J) -> f64 {
    v.as_f64().unwrap()
}

#[test]
fn rust_motion_matches_motion_cs() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../oracle/golden/motion.json");
    let golden: J = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let (mut compared, mut worst, mut identical) = (0usize, 0f64, 0usize);
    let mut check = |rust: f64, cs: f64, what: &str| {
        let d = (rust - cs).abs();
        compared += 1;
        if rust.to_bits() == cs.to_bits() {
            identical += 1;
        }
        worst = worst.max(d);
        assert!(d <= TOLERANCE, "{what}: rust {rust} motion.cs {cs}");
    };
    for s in golden["spring"].as_array().unwrap() {
        check(spring(n(&s[1]), n(&s[0])), n(&s[2]), "spring");
    }
    let flights = golden["flights"].as_array().unwrap();
    for f in flights {
        let feel = match f["feel"].as_str().unwrap() {
            "balanced" => Feel::BALANCED,
            "firm" => Feel::FIRM,
            _ => Feel::ELASTIC,
        };
        let from = Pose::new(n(&f["from"][0]), n(&f["from"][1]), n(&f["from"][2]));
        let poses = flight(from, n(&f["target"]), feel);
        for (i, cs) in f["poses"].as_array().unwrap().iter().enumerate() {
            let what = format!("{} {from:?} -> {} keyframe {i}", f["feel"], f["target"]);
            check(poses[i].x, n(&cs[0]), &what);
            check(poses[i].stretch, n(&cs[1]), &what);
            check(poses[i].height, n(&cs[2]), &what);
        }
    }
    println!("motion: {} flights, {compared} values; {identical} bit-identical; largest difference {worst:e}", flights.len());
    assert!(flights.len() >= 48);
}
