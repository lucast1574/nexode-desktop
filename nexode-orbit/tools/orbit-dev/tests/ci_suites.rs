#![cfg(unix)]
mod support;
use std::{
    fs,
    process::Command,
    time::{Duration, Instant},
};
use support::*;
fn fixture() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    executable(
        tmp.path().join("cargo"),
        r#"#!/bin/sh
printf '%s\n' "$*" >> "$SUITE_LOG"
case "$MODE" in
 fail) exit 17 ;;
 slow) sleep 60 & echo "$!" > "$SUITE_CHILD"; wait ;;
esac
"#,
    );
    tmp
}
fn command(tmp: &tempfile::TempDir, mode: &str) -> Command {
    let mut c = Command::new(binary("orbit-dev"));
    c.arg("test-ci-suites")
        .env_clear()
        .env("PATH", path(tmp.path()))
        .env("MODE", mode)
        .env("SUITE_LOG", tmp.path().join("log"))
        .env("SUITE_CHILD", tmp.path().join("child"));
    c
}
#[test]
fn suites_order_thread_policy_failure_and_timeout() {
    let t = fixture();
    let text = success(command(&t, "").output().unwrap());
    assert!(text.contains("All selected"));
    let lines = fs::read_to_string(t.path().join("log")).unwrap();
    assert_eq!(lines.lines().count(), 3);
    assert!(
        lines
            .lines()
            .all(|l| l.contains("--locked") && l.ends_with("-- --test-threads=1"))
    );
    fs::remove_file(t.path().join("log")).unwrap();
    success(
        command(&t, "")
            .args(["e2e", "--parallel"])
            .output()
            .unwrap(),
    );
    assert_eq!(
        fs::read_to_string(t.path().join("log")).unwrap().trim(),
        "test --locked --test e2e"
    );
    fs::remove_file(t.path().join("log")).unwrap();
    assert_eq!(
        command(&t, "fail").output().unwrap().status.code(),
        Some(17)
    );
    assert_eq!(
        fs::read_to_string(t.path().join("log"))
            .unwrap()
            .lines()
            .count(),
        1
    );
    let start = Instant::now();
    assert_eq!(
        command(&t, "slow")
            .args(["e2e", "--timeout-scale", "0.0001"])
            .output()
            .unwrap()
            .status
            .code(),
        Some(124)
    );
    assert!(start.elapsed() < Duration::from_secs(8));
    assert_child_stopped(&t);
    for value in ["0", "NaN", "inf", "1001"] {
        assert!(
            !command(&t, "")
                .args(["--timeout-scale", value])
                .output()
                .unwrap()
                .status
                .success()
        );
    }
}
fn assert_child_stopped(t: &tempfile::TempDir) {
    let pid: i32 = fs::read_to_string(t.path().join("child"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if unsafe { libc::kill(pid, 0) } != 0 {
            return;
        }
        #[cfg(target_os = "linux")]
        if fs::read_to_string(format!("/proc/{pid}/stat"))
            .is_ok_and(|s| s.split_once(") ").is_some_and(|(_, s)| s.starts_with('Z')))
        {
            return;
        }
        assert!(Instant::now() < deadline, "child {pid} still running");
        std::thread::sleep(Duration::from_millis(20));
    }
}
#[test]
fn interrupt_reaps_process_group_and_returns_signal_status() {
    let t = fixture();
    let mut child = command(&t, "slow")
        .arg("e2e")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !t.path().join("child").exists() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    unsafe {
        libc::kill(child.id() as i32, libc::SIGTERM);
    }
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert_eq!(status.code(), Some(143));
            break;
        }
        assert!(Instant::now() < deadline + Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_child_stopped(&t);
}
