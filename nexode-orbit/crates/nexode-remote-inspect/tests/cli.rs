#[cfg(unix)]
#[test]
fn collector_runs_without_python_or_a_shell_in_path() {
    use serde_json::json;
    use std::{fs, process::Command};
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path();
    let state = home.join(".nexode");
    let project = home.join("project ' 雲");
    fs::create_dir(&project).unwrap();
    for name in ["active_pids", "streaming_pids", "sessions"] {
        fs::create_dir_all(state.join(name)).unwrap();
    }
    let pid = std::process::id().to_string();
    fs::write(state.join("active_pids/real"), &pid).unwrap();
    fs::write(state.join("streaming_pids/real"), &pid).unwrap();
    fs::write(
        state.join("sessions/real.json"),
        json!({"working_dir":project}).to_string(),
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_nexode-remote-inspect"))
        .arg(&project)
        .env("HOME", home)
        .env("NEXODE_HOME", state)
        .env("PATH", "")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let s: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(s["current_path"], "~/project ' 雲");
    let entry = s["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["path"] == "~/project ' 雲")
        .unwrap();
    assert_eq!(entry["agents"], 1);
    assert_eq!(entry["busy"], 1);
}
