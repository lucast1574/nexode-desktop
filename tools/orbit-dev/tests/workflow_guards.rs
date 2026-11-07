#![cfg(unix)]
mod support;
use regex::Regex;
use support::*;
#[test]
fn npm_duplicate_upload_defers_only_exact_conflict_to_visibility_gate() {
    let wf = read(".github/workflows/publish-typescript-sdk.yml");
    let block = between(
        &wf,
        "      - name: Publish platform runtime packages\n",
        "      - name: Wait for platform runtime registry visibility\n",
    );
    let script = dedent(block.split_once("        run: |\n").unwrap().1)
        .replace("${{ steps.channel.outputs.tag }}", "preview");
    for (code, message, expected) in [
        (
            "E409",
            "Cannot publish over previously staged version \"1.2.0-preview.49\".",
            0,
        ),
        ("E409", "Unrelated conflict", 1),
        ("E404", "Not authorized", 1),
    ] {
        let t = tempfile::tempdir().unwrap();
        let root = t.path();
        write(
            root.join("sdk/npm/runtime/package.json"),
            r#"{"name":"@test/runtime","version":"1.2.0-preview.49"}"#,
        );
        executable(
            root.join("npm"),
            "#!/bin/sh\nif [ \"$1\" = view ]; then exit 1; fi\necho \"npm error code $ERROR_CODE\" >&2\necho \"$ERROR_MESSAGE\" >&2\nexit 1\n",
        );
        let out = shell(&script, root)
            .env("PATH", path(root))
            .env("ERROR_CODE", code)
            .env("ERROR_MESSAGE", message)
            .output()
            .unwrap();
        assert_eq!(
            out.status.code(),
            Some(expected),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}
#[test]
fn sdk_waits_for_runtime_visibility_and_fails_closed() {
    let wf = read(".github/workflows/publish-typescript-sdk.yml");
    let guard = between(
        &wf,
        "      - name: Wait for platform runtime registry visibility\n",
        "      - name: Publish with npm provenance\n",
    );
    let script = dedent(guard.split_once("        run: |\n").unwrap().1);
    assert!(script.contains("--prefer-online"));
    for mode in ["retry", "missing"] {
        let t = tempfile::tempdir().unwrap();
        let root = t.path();
        write(
            root.join("sdk/npm/runtime/package.json"),
            r#"{"name":"@test/runtime","version":"1.2.0-preview.49"}"#,
        );
        executable(
            root.join("bin/npm"),
            "#!/bin/sh\nif [ \"$VISIBILITY_MODE\" = retry ] && [ -f \"$MARKER\" ]; then\n echo 1.2.0-preview.49\n exit 0\nfi\ntouch \"$MARKER\"\nexit 1\n",
        );
        executable(root.join("bin/sleep"), "#!/bin/sh\nexit 0\n");
        let out = shell(&script, root)
            .env("PATH", path(&root.join("bin")))
            .env("VISIBILITY_MODE", mode)
            .env("MARKER", root.join("attempted"))
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(if mode == "retry" { 0 } else { 1 }));
        assert!(String::from_utf8_lossy(&out.stdout).contains("Waiting for registry visibility"));
        if mode == "missing" {
            assert!(String::from_utf8_lossy(&out.stderr).contains("refusing to publish SDK"));
        }
    }
}
#[test]
fn preview_dispatches_standalone_trusted_npm_publisher() {
    let wf = read(".github/workflows/preview.yml");
    let publish = between(&wf, "\n  publish-sdk:\n", "\n  announce:");
    for s in [
        "needs: publish",
        "actions: write",
        "gh workflow run publish-typescript-sdk.yml",
        "--ref master",
        "nexode_release_tag=$NEXODE_BUILD_SEMVER",
    ] {
        assert!(publish.contains(s), "{s}");
    }
    assert!(!publish.contains("uses: ./.github/workflows/publish-typescript-sdk.yml"));
}
#[test]
fn exact_source_ci_can_be_manually_retried_after_bot_push() {
    let wf = read(".github/workflows/ci.yml");
    let triggers = between(&wf, "\non:\n", "\nconcurrency:");
    assert!(triggers.contains("\n  workflow_dispatch:\n"));
    assert!(triggers.contains("\n  push:\n    branches: [main, master]"));
    assert!(!triggers.contains("inputs:"));
}
#[test]
fn ci_builds_and_tests_cannot_silently_refresh_lockfile() {
    let wf = read(".github/workflows/ci.yml");
    let pattern = Regex::new(
        r#"(?:cargo|"\$CARGO_BIN"|"\$\(rustup which cargo\)") (?:build|check|test|clippy)\b"#,
    )
    .unwrap();
    let mut checked = 0;
    for line in wf
        .lines()
        .filter(|l| !l.trim_start().starts_with('#') && pattern.is_match(l))
    {
        assert!(line.contains("--locked"), "{line}");
        checked += 1;
    }
    assert!(checked >= 15);
    assert!(wf.contains("git diff --exit-code -- Cargo.lock"));
}
#[test]
fn arm64_release_graph_is_mandatory_native_windows_check() {
    let wf = read(".github/workflows/ci.yml");
    let arm = between(&wf, "  windows-arm64-check:\n", "\n  windows-cross-check:");
    for s in [
        "runs-on: windows-latest",
        "arch: amd64_arm64",
        "targets: aarch64-pc-windows-msvc",
        "JUSTRUST_TOOLCHAIN: system",
        "JUSTRUST_DISABLE: \"1\"",
        "& justrust check --local --locked -p nexode --bins --target aarch64-pc-windows-msvc --no-default-features --features pdf",
        "if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }",
    ] {
        assert!(arm.contains(s), "{s}");
    }
    assert!(!arm.contains("continue-on-error:"));
    assert!(!arm.contains("if:"));
}
#[test]
fn quality_gates_report_failures_without_weakening_them() {
    let wf = read(".github/workflows/ci.yml");
    let quality = between(&wf, "  quality:\n", "\n  release-automation:");
    let steps: Vec<_> = quality.split("      - name: ").skip(1).collect();
    assert!(steps.len() >= 15);
    let weakened = Regex::new(r"\bcontinue-on-error:\s*true\b").unwrap();
    for step in steps {
        if step.lines().next() != Some("Check module declarations resolve") {
            assert!(step.contains("if: ${{ !cancelled() }}"), "{step}");
        }
        assert!(!weakened.is_match(step));
    }
    assert!(!quality.contains("continue-on-error: true"));
    assert!(quality.contains("cargo clippy --locked --all-targets --all-features -- -D warnings"));
    for c in [
        "check-code-size",
        "check-test-size",
        "check-panics",
        "check-swallowed-errors",
    ] {
        assert!(quality.contains(&format!("run: cargo xtask {c}\n")));
    }
}
#[test]
fn compiler_lookup_failure_is_not_masked_by_export() {
    let wf = read(".github/workflows/ci.yml");
    let re = Regex::new(
        r"(?m)^          (RUSTC=.*)\n          (export RUSTC)\n          (CARGO_BIN=.*)\n",
    )
    .unwrap();
    let c = re
        .captures(&wf)
        .expect("compiler discovery separate from export");
    let script = format!("{}\n{}\n{}\nprintf should-not-run\n", &c[1], &c[2], &c[3]);
    let t = tempfile::tempdir().unwrap();
    executable(t.path().join("rustup"), "#!/bin/sh\nexit 17\n");
    let out = shell(&script, t.path())
        .env("PATH", path(t.path()))
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(17));
    assert!(!String::from_utf8_lossy(&out.stdout).contains("should-not-run"));
}
#[test]
fn launch_gate_uses_test_module_platform_guard() {
    let source = read("crates/nexode-app-core/src/tool/bash.rs");
    let module =
        Regex::new(r##"#\[cfg\(([^\n]+)\)\]\s*#\[path = "bash_tests.rs"\]\s*mod tests;"##).unwrap();
    let hook=Regex::new(r"#\[cfg\(([^\n]+)\)\]\s*tests::wait_for_session_env_launch_gate\(&session_id_env\)\.await;").unwrap();
    assert_eq!(
        &module.captures(&source).unwrap()[1],
        "all(test, not(windows))"
    );
    assert_eq!(
        &hook.captures(&source).unwrap()[1],
        &module.captures(&source).unwrap()[1]
    );
}
