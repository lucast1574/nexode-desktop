#![cfg(unix)]
use std::{fs, os::unix::fs::PermissionsExt, process::Command};
#[test]
fn antigravity_shell_parses_native_json_and_rejects_malformed_turns() {
    let d = tempfile::tempdir().unwrap();
    let bin = d.path().join("fixture");
    fs::write(
        &bin,
        r#"#!/bin/sh
case "$*" in
*'Remember this magic'*) echo '{"session_id":"session_fixture","text":"READY"}';;
*'What was the magic'*) echo '{"session_id":"session_fixture","text":"ZEBRA42"}';;
*) echo '{"session_id":"session_fixture","text":"TOOLOUT77 ZEBRA42"}';;
esac
"#,
    )
    .unwrap();
    fs::set_permissions(&bin, fs::Permissions::from_mode(0o700)).unwrap();
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../scripts/antigravity_multiturn_coverage.sh");
    let run = || {
        Command::new("bash")
            .arg(&script)
            .env("ORBIT_DEV_BIN", env!("CARGO_BIN_EXE_orbit-dev"))
            .env("JC", &bin)
            .env("MODELS", "fixture-model")
            .output()
            .unwrap()
    };
    let result = run();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(String::from_utf8_lossy(&result.stdout).contains("passed 4-turn"));
    fs::write(&bin, "#!/bin/sh\necho malformed\n").unwrap();
    assert!(!run().status.success());
}
