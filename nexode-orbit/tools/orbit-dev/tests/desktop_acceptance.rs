#![cfg(unix)]
mod support;
use std::{fs, process::Command};

#[test]
fn compile_only_retains_real_sdk_source_and_never_starts_daemons() {
    let d = tempfile::tempdir().unwrap();
    let desktop = d.path().join("desktop");
    fs::create_dir_all(desktop.join("crates/nexode-desktop-ui/src")).unwrap();
    let bin = d.path().join("bin");
    fs::create_dir(&bin).unwrap();
    support::executable(
        bin.join("cargo"),
        "#!/bin/sh\n[ \"$1\" = build ] && [ \"$2\" = --offline ]\n",
    );
    let runtime = bin.join("runtime");
    support::executable(
        &runtime,
        &format!(
            "#!/bin/sh\ntouch '{}'\nexit 91\n",
            d.path().join("started").display()
        ),
    );
    let out = d.path().join("artifacts");
    let result = Command::new(env!("CARGO_BIN_EXE_orbit-dev"))
        .args(["test-desktop-selfdev", "--nexode-binary"])
        .arg(&runtime)
        .arg("--bridge-binary")
        .arg(&runtime)
        .arg("--desktop-repo")
        .arg(&desktop)
        .arg("--output-dir")
        .arg(&out)
        .arg("--compile-only")
        .env("PATH", support::path(&bin))
        .output()
        .unwrap();
    support::success(result);
    assert!(!d.path().join("started").exists());
    let source = fs::read_to_string(out.join("sdk/src/main.rs")).unwrap();
    assert!(source.contains("NexodeClient::connect"));
    assert!(source.contains("no_reply: true"));
    assert!(source.contains("history).unwrap()"));
    assert!(
        out.join("desktop-link")
            .canonicalize()
            .unwrap()
            .ends_with("crates/nexode-desktop-ui/src")
    );
    let manifest: toml::Value = fs::read_to_string(out.join("sdk/Cargo.toml"))
        .unwrap()
        .parse()
        .unwrap();
    assert!(
        manifest["dependencies"]["nexode-sdk"]["path"]
            .as_str()
            .unwrap()
            .ends_with("crates/nexode-sdk")
    );
}

#[test]
fn artifact_checkout_guard_resolves_symlinked_parents() {
    use std::os::unix::fs::symlink;
    let d = tempfile::tempdir().unwrap();
    let desktop = d.path().join("desktop");
    fs::create_dir_all(desktop.join("crates/nexode-desktop-ui/src")).unwrap();
    let link = d.path().join("link");
    symlink(&desktop, &link).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_orbit-dev"))
        .args([
            "test-desktop-selfdev",
            "--nexode-binary",
            "/bin/true",
            "--bridge-binary",
            "/bin/true",
        ])
        .arg("--desktop-repo")
        .arg(&desktop)
        .arg("--output-dir")
        .arg(link.join("artifacts"))
        .arg("--compile-only")
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("outside both repositories"));
    assert!(!desktop.join("artifacts").exists());
}

#[cfg(target_os = "linux")]
#[test]
fn full_orchestration_is_isolated_and_reaps_daemon_after_sdk_failure() {
    for fail in [false, true] {
        let d = tempfile::tempdir().unwrap();
        let desktop = d.path().join("desktop");
        fs::create_dir_all(desktop.join("crates/nexode-desktop-ui/src")).unwrap();
        let bin = d.path().join("bin");
        fs::create_dir(&bin).unwrap();
        let fixture = env!("CARGO_BIN_EXE_orbit-fixture");
        support::executable(
            bin.join("daemon"),
            &format!("#!/bin/sh\nexec '{fixture}' --desktop-daemon-fixture \"$@\"\n"),
        );
        support::executable(
            bin.join("bridge"),
            &format!(
                "#!/bin/sh\nprintf '%s' $$ > \"$NEXODE_RUNTIME_DIR/bridge-pid\"\nexec '{fixture}' --bridge-fixture \"$@\"\n"
            ),
        );
        let helper = if fail {
            "exit 9"
        } else {
            "printf '%s\\n' '{\"mode\":\"desktop\"}' '{\"mode\":\"desktop\"}' '{\"mode\":\"desktop\"}' '{\"mode\":\"regular\"}' '{\"mode\":\"cli\"}'"
        };
        support::executable(
            bin.join("cargo"),
            &format!(
                r#"#!/bin/sh
set -eu
while [ "$1" != --target-dir ]; do shift; done
mkdir -p "$2/debug"
cat > "$2/debug/desktop-selfdev-sdk-acceptance" <<'EOF'
#!/bin/sh
[ -z "${{REAL_PROVIDER_SECRET:-}}" ] || exit 90
{helper}
EOF
chmod +x "$2/debug/desktop-selfdev-sdk-acceptance"
"#
            ),
        );
        let root = d.path().join("out");
        let out = Command::new(env!("CARGO_BIN_EXE_orbit-dev"))
            .args(["test-desktop-selfdev", "--nexode-binary"])
            .arg(bin.join("daemon"))
            .arg("--bridge-binary")
            .arg(bin.join("bridge"))
            .arg("--desktop-repo")
            .arg(&desktop)
            .arg("--output-dir")
            .arg(&root)
            .env("PATH", support::path(&bin))
            .env("REAL_PROVIDER_SECRET", "not-inherited")
            .output()
            .unwrap();
        assert_eq!(
            out.status.success(),
            !fail,
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(root.join("acceptance.json").exists(), !fail);
        for name in ["daemon-pid", "bridge-pid"] {
            let pid = fs::read_to_string(root.join("runtime").join(name)).unwrap();
            assert!(
                !std::path::Path::new(&format!("/proc/{pid}")).exists(),
                "owned process survived"
            );
        }
    }
}
