#![cfg(unix)]
mod support;
use std::process::Command;
#[test]
fn real_pty_measures_picker_and_cancellation() {
    for cancel in [false, true] {
        let mut c = Command::new(support::binary("orbit-dev"));
        c.arg("bench-resume-picker")
            .arg(support::binary("orbit-fixture"))
            .args(["--runs", "1", "--timeout", "2"])
            .env("ORBIT_FIXTURE_MODE", "resume-picker");
        if cancel {
            c.arg("--cancel-loading");
        }
        let output = support::success(c.output().unwrap());
        let rows: Vec<serde_json::Value> = output
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert!(rows[0]["first_frame_ms"].is_number());
        assert!(rows[0]["populated_list_ms"].is_number());
        assert_eq!(rows[0]["terminal_restored"], true);
        assert_eq!(rows[1]["runs"], 1);
    }
}
