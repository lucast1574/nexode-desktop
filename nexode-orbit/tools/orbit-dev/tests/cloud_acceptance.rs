#![cfg(target_os = "linux")]
use std::{fs, process::Command};
#[test]
fn missing_prerequisites_fail_and_emit_both_reports() {
    let d = tempfile::tempdir().unwrap();
    let artifacts = d.path().join("report");
    let output = Command::new(env!("CARGO_BIN_EXE_orbit-dev"))
        .args([
            "test-cloud-acceptance",
            "--binary",
            "/bin/true",
            "--layers",
            "tui",
            "--artifacts",
        ])
        .arg(&artifacts)
        .env("PATH", d.path())
        .output()
        .unwrap();
    assert!(!output.status.success());
    let report: serde_json::Value =
        serde_json::from_slice(&fs::read(artifacts.join("report.json")).unwrap()).unwrap();
    assert_eq!(report["results"][0]["status"], "failed");
    assert!(
        report["results"][0]["error"]
            .as_str()
            .unwrap()
            .contains("Missing test prerequisites")
    );
    let text = fs::read_to_string(artifacts.join("junit.xml")).unwrap();
    let doc = roxmltree::Document::parse(&text).unwrap();
    assert_eq!(doc.root_element().attribute("failures"), Some("1"));
}
