use serde_json::{Value, json};
use std::{fs, path::Path, process::Command};
fn run(root: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_orbit-dev"))
        .arg("analyze-runtime-memory-log")
        .arg(root)
        .args(args)
        .env("PATH", root)
        .output()
        .unwrap()
}
#[test]
fn cli_handles_partial_lines_date_filters_and_latest_process_lifetimes_without_interpreter() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let row = |time, id, pss| json!({"timestamp_ms":time,"server":{"id":id},"process":{"os":{"pss_bytes":pss}}});
    fs::write(
        root.join("server-runtime-memory-2026-11-06.jsonl"),
        format!("{}\n{}\n", row(1, "old", 100), row(2, "old", 200)),
    )
    .unwrap();
    fs::write(
        root.join("server-runtime-memory-2026-11-07.jsonl"),
        format!("\n{}\n{}\n{{partial", row(3, "new", 20), row(4, "new", 20)),
    )
    .unwrap();
    fs::write(root.join("unrelated.jsonl"), "invalid").unwrap();
    let output = run(root, &["--json"]);
    assert!(output.status.success());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["instance_id"], "new");
    assert_eq!(value["sample_count"], 2);
    assert_eq!(value["process"]["peak_timestamp_ms"], 3);
    assert_eq!(value["process"]["net_pss_growth_bytes"], 0);
    let output = run(root, &["--json", "--days", "1", "--list-instances"]);
    assert!(output.status.success());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["instances"].as_array().unwrap().len(), 1);
    assert_eq!(value["files"].as_array().unwrap().len(), 1);
    let output = run(root, &["--json", "--instance", "old"]);
    assert!(output.status.success());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["process"]["net_pss_growth_bytes"], 100);
    assert!(!run(root, &["--instance", "missing"]).status.success());
    assert!(
        !run(root, &["--all-instances", "--instance", "old"])
            .status
            .success()
    );
    assert!(!run(root, &["--min-spike-mb", "NaN"]).status.success());
}
