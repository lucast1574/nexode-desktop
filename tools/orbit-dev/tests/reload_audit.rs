use serde_json::Value;
use std::{fs, process::Command};
#[test]
fn reload_audit_matches_original_log_trace_and_durable_correlation() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/reload_audit_reference.json")).unwrap();
    let root = tempfile::tempdir().unwrap();
    for (name, text) in fixture["files"].as_object().unwrap() {
        let path = root.path().join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text.as_str().unwrap()).unwrap();
    }
    let call = |extra: &[&str]| {
        let out = Command::new(env!("CARGO_BIN_EXE_orbit-dev"))
            .args(["reload-recovery-audit", "--home"])
            .arg(root.path())
            .args(["--json", "--show-lines"])
            .args(extra)
            .env("PATH", "")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice::<Value>(&out.stdout).unwrap()
    };
    let mut report = call(&[]);
    report.as_object_mut().unwrap().remove("home");
    assert_eq!(report, fixture["expected"]);
    let filtered = call(&["--reload-id", "1_1", "--session", "session_1"]);
    assert_eq!(filtered["records"].as_array().unwrap().len(), 1);
    assert_eq!(
        filtered["records"][0]["verdict"],
        "suspect-delivered-without-acceptance-log"
    );
    let empty = call(&["--session", "nonexistent"]);
    assert!(empty["records"].as_array().unwrap().is_empty());
    assert_eq!(empty["global_hidden_sends"].as_array().unwrap().len(), 1);
}
