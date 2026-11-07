#![cfg(unix)]
mod support;
use std::{
    fs,
    process::{Command, Output},
};
use support::*;
const REVISION: &str = "5c8c6e653d1874923cf94b93dd96aa3d312343f0";
struct Fixture {
    temp: tempfile::TempDir,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let r = temp.path();
        executable(
            r.join("bin/git"),
            r#"#!/bin/sh
printf '%s\n' "$*" >> "$BOOTSTRAP_LOG"
case "$*" in
 *"rev-parse HEAD"*) printf '%s\n' "${RETURNED_REVISION}" ;;
esac
"#,
        );
        executable(
            r.join("bin/cargo"),
            r#"#!/bin/sh
printf '%s\n' "$*" >> "$BOOTSTRAP_LOG"
[ "${CARGO_STATUS:-0}" = 0 ] || exit "$CARGO_STATUS"
while [ "$#" -gt 0 ]; do
 if [ "$1" = --root ]; then shift; root=$1; fi
 shift
done
mkdir -p "$root/bin"
printf '#!/bin/sh\n[ "$1" = --version ] && exit 0\nexit "${CAPABILITY_STATUS:-0}"\n' > "$root/bin/justrust"
chmod +x "$root/bin/justrust"
"#,
        );
        Self { temp }
    }
    fn run(&self, revision: &str, cargo: &str, capability: &str) -> Output {
        let r = self.temp.path();
        Command::new("bash")
            .arg(root().join("scripts/install_justrust_hosted_ci.sh"))
            .env("PATH", path(&r.join("bin")))
            .env("RUNNER_TEMP", r)
            .env("GITHUB_PATH", r.join("path"))
            .env("BOOTSTRAP_LOG", r.join("calls"))
            .env("RETURNED_REVISION", revision)
            .env("CARGO_STATUS", cargo)
            .env("CAPABILITY_STATUS", capability)
            .output()
            .unwrap()
    }
}
#[test]
fn exact_source_pin_locked_compiler_and_embedded_helper() {
    let f = Fixture::new();
    success(f.run(REVISION, "0", "0"));
    let calls = fs::read_to_string(f.temp.path().join("calls")).unwrap();
    for s in [
        format!("--depth=1 --filter=blob:none origin {REVISION}"),
        "/scripts/sign-release.py".into(),
        "+1.99.0 install --path".into(),
        "--locked --root".into(),
    ] {
        assert!(calls.contains(&s), "{calls}");
    }
    assert!(!calls.contains("toolchain/"));
    assert!(
        fs::read_to_string(f.temp.path().join("path"))
            .unwrap()
            .trim()
            .ends_with("/install/bin")
    );
}
#[test]
fn wrong_revision_never_compiles_or_activates() {
    let f = Fixture::new();
    assert!(!f.run(&"0".repeat(40), "0", "0").status.success());
    assert!(
        !fs::read_to_string(f.temp.path().join("calls"))
            .unwrap()
            .contains("install --path")
    );
    assert!(!f.temp.path().join("path").exists());
}
#[test]
fn compiler_failure_never_activates() {
    let f = Fixture::new();
    assert_eq!(f.run(REVISION, "42", "0").status.code(), Some(42));
    assert!(!f.temp.path().join("path").exists());
}
#[test]
fn missing_capability_never_activates() {
    let f = Fixture::new();
    assert!(!f.run(REVISION, "0", "2").status.success());
    assert!(!f.temp.path().join("path").exists());
}
