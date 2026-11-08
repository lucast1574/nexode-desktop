#![cfg(unix)]
mod support;
use std::{fs, process::Command};
use support::*;
#[test]
fn workflow_preserves_release_and_toolchains() {
    let w = read(".github/workflows/release.yml");
    for s in [
        "CARGO_PROFILE_RELEASE_OPT_LEVEL: \"3\"",
        "JUSTRUST_TOOLCHAIN: system",
        "RUSTUP_TOOLCHAIN=nightly justrust build --local --locked -Z build-std=std,panic_abort",
        "& justrust @cargoArgs",
        "if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }",
    ] {
        assert!(w.contains(s), "{s}");
    }
    let c = read("Cargo.toml");
    assert!(c.contains("[profile.release]\nopt-level = 1"));
    assert!(c.contains("[profile.selfdev]\ninherits = \"release\"\nopt-level = 0"));
}
#[test]
fn freebsd_lock_guard_without_git_metadata() {
    let w = read(".github/workflows/release.yml");
    let vm = w.split_once("  build-freebsd:").unwrap().1;
    let fragment = vm[vm.find("            lock_snapshot=").unwrap()..]
        .split_once("            echo \"::endgroup::\"")
        .unwrap()
        .0;
    let fragment = dedent(fragment);
    assert!(fragment.contains("cargo build --locked --release -p nexode --bin nexode"));
    assert!(!fragment.contains("git "));
    for mode in [
        "unchanged",
        "modified",
        "missing",
        "missing_before_build",
        "missing_snapshot",
        "build_failed",
    ] {
        let t = tempfile::tempdir().unwrap();
        let directory = t.path();
        let source = directory.join("source");
        fs::create_dir(&source).unwrap();
        assert!(!source.join(".git").exists());
        if mode != "missing_before_build" {
            write(source.join("Cargo.lock"), "original lockfile\n");
        }
        executable(
            directory.join("cargo"),
            r#"#!/bin/sh
printf "%s\n" "$@" > cargo-args
case "$lock_snapshot" in "$PWD"/*) exit 90 ;; esac
printf "%s" "$lock_snapshot" > snapshot-path
case "$LOCK_TEST_MODE" in
modified) printf "changed\n" > Cargo.lock ;;
missing) rm Cargo.lock ;;
missing_snapshot) rm "$lock_snapshot" ;;
build_failed) exit 42 ;;
esac
"#,
        );
        let out = Command::new("/bin/sh")
            .args(["-ec", &format!("export lock_snapshot\n{fragment}")])
            .current_dir(&source)
            .env("PATH", path(directory))
            .env("LOCK_TEST_MODE", mode)
            .output()
            .unwrap();
        assert_eq!(
            out.status.success(),
            mode == "unchanged",
            "{mode}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        if mode == "missing_before_build" {
            assert!(!source.join("cargo-args").exists());
        } else {
            assert_eq!(
                fs::read_to_string(source.join("cargo-args"))
                    .unwrap()
                    .lines()
                    .collect::<Vec<_>>(),
                vec![
                    "build",
                    "--locked",
                    "--release",
                    "-p",
                    "nexode",
                    "--bin",
                    "nexode"
                ]
            );
            let p = fs::read_to_string(source.join("snapshot-path")).unwrap();
            assert!(
                !std::path::Path::new(&p).exists(),
                "snapshot not cleaned up"
            );
        }
    }
}
#[test]
fn compatibility_ci_and_local_selection() {
    let s = read("scripts/build_linux_compat.sh");
    for p in [
        "-e CARGO_PROFILE_RELEASE_OPT_LEVEL",
        "-e CARGO_BUILD_JOBS",
        "-e NEXODE_CI_BUILD",
    ] {
        assert!(s.contains(p));
    }
    let f = &s[s.find("build_command=(cargo build)").unwrap()..];
    let f = f
        .split_once("\n\n        # Exercise the exact portable binary")
        .unwrap()
        .0;
    let t = tempfile::tempdir().unwrap();
    for name in ["cargo", "justrust", "bash"] {
        executable(
            t.path().join(name),
            &format!(
                r#"#!/bin/sh
printf "%s" "{name}"
printf " <%s>" "$@"
printf " toolchain=%s slots=%s level=%s\n" "${{JUSTRUST_TOOLCHAIN:-}}" "${{JUSTRUST_SLOTS:-}}" "${{CARGO_PROFILE_RELEASE_OPT_LEVEL:-}}"
"#
            ),
        );
    }
    for ci in ["0", "1"] {
        let out = success(
            shell(f, t.path())
                .env("PATH", path(t.path()))
                .env("NEXODE_CI_BUILD", ci)
                .env("NEXODE_COMPAT_PROFILE", "release")
                .env("NEXODE_COMPAT_TARGET", "x86_64-unknown-linux-gnu")
                .env("CARGO_PROFILE_RELEASE_OPT_LEVEL", "3")
                .output()
                .unwrap(),
        );
        for s in [
            "<--locked>",
            "<--profile> <release> <--target> <x86_64-unknown-linux-gnu>",
            "<linux-compat-vendored-openssl>",
        ] {
            assert!(out.contains(s), "{out}");
        }
        if ci == "1" {
            for s in [
                "bash <scripts/install_justrust_ci.sh>",
                "justrust <build> <--local>",
                "toolchain=system slots=0 level=3",
            ] {
                assert!(out.contains(s), "{out}");
            }
        } else {
            assert!(out.contains("cargo <build>"));
            assert!(!out.contains("install_justrust"));
            assert!(!out.contains("justrust <build>"));
        }
    }
}
#[test]
fn provenance_check_drains_version_output() {
    let s = read("scripts/build_linux_compat.sh");
    let fragment = &s[s.find("if [[ -n \"$git_hash\" ]]; then").unwrap()..];
    let t = tempfile::tempdir().unwrap();
    fs::copy(binary("orbit-fixture"), t.path().join("nexode.bin")).unwrap();
    success(
        Command::new("/bin/bash")
            .args(["-euo", "pipefail", "-c", fragment])
            .env("git_hash", "abc123")
            .env("out_dir", t.path())
            .env("artifact", "nexode")
            .env("ORBIT_FIXTURE_MODE", "large-version")
            .output()
            .unwrap(),
    );
}
#[test]
fn cloud_artifact_gate_rejects_missing_protocol_and_newer_glibc() {
    let s = read("scripts/build_linux_compat.sh");
    let fragment = &s[s.find("        built=\"$CARGO_TARGET_DIR/").unwrap()..];
    let fragment = fragment.split_once("\n\n\t    cp ").unwrap().0;
    let t = tempfile::tempdir().unwrap();
    let directory = t.path();
    executable(
        directory.join("target/x86_64-unknown-linux-gnu/release/nexode"),
        r#"#!/bin/sh
printf "%s\n" "$*" >> "$TEST_CALLS"
case "$*" in *"cloud ${FAIL_CLOUD:-never} --help"*) exit 1 ;; esac
"#,
    );
    executable(
        directory.join("objdump"),
        "#!/bin/sh\nprintf \"0000 DF .text %s symbol\\n\" \"$TEST_GLIBC_SYMBOL\"\n",
    );
    for (glibc, missing) in [
        ("2.17", ""),
        ("2.17-legacy", ""),
        ("2.34-legacy", ""),
        ("2.34", ""),
        ("2.17", "receive"),
        ("2.17", "credentials"),
        ("2.17", "cancel-return"),
    ] {
        let calls = directory.join("calls");
        write(&calls, "");
        let symbol = if let Some(v) = glibc.strip_suffix("-legacy") {
            format!("GLIBC_{v}")
        } else {
            format!("(GLIBC_{glibc})")
        };
        let out = Command::new("/bin/bash")
            .args(["-euo", "pipefail", "-c", fragment])
            .env("PATH", path(directory))
            .env("CARGO_TARGET_DIR", directory.join("target"))
            .env("NEXODE_COMPAT_TARGET", "x86_64-unknown-linux-gnu")
            .env("NEXODE_COMPAT_PROFILE", "release")
            .env("TEST_GLIBC_SYMBOL", symbol)
            .env("FAIL_CLOUD", missing)
            .env("TEST_CALLS", &calls)
            .output()
            .unwrap();
        let expected = glibc.trim_end_matches("-legacy") == "2.17" && missing.is_empty();
        assert_eq!(
            out.status.success(),
            expected,
            "{glibc}/{missing}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        if expected {
            let calls = fs::read_to_string(calls).unwrap();
            for c in ["receive", "credentials", "cancel-return"] {
                assert!(calls.contains(&format!("cloud {c} --help")));
            }
        }
    }
}
