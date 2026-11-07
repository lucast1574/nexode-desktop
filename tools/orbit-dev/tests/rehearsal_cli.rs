#![cfg(unix)]
mod support;
use std::{fs, process::Command};
#[test]
fn preparation_needs_no_python_gzip_git_or_shell() {
    let tmp = tempfile::tempdir().unwrap();
    let binary = tmp.path().join("payload");
    fs::write(&binary, b"native test payload").unwrap();
    let root = tmp.path().join("sandbox");
    let result = Command::new(support::binary("orbit-dev"))
        .env("PATH", "")
        .args([
            "update-rehearsal",
            "--no-strip",
            "--prepare-only",
            "--binary",
        ])
        .arg(&binary)
        .arg("--root")
        .arg(&root)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        fs::read_link(root.join("home/bin/nexode")).unwrap(),
        root.join("home/builds/current/nexode")
    );
    assert!(root.join("cache/release.tar.gz").is_file());
    let script = fs::read_to_string(root.join("home/bin/nexode")).unwrap();
    assert!(script.contains("NEXODE_RUNTIME_RELEASE_SEMVER"));
    assert_eq!(fs::read(&binary).unwrap(), b"native test payload");
}
