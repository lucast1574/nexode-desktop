#![cfg(unix)]
use serde_json::{Value, json};
use std::{fs, os::unix::fs::PermissionsExt, process::Command};
fn fake(root: &std::path::Path) -> std::path::PathBuf {
    let path = root.join("nexode-fixture");
    fs::write(&path,r#"#!/bin/sh
if [ "$1" = --version ]; then echo 'nexode v9 (abcdef123-dirty)'; exit 0; fi
for arg do
 case "$arg" in
 serve) exec sleep 300;;
 debug) echo '{}'; exit 0;;
 esac
done
for last do :; done
case "$last" in
 *MISSING*) echo 'provider error: missing api key' >&2; exit 1;;
 *Draft*) printf '%s\n' '{"type":"done","text":"draft"}'; exit 0;;
esac
printf '%s\n' '{"type":"tool_start","id":"a","name":"integration_tools"}' '{"type":"tool_input","delta":"{\"category\":\"payments\"}"}' '{"type":"tool_done","id":"a","name":"integration_tools","output":"Available integrations in '\''payments'\'':\n- agentcard: cards"}' '{"type":"done","text":"done"}'
"#).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    path
}
#[test]
fn listing_cli_keeps_control_single_attempt_and_rate_excludes_auth_failure() {
    let root = tempfile::tempdir().unwrap();
    let binary = fake(root.path());
    let cases = root.path().join("cases.json");
    let catalog = root.path().join("catalog.json");
    let output = root.path().join("report.json");
    fs::write(&cases,json!({"version":2,"cases":[{"id":"positive","prompt":"Buy a sample.","expected_category":"payments","expected_tool":"agentcard"},{"id":"control","prompt":"Draft a letter.","expectation":"no-discovery"}]}).to_string()).unwrap();
    fs::write(
        &catalog,
        json!({"payments":[{"name":"agentcard"}]}).to_string(),
    )
    .unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_orbit-dev"))
        .arg("benchmark-discovery")
        .arg("--nexode")
        .arg(&binary)
        .arg("--cases")
        .arg(&cases)
        .arg("--catalog-file")
        .arg(&catalog)
        .arg("--output")
        .arg(&output)
        .args(["--retry-delay", "0", "--timeout", "3"])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report: Value = serde_json::from_slice(&fs::read(&output).unwrap()).unwrap();
    assert_eq!(report["passed"], true);
    assert_eq!(
        report["results"][1]["trials"][0]["attempts"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    fs::write(&cases,json!({"version":1,"cases":[{"id":"positive","expect":"call","prompt":"Buy a sample.","expected_category":"payments"},{"id":"control","expect":"no-call","prompt":"Draft a letter."},{"id":"auth","expect":"call","prompt":"MISSING credential fixture."}]}).to_string()).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_orbit-dev"))
        .arg("benchmark-discovery-rate")
        .arg("--nexode")
        .arg(&binary)
        .arg("--cases")
        .arg(&cases)
        .arg("--output")
        .arg(&output)
        .args([
            "--retry-delay",
            "0",
            "--invalid-retries",
            "0",
            "--timeout",
            "3",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report: Value = serde_json::from_slice(&fs::read(&output).unwrap()).unwrap();
    assert_eq!(report["summary"]["scored_trial_count"], 2);
    assert_eq!(report["summary"]["invalid_trial_count"], 1);
    assert_eq!(report["summary"]["recall_browse_rate"], 1.);
    assert_eq!(report["config"]["executable"]["commit"], "abcdef123");
}
