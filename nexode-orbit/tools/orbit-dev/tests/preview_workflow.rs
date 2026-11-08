#![cfg(unix)]
mod support;
use std::{fs, io::Read};
use support::*;
#[test]
fn preview_exact_source_nonrecursive_and_scoped_permissions() {
    let wf = read(".github/workflows/preview.yml");
    let triggers = between(&wf, "on:\n", "concurrency:");
    for text in ["branches: [master]", "  workflow_dispatch:\n"] {
        assert!(triggers.contains(text));
    }
    for text in ["inputs:", "tags:", "workflow_run:", "release:"] {
        assert!(!triggers.contains(text));
    }
    for text in [
        "github.ref == 'refs/heads/master'",
        "cancel-in-progress: ${{ github.event_name == 'push' }}",
        "group: v1-preview-master",
        "vars.NEXODE_V1_PREVIEW_ENABLED != 'false'",
        "NEXODE_BUILD_SEMVER: v1.0.0-preview.${{ github.run_number }}",
        "cargo xtask check-release-ci \"$GITHUB_SHA\"",
        "--timeout 10800 --preview",
        "toolchain: 1.99.0",
        "CARGO_PROFILE_RELEASE_OPT_LEVEL: \"3\"",
        "CARGO_INCREMENTAL: \"0\"",
        "GH_TOKEN: ${{ github.token }}",
        "gh workflow run discord-release.yml",
    ] {
        assert!(wf.contains(text), "{text}");
    }
    assert!(wf.find("run: sleep 900") < wf.find("cargo xtask check-release-ci"));
    assert_eq!(wf.matches("ref: ${{ github.sha }}").count(), 4);
    assert_eq!(wf.matches("contents: write").count(), 1);
    assert_eq!(wf.matches("            hosted: true").count(), 2);
    for text in [
        "continue-on-error:",
        "quick-release",
        "npm ",
        "brew ",
        "DEPLOY_KEY",
        "latest.json",
        "push origin",
        "secrets.PAT",
        "gh workflow run release.yml",
    ] {
        assert!(!wf.contains(text), "{text}");
    }
    assert!(
        wf.find("bash scripts/build_preview_linux_hosted.sh")
            < wf.find("bash scripts/package_preview_linux_hosted.sh")
    );
    let hosted = between(
        &wf,
        "      - name: Build Linux preview on hosted justrust\n",
        "      - name:",
    );
    assert!(hosted.contains("if: matrix.hosted == true"));
    assert!(hosted.contains("secrets.JUSTRUST_API_KEY"));
    assert_eq!(wf.matches("secrets.JUSTRUST_API_KEY").count(), 1);
    let native = between(&wf, "      - name: Build native preview\n", "      - name:");
    assert!(native.contains("if: matrix.hosted != true && runner.os != 'Windows'"));
    assert!(!native.contains("secrets."));
    let windows = between(
        &wf,
        "      - name: Build Windows preview with MSVC environment\n",
        "      - name:",
    );
    for text in [
        "shell: pwsh",
        "$extraArgs = @()",
        "$env:EXTRA_ARGS -split '\\s+'",
        "--target $env:TARGET -p nexode --bin nexode @extraArgs",
        "if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }",
    ] {
        assert!(windows.contains(text), "{text}");
    }
    let stable = read(".github/workflows/release.yml");
    assert!(stable.contains("- '!v*-preview.*'"));
    assert!(stable.contains("!contains(inputs.tag || github.ref_name, '-preview.')"));
    assert!(stable.contains("git archive \"$WORKFLOW_SHA\" tools/orbit-dev"));
    assert!(stable.contains("check-release-ci \"$TAG_SHA\""));
    assert!(!stable.contains("--preview"));
}
#[test]
fn native_shell_arguments_preserve_empty_options_and_arm_package_never_executes() {
    let wf = read(".github/workflows/preview.yml");
    let native = between(&wf, "      - name: Build native preview\n", "      - name:");
    let body = dedent(native.split_once("        run: |\n").unwrap().1);
    assert!(body.contains("bin nexode \"$@\""));
    for (extra, tail) in [
        ("", vec![]),
        (
            "--no-default-features --features pdf",
            vec!["--no-default-features", "--features", "pdf"],
        ),
    ] {
        let tmp = tempfile::tempdir().unwrap();
        let out = shell(
            &format!("justrust() {{ printf '%s\\n' \"$@\"; }}\n{body}"),
            tmp.path(),
        )
        .env("TARGET", "aarch64-apple-darwin")
        .env("EXTRA_ARGS", extra)
        .output()
        .unwrap();
        assert!(out.status.success());
        let mut expected = vec![
            "build",
            "--local",
            "--locked",
            "--release",
            "--target",
            "aarch64-apple-darwin",
            "-p",
            "nexode",
            "--bin",
            "nexode",
        ];
        expected.extend(tail);
        assert_eq!(
            String::from_utf8(out.stdout)
                .unwrap()
                .lines()
                .collect::<Vec<_>>(),
            expected
        );
    }
    let package = between(
        &wf,
        "      - name: Package native binary\n",
        "      - name:",
    );
    let body = dedent(package.split_once("        run: |\n").unwrap().1);
    let tmp = tempfile::tempdir().unwrap();
    executable(
        tmp.path()
            .join("target/aarch64-unknown-linux-gnu/release/nexode"),
        "#!/bin/sh\necho executed > should-not-execute\nexit 99\n",
    );
    let out = shell(&body, tmp.path())
        .env("TARGET", "aarch64-unknown-linux-gnu")
        .env("ARTIFACT", "nexode-linux-aarch64")
        .env("RUNNER_OS", "Linux")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!tmp.path().join("should-not-execute").exists());
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(
        fs::File::open(tmp.path().join("dist/nexode-linux-aarch64.tar.gz")).unwrap(),
    ));
    let mut entries = archive.entries().unwrap();
    let mut entry = entries.next().unwrap().unwrap();
    assert_eq!(
        entry.path().unwrap().to_str().unwrap(),
        "nexode-linux-aarch64"
    );
    let mut bytes = vec![];
    entry.read_to_end(&mut bytes).unwrap();
    assert_eq!(
        bytes,
        fs::read(
            tmp.path()
                .join("target/aarch64-unknown-linux-gnu/release/nexode")
        )
        .unwrap()
    );
    assert!(entries.next().is_none());
}
#[test]
fn arm_gate_consumes_uploaded_artifact_and_recovery_only_trusted_master() {
    let wf = read(".github/workflows/preview.yml");
    let build = between(&wf, "  build:\n", "  native-arm-runtime:\n");
    assert!(!build.contains("ubuntu-24.04-arm"));
    assert_eq!(build.matches("            artifact: nexode-").count(), 6);
    assert!(build.contains("name: preview-${{ matrix.artifact }}"));
    assert!(build.contains("path: dist/${{ matrix.artifact }}.tar.gz"));
    let gate = between(&wf, "  native-arm-runtime:\n", "  publish:\n");
    for text in [
        "needs: build",
        "runs-on: ubuntu-24.04-arm",
        "uses: actions/download-artifact@v4",
        "name: preview-nexode-linux-aarch64",
        "cargo xtask verify-preview-arm-runtime dist/nexode-linux-aarch64.tar.gz",
    ] {
        assert!(gate.contains(text));
    }
    for text in [
        "secrets.",
        "justrust",
        "upload-artifact",
        "tar czf",
        "continue-on-error",
        "if:",
    ] {
        assert!(!gate.contains(text));
    }
    let publish = between(&wf, "  publish:\n", "  publish-sdk:\n");
    assert!(publish.contains("needs: [quiet-and-ci, build, native-arm-runtime]"));
    assert!(!publish.contains("always()"));
    assert!(publish.contains("pattern: preview-nexode-*"));
    let recovery = read(".github/workflows/preview-recovery.yml");
    for text in [
        "workflows: [CI]",
        "types: [completed]",
        "branches: [master]",
        "schedule:",
        "cancel-in-progress: false",
        "contents: read",
        "actions: write",
        "ref: master",
        "persist-credentials: false",
        "github.event.workflow_run.event == 'push'",
        "github.event.workflow_run.head_repository.full_name == github.repository",
        "vars.NEXODE_V1_PREVIEW_ENABLED != 'false'",
        "cargo xtask reconcile-preview-release",
    ] {
        assert!(recovery.contains(text), "{text}");
    }
    for text in ["contents: write", "download-artifact", "head_sha }}"] {
        assert!(!recovery.contains(text));
    }
}
