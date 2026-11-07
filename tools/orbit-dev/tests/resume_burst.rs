#![cfg(target_os = "linux")]
use std::process::Command;
#[test]
fn rejects_invalid_ranges_and_early_server_exit() {
    for args in [
        vec!["--burst", "0"],
        vec!["--timeout", "NaN"],
        vec!["--stagger-ms", "-1"],
        vec!["--binary", "/bin/false"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_orbit-dev"))
            .arg("profile-remote-resume-burst")
            .args(args)
            .output()
            .unwrap();
        assert!(!output.status.success());
    }
}

#[test]
fn spawn_profile_rejects_invalid_and_failed_binary() {
    for args in [
        vec!["--runs", "0"],
        vec!["--sample-interval-ms", "NaN"],
        vec!["--skip-client", "--skip-server"],
        vec!["/bin/false"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_orbit-dev"))
            .arg("profile-spawn")
            .args(args)
            .output()
            .unwrap();
        assert!(!output.status.success());
    }
}
