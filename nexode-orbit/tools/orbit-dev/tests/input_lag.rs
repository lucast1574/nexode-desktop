#![cfg(unix)]
use std::process::Command;
#[test]
fn input_lag_reports_setup_failures_separately_from_budget() {
    for args in [
        vec!["--repeat", "0"],
        vec!["--binary", "/bin/false"],
        vec!["--background-clients", "101"],
        vec!["--socket", "/tmp/missing"],
    ] {
        let out = Command::new(env!("CARGO_BIN_EXE_orbit-dev"))
            .arg("repro-input-lag")
            .args(args)
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(3));
    }
}
