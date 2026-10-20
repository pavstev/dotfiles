//! Port of the CLI class of claude/hooks/test_pace.py: the `pace` binary (`CARGO_BIN_EXE_pace`).
//! The other 60 tests of that file need an injected clock and live in `src/pace_core.rs` (`mod tests`).
mod common;

use common::{iso_utc, run_bin, temp_home};
use serde_json::{Value, json};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

const PACE: &str = env!("CARGO_BIN_EXE_pace");
/// Default record on sonnet (3 / 15 per M): 3 x (1000 + 2 x 2000 + 0.1 x 10000) + 15 x 500, in millionths.
const WEIGHT: f64 = 0.0255;

struct Case {
    home: PathBuf,
}

impl Case {
    fn new() -> Case {
        let home = temp_home("pace");
        fs::create_dir_all(home.join("cache")).unwrap();
        fs::create_dir_all(home.join(".claude/projects/proj")).unwrap();
        Case { home }
    }

    fn run_cli(&self, args: &[&str]) -> (i32, String, String) {
        let home = self.home.to_string_lossy().into_owned();
        let cache = self.home.join("cache").to_string_lossy().into_owned();
        run_bin(PACE, args, "", &[("HOME", &home), ("XDG_CACHE_HOME", &cache), ("TZ", "UTC")], None)
    }

    fn write_record(&self, name: &str, epoch: i64, mid: &str) {
        let rec = json!({"type": "assistant", "timestamp": iso_utc(epoch), "message": {
            "id": mid, "model": "claude-sonnet-5-5", "role": "assistant",
            "usage": {"input_tokens": 1000, "output_tokens": 500, "cache_creation_input_tokens": 2000,
                "cache_read_input_tokens": 10000}}});
        fs::write(self.home.join(".claude/projects/proj").join(name), format!("{rec}\n")).unwrap();
    }

    fn calib(&self) -> Value {
        serde_json::from_str(&fs::read_to_string(self.home.join("cache/claude-pace-calib.json")).unwrap()).unwrap()
    }
}

impl Drop for Case {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.home);
    }
}

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64
}

#[test]
fn test_status_prints_json() {
    let c = Case::new();
    let (rc, out, err) = c.run_cli(&["status"]);
    assert_eq!(rc, 0, "{err}");
    let d: Value = serde_json::from_str(&out).unwrap();
    let mut keys: Vec<&str> = d.as_object().unwrap().keys().map(String::as_str).collect();
    keys.sort();
    assert_eq!(keys, vec!["budget", "resets_at", "source", "state", "used"]);
}

#[test]
fn test_status_cli_estimates_from_transcripts() {
    let c = Case::new();
    c.write_record("a.jsonl", now() - 60, "m1");
    let (_, out, _) = c.run_cli(&["status"]);
    let d: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(d["source"], "estimate");
    assert!(!d["used"].is_null(), "{d}");
}

#[test]
fn test_refresh_and_seed() {
    let c = Case::new();
    c.write_record("a.jsonl", now() - 60, "m1");
    assert_eq!(c.run_cli(&["refresh"]).0, 0);
    let (rc, _, err) = c.run_cli(&["seed", "12.5"]);
    assert_eq!(rc, 0, "{err}");
    let got = c.calib()["week_weight"].as_f64().unwrap();
    assert!((got - WEIGHT / 0.125).abs() < 5e-8, "{got}");
    assert_eq!(c.run_cli(&["seed", "abc"]).0, 2);
}

#[test]
fn test_bad_usage_exits_2() {
    let c = Case::new();
    assert_eq!(c.run_cli(&[]).0, 2);
    assert_eq!(c.run_cli(&["nope"]).0, 2);
}
