//! Offline ABI/orchestration tests, not evidence of a real tenant-hosted build.
#![cfg(target_os = "linux")]
mod support;
use serde_json::Value;
use std::{
    fs,
    process::{Command, Output},
};
use support::*;
const X86: &str = "x86_64-unknown-linux-gnu";
const ARM: &str = "aarch64-unknown-linux-gnu";
struct Fixture {
    temp: tempfile::TempDir,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let r = temp.path().to_owned();
        fs::create_dir(r.join("scripts")).unwrap();
        fs::create_dir(r.join("bin")).unwrap();
        for script in ["build_preview_linux_hosted.sh", "orbit_dev.sh"] {
            fs::copy(
                root().join("scripts").join(script),
                r.join("scripts").join(script),
            )
            .unwrap();
        }
        for name in ["justrust", "rustup", "cargo", "readelf"] {
            std::os::unix::fs::symlink(binary("orbit-fixture"), r.join("bin").join(name)).unwrap();
        }
        let f = Self { temp };
        success(f.command("git").args(["init", "-q"]).output().unwrap());
        fs::write(r.join(".gitignore"), "/bin/\n/record*\n/target/\n").unwrap();
        success(
            f.command("git")
                .args(["add", "scripts", ".gitignore"])
                .output()
                .unwrap(),
        );
        success(
            f.command("git")
                .args([
                    "-c",
                    "user.name=Fixture",
                    "-c",
                    "user.email=fixture@example.invalid",
                    "commit",
                    "-qm",
                    "fixture source",
                ])
                .output()
                .unwrap(),
        );
        f
    }
    fn command(&self, name: &str) -> Command {
        let mut c = Command::new(name);
        c.current_dir(self.temp.path())
            .env_clear()
            .env("PATH", path(&self.temp.path().join("bin")))
            .env("HOME", self.temp.path())
            .env("ORBIT_HOSTED_FIXTURE", "1")
            .env("ORBIT_DEV_BIN", binary("orbit-dev"))
            .env("NEXODE_BUILD_SEMVER", "v1.0.0-preview.42")
            .env("RECORD", self.temp.path().join("record"));
        c
    }
    fn run(&self, target: &str, updates: &[(&str, &str)]) -> Output {
        self.command("bash")
            .arg("scripts/build_preview_linux_hosted.sh")
            .arg(target)
            .envs(updates.iter().copied())
            .output()
            .unwrap()
    }
    fn records(&self) -> Vec<Value> {
        fs::read_to_string(self.temp.path().join("record"))
            .unwrap_or_default()
            .lines()
            .map(|s| serde_json::from_str(s).unwrap())
            .collect()
    }
}
#[test]
fn x86_metadata_command_and_dirty_source_are_preserved() {
    let f = Fixture::new();
    let text = success(f.run(
        X86,
        &[
            ("JUSTRUST_TOOLCHAIN", "system"),
            ("CARGO_PROFILE_RELEASE_OPT_LEVEL", "1"),
            ("NEXODE_BUILD_GIT_HASH", "stale"),
            ("NEXODE_BUILD_GIT_DIRTY", "true"),
        ],
    ));
    let records = f.records();
    assert_eq!(records.len(), 2);
    assert_eq!(
        records[0]["args"],
        serde_json::json!(["remote", "use", "hosted"])
    );
    let build = &records[1];
    assert_eq!(
        build["args"],
        serde_json::json!([
            "zigbuild",
            "--remote",
            "--locked",
            "--release",
            "--target",
            format!("{X86}.2.17"),
            "-p",
            "nexode",
            "--bin",
            "nexode",
            "--features",
            "linux-compat-vendored-openssl"
        ])
    );
    for (k, v) in [
        ("JUSTRUST_REMOTE_REQUIRED", "1"),
        ("JUSTRUST_TOOLCHAIN_REQUIRED", "1"),
        ("JUSTRUST_TOOLCHAIN", "1.99.0"),
        ("CARGO_PROFILE_RELEASE_OPT_LEVEL", "3"),
        ("CARGO_INCREMENTAL", "0"),
        ("CARGO_BUILD_RUSTC_WRAPPER", ""),
        ("NEXODE_RELEASE_BUILD", "1"),
        ("NEXODE_BUILD_SEMVER", "v1.0.0-preview.42"),
        ("NEXODE_BUILD_GIT_DIRTY", "false"),
    ] {
        assert_eq!(build["env"][k], v, "{k}");
    }
    assert_eq!(
        build["env"]["NEXODE_BUILD_GIT_HASH"],
        success(
            f.command("git")
                .args(["rev-parse", "--short", "HEAD"])
                .output()
                .unwrap()
        )
        .trim()
    );
    assert!(
        build["env"]["NEXODE_BUILD_CHANGELOG_RAW"]
            .as_str()
            .unwrap()
            .contains("fixture source")
    );
    assert!(
        !build["env"]["NEXODE_BUILD_GIT_DATE"]
            .as_str()
            .unwrap()
            .is_empty()
    );
    assert!(build["env"].get("RUSTUP_TOOLCHAIN").is_none());
    assert!(!f.temp.path().join("record.rustup").exists());
    assert!(text.contains("GLIBC max 2.17"));
    fs::write(f.temp.path().join("untracked.txt"), "dirty").unwrap();
    success(f.run(X86, &[]));
    assert_eq!(
        f.records().last().unwrap()["env"]["NEXODE_BUILD_GIT_DIRTY"],
        "true"
    );
}
#[test]
fn arm_caller_nightly_target_flags_and_metadata_guard() {
    let f = Fixture::new();
    let text = success(f.run(
        ARM,
        &[
            ("RUSTFLAGS", ""),
            ("CARGO_ENCODED_RUSTFLAGS", ""),
            ("RUSTUP_TOOLCHAIN", "1.99.0"),
            (
                "CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_RUSTFLAGS",
                "-Cdebuginfo=1",
            ),
        ],
    ));
    let records = f.records();
    let b = records.last().unwrap();
    assert_eq!(b["args"][0], "build");
    assert!(
        b["args"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("-Zbuild-std=std,panic_abort"))
    );
    for key in ["JUSTRUST_TOOLCHAIN", "RUSTUP_TOOLCHAIN"] {
        assert_eq!(b["env"][key], "nightly-2026-10-08");
    }
    for key in ["JUSTRUST_REMOTE_REQUIRED", "JUSTRUST_TOOLCHAIN_REQUIRED"] {
        assert_eq!(b["env"][key], "1");
    }
    assert_eq!(
        b["env"]["CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_RUSTFLAGS"],
        "-Cdebuginfo=1 -Ztls-model=emulated"
    );
    for key in ["RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "RUSTC_BOOTSTRAP"] {
        assert!(b["env"].get(key).is_none());
    }
    for arg in ["--local", "--no-default-features"] {
        assert!(
            !b["args"]
                .as_array()
                .unwrap()
                .contains(&serde_json::json!(arg))
        );
    }
    assert_eq!(
        serde_json::from_slice::<Value>(&fs::read(f.temp.path().join("record.rustup")).unwrap())
            .unwrap(),
        serde_json::json!([
            "toolchain",
            "install",
            "nightly-2026-10-08",
            "--profile",
            "minimal",
            "--no-self-update"
        ])
    );
    assert!(text.contains("GLIBC max 2.39"));
    let out = f
        .command(f.temp.path().join("bin/cargo").to_str().unwrap())
        .args(["metadata", "--format-version", "1", "--offline"])
        .env("JUSTRUST_TOOLCHAIN", "nightly-2026-10-08")
        .env(
            "CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_RUSTFLAGS",
            "-Ztls-model=emulated",
        )
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("only accepted on the nightly compiler"));
}
#[test]
fn invalid_inputs_and_installer_failure_never_dispatch() {
    for (key, value) in [
        ("NEXODE_BUILD_SEMVER", ""),
        ("NEXODE_BUILD_SEMVER", "v1.0.0-preview.0"),
        ("NEXODE_BUILD_SEMVER", "v1.0.0-preview.01"),
        ("NEXODE_BUILD_SEMVER", "v1.0.0-preview.18446744073709551616"),
        ("NEXODE_BUILD_SEMVER", "v1.1.0-preview.2"),
        ("RUSTFLAGS", "-Ztls-model=emulated"),
        ("CARGO_ENCODED_RUSTFLAGS", "-Clinker=mold"),
        ("RUSTC_BOOTSTRAP", "1"),
        ("CARGO_TARGET_DIR", "other"),
        ("CARGO_BUILD_TARGET_DIR", "other"),
        ("JUSTRUST_DISABLE", "1"),
    ] {
        let f = Fixture::new();
        assert!(!f.run(X86, &[(key, value)]).status.success(), "{key}");
        assert!(f.records().is_empty());
    }
    let f = Fixture::new();
    assert!(!f.run("aarch64-unknown-linux-musl", &[]).status.success());
    assert!(f.records().is_empty());
    assert_eq!(
        f.run(ARM, &[("RUSTUP_FAILURE", "19")]).status.code(),
        Some(19)
    );
    assert!(f.records().is_empty());
}
#[test]
fn backend_build_missing_artifact_and_abi_fail_closed() {
    let f = Fixture::new();
    assert_eq!(
        f.run(X86, &[("BACKEND_FAILURE", "17")]).status.code(),
        Some(17)
    );
    assert_eq!(f.records().len(), 1);
    let f = Fixture::new();
    let out = f.run(X86, &[("NO_ARTIFACT", "1")]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("did not return an executable"));
    let f = Fixture::new();
    success(f.run(X86, &[]));
    let out = f.run(X86, &[("BUILD_FAILURE", "1")]);
    assert_eq!(out.status.code(), Some(23));
    assert!(!String::from_utf8_lossy(&out.stdout).contains("Validated hosted"));
    assert!(
        !f.records().last().unwrap()["args"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("--local"))
    );
    for (target, cases) in [
        (
            X86,
            vec![
                ("INVALID_ELF", "1"),
                ("MACHINE", "183"),
                ("GLIBC_VERSION", "2.18"),
                ("GLIBC_VERSION", ""),
                ("GLIBC_VERSION", "PRIVATE"),
                ("LOADER", "/lib/ld-musl-x86_64.so.1"),
                ("DYNAMIC_OPENSSL", "1"),
                ("READELF_FAILURE", "1"),
            ],
        ),
        (
            ARM,
            vec![
                ("NATIVE_TLS", "1"),
                ("NO_EMUTLS", "1"),
                ("GLIBC_VERSION", "2.40"),
                ("MACHINE", "62"),
                ("DYNAMIC_OPENSSL", "1"),
            ],
        ),
    ] {
        for (key, value) in cases {
            let f = Fixture::new();
            let out = f.run(target, &[(key, value)]);
            assert!(!out.status.success(), "{target} {key}");
            assert!(!String::from_utf8_lossy(&out.stdout).contains("Validated hosted"));
        }
    }
}
