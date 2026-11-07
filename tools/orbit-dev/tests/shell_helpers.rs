use std::{
    fs,
    io::Write,
    process::{Command, Stdio},
};
#[test]
fn marker_cleanup_preserves_binary_bytes_and_only_one_suffix() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("source.rs");
    let original = b"// \xff\r\n// bench_selfdev_build touch\r\n";
    let mut modified = original.to_vec();
    modified.extend_from_slice(b"\n// bench_selfdev_build touch\n");
    fs::write(&path, modified).unwrap();
    for _ in 0..2 {
        assert!(
            Command::new(env!("CARGO_BIN_EXE_orbit-dev"))
                .arg("strip-selfdev-marker")
                .arg(&path)
                .status()
                .unwrap()
                .success()
        );
        assert_eq!(fs::read(&path).unwrap(), original);
    }
}
#[test]
fn kitty_stdin_and_replay_dry_run_are_native_and_preserve_payloads() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_orbit-dev"))
        .args(["kitty-window", "id", "demo ' $(touch marker)"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(br#"[{"tabs":[{"windows":[{"id":42,"title":"demo ' $(touch marker)"}]}]}]"#)
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    assert_eq!(String::from_utf8(out.stdout).unwrap().trim(), "42");
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("recording.json");
    fs::write(
        &path,
        br#"[{"offset_ms":12,"event":{"type":"Key","data":{"code":"Char(';')"}}}]"#,
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_orbit-dev"))
        .arg("replay-keys")
        .arg(path)
        .arg("--dry-run")
        .env("PATH", d.path())
        .output()
        .unwrap();
    assert!(out.status.success());
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value["delay_ms"], 12);
    assert_eq!(value["argv"], serde_json::json!(["--", ";"]));
}
