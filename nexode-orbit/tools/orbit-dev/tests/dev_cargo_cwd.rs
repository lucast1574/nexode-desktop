#![cfg(unix)]
mod support;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
use support::*;
struct Fixture {
    temp: tempfile::TempDir,
    project: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("other rust project");
        write(
            project.join("Cargo.toml"),
            "[package]\nname = \"cwd_probe\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        );
        write(project.join("src/lib.rs"), "");
        Self { temp, project }
    }
    fn metadata(&self, cwd: &Path, child: bool) -> serde_json::Value {
        let wrapper = root().join("scripts/dev_cargo.sh");
        let mut command = if child {
            let mut c = Command::new("bash");
            c.args([
                "-c",
                r#"
 cargo() {
  if [[ "${NEXODE_IN_DEV_CARGO:-0}" == "1" ]]; then command cargo "$@"
  else NEXODE_IN_DEV_CARGO=1 "$NEXODE_DEV_CARGO_SCRIPT" "$@"; fi
 }
 export -f cargo
 cd "$1"
 bash -c 'cargo metadata --no-deps --format-version 1'
 "#,
                "probe",
            ])
            .arg(cwd)
            .current_dir(root());
            c
        } else {
            let mut c = Command::new(&wrapper);
            c.args(["metadata", "--no-deps", "--format-version", "1"])
                .current_dir(cwd);
            c
        };
        command
            .env_remove("CARGO_MANIFEST_DIR")
            .env("NEXODE_RUST_ACTION_LOG", "0")
            .env("NEXODE_DEV_CARGO_SCRIPT", wrapper);
        let out = success(command.output().unwrap());
        serde_json::from_str(&out).unwrap()
    }
}
#[test]
fn foreign_project_keeps_its_workspace() {
    let f = Fixture::new();
    let m = f.metadata(&f.project, false);
    assert_eq!(Path::new(m["workspace_root"].as_str().unwrap()), f.project);
    assert_eq!(m["packages"][0]["name"], "cwd_probe");
}
#[test]
fn exported_shim_after_cd_in_child_shell() {
    let f = Fixture::new();
    assert_eq!(
        Path::new(
            f.metadata(&f.project, true)["workspace_root"]
                .as_str()
                .unwrap()
        ),
        f.project
    );
}
#[test]
fn nexode_root_and_member_keep_wrapper_policy() {
    let f = Fixture::new();
    for cwd in [root(), root().join("crates/nexode-tui")] {
        assert_eq!(
            Path::new(f.metadata(&cwd, false)["workspace_root"].as_str().unwrap()),
            fs::canonicalize(root()).unwrap()
        );
    }
}
#[test]
fn symlink_to_foreign_project_keeps_its_workspace() {
    let f = Fixture::new();
    let link = f.temp.path().join("linked project");
    std::os::unix::fs::symlink(&f.project, &link).unwrap();
    assert_eq!(
        Path::new(f.metadata(&link, false)["workspace_root"].as_str().unwrap()),
        f.project
    );
}
#[test]
fn foreign_cargo_errors_are_preserved() {
    let f = Fixture::new();
    let out = Command::new(root().join("scripts/dev_cargo.sh"))
        .args(["metadata", "--manifest-path", "missing.toml"])
        .current_dir(&f.project)
        .env_remove("CARGO_MANIFEST_DIR")
        .env("NEXODE_RUST_ACTION_LOG", "0")
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("missing.toml"));
}
