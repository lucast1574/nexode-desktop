#![cfg(unix)]
mod support;
use std::process::{Command, Output};
use support::*;
fn watch(status: &str) -> Output {
    let t = tempfile::tempdir().unwrap();
    executable(
        t.path().join("gh"),
        "#!/bin/sh\ncase \"$*\" in *status,conclusion*) printf \"%s\\n\" \"$TEST_RUN_STATUS\" ;; *) printf \"job diagnostics\\n\" ;; esac\n",
    );
    Command::new("bash")
        .arg(root().join("scripts/watch_release_run.sh"))
        .args(["123", "1", "0"])
        .env("PATH", path(t.path()))
        .env("TEST_RUN_STATUS", status)
        .output()
        .unwrap()
}
#[test]
fn successful_run() {
    let out = success(watch("completed success"));
    assert!(out.contains("RUN SUCCESS"));
}
#[test]
fn all_non_success_terminal_conclusions_fail_immediately() {
    for c in [
        "failure",
        "cancelled",
        "timed_out",
        "action_required",
        "startup_failure",
        "stale",
        "skipped",
        "neutral",
    ] {
        let out = watch(&format!("completed {c}"));
        assert_eq!(out.status.code(), Some(1));
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(text.contains("RUN FAILED"));
        assert!(text.contains("job diagnostics"));
    }
}
#[test]
fn pending_does_not_claim_success_or_failure() {
    let out = watch("in_progress ");
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stdout).contains("RUN STILL RUNNING"));
}
#[test]
fn default_budget_covers_release_job_timeouts() {
    let script = read("scripts/watch_release_run.sh");
    assert!(script.contains("MAX_POLLS=\"${2:-480}\""));
    assert!(script.contains("SLEEP_S=\"${3:-30}\""));
}
