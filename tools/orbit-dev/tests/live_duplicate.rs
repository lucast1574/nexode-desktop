#![cfg(unix)]
use std::process::Command;
#[test]
fn duplicate_probe_cannot_report_success_without_valid_setup() {
    for args in [vec!["--turns", "0"], vec!["--binary", "/bin/false"]] {
        let out = Command::new(env!("CARGO_BIN_EXE_orbit-dev"))
            .arg("repro-live-duplicate")
            .args(args)
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(3));
    }
}
