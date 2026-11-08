#![cfg(unix)]
mod support;
use serde_json::{Value, json};
use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
};
use support::*;
const TAG: &str = "v1.2.3";
const JOBS: &[&str] = &[
    "Quality Guardrails",
    "Build & Test (ubuntu-latest)",
    "Build & Test (macos-latest)",
    "Build & Test (windows-latest)",
    "Windows ARM64 Release Configuration",
];
struct Fixture {
    temp: tempfile::TempDir,
    repo: PathBuf,
    origin: PathBuf,
    sha: String,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("source");
        let origin = temp.path().join("origin.git");
        fs::create_dir_all(repo.join("scripts")).unwrap();
        for name in ["quick-release.sh", "orbit_dev.sh"] {
            fs::copy(
                root().join("scripts").join(name),
                repo.join("scripts").join(name),
            )
            .unwrap();
        }
        fs::write(
            repo.join("Cargo.toml"),
            "[package]\nname = \"release-fixture\"\nversion = \"1.2.3\"\n",
        )
        .unwrap();
        let mut f = Self {
            temp,
            repo,
            origin,
            sha: String::new(),
        };
        for args in [
            vec!["init", "-b", "master"],
            vec!["config", "user.name", "Release Fixture"],
            vec!["config", "user.email", "fixture@example.invalid"],
            vec!["config", "commit.gpgsign", "false"],
            vec!["config", "tag.gpgsign", "false"],
            vec!["add", "scripts", "Cargo.toml"],
            vec!["commit", "-m", "Initial fixture"],
            vec!["commit", "--allow-empty", "-m", "Release metadata"],
        ] {
            f.git(&args);
        }
        f.sha = f.git(&["rev-parse", "HEAD"]);
        assert!(
            Command::new("git")
                .args(["init", "--bare"])
                .arg(&f.origin)
                .output()
                .unwrap()
                .status
                .success()
        );
        f.git(&["remote", "add", "origin", f.origin.to_str().unwrap()]);
        f.git(&["push", "origin", "master"]);
        fs::create_dir_all(f.temp.path().join("bin")).unwrap();
        std::os::unix::fs::symlink(
            env!("CARGO_BIN_EXE_orbit-fixture"),
            f.temp.path().join("bin/gh"),
        )
        .unwrap();
        f
    }
    fn git_raw(&self, args: &[&str]) -> Output {
        Command::new("git")
            .args(args)
            .current_dir(&self.repo)
            .output()
            .unwrap()
    }
    fn git(&self, args: &[&str]) -> String {
        let out = self.git_raw(args);
        assert!(
            out.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap().trim().into()
    }
    fn release(&self, scenario: &str, options: &[&str]) -> Output {
        let base = self.temp.path();
        let marker = base.join("confirmed");
        let _ = fs::remove_file(&marker);
        fs::write(
            base.join("ci.json"),
            json!({"scenario":scenario,"sha":self.sha,"jobs":JOBS}).to_string(),
        )
        .unwrap();
        Command::new("bash")
            .arg("scripts/quick-release.sh")
            .args(options)
            .arg(TAG)
            .current_dir(&self.repo)
            .env("PATH", path(&base.join("bin")))
            .env("HOME", base)
            .env("ORBIT_DEV_BIN", env!("CARGO_BIN_EXE_orbit-dev"))
            .env("TEST_CI_CONFIG", base.join("ci.json"))
            .env("TEST_GH_LOG", base.join("gh.jsonl"))
            .env("TEST_CONFIRMATION_MARKER", marker)
            .output()
            .unwrap()
    }
    fn no_tag(&self) {
        assert!(
            !self
                .git_raw(&["show-ref", "--verify", "refs/tags/v1.2.3"])
                .status
                .success()
        );
        assert!(
            self.git(&["ls-remote", "--tags", "origin", "refs/tags/v1.2.3"])
                .is_empty()
        );
    }
    fn calls(&self) -> Vec<Value> {
        fs::read_to_string(self.temp.path().join("gh.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }
    fn prepare_standard(&self) {
        for (program, body) in [
            (
                "cargo",
                "mkdir -p target/aarch64-apple-darwin/release\nprintf fixture > target/aarch64-apple-darwin/release/nexode",
            ),
            ("docker", "exit 0"),
            ("rcodesign", "exit 0"),
            ("file", "echo 'ELF 64-bit Mach-O 64-bit'"),
            ("aarch64-apple-darwin23.5-clang", "exit 0"),
        ] {
            executable(
                self.temp.path().join("bin").join(program),
                &format!("#!/bin/sh\n{body}\n"),
            );
        }
        executable(
            self.repo.join("scripts/build_linux_compat.sh"),
            r#"#!/bin/sh
set -eu
printf '#!/bin/sh\n' > "$1/nexode-linux-x86_64"
printf fixture > "$1/nexode-linux-x86_64.bin"
(cd "$1" && tar czf nexode-linux-x86_64.tar.gz nexode-linux-x86_64 nexode-linux-x86_64.bin)
"#,
        );
        fs::write(self.repo.join(".gitignore"), "/dist/\n/target/\n").unwrap();
        self.git(&["add", "scripts/build_linux_compat.sh", ".gitignore"]);
        self.git(&["commit", "--amend", "--no-edit"]);
    }
}
fn success(out: &Output) {
    assert!(
        out.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}
#[test]
fn real_git_tag_success_idempotence_and_branch_collision() {
    let f = Fixture::new();
    f.git(&["branch", TAG, "HEAD^"]);
    let out = f.release("success", &["--remote"]);
    success(&out);
    assert_eq!(f.git(&["rev-parse", "refs/tags/v1.2.3^{commit}"]), f.sha);
    assert_eq!(f.git(&["cat-file", "-t", "refs/tags/v1.2.3"]), "tag");
    assert!(
        f.git(&["ls-remote", "--tags", "origin", "refs/tags/v1.2.3^{}"])
            .contains(&f.sha)
    );
    assert!(
        f.git(&["ls-remote", "--heads", "origin", "refs/heads/v1.2.3"])
            .is_empty()
    );
    let calls = f.calls();
    assert_eq!(
        calls[0],
        json!([
            "repo",
            "view",
            f.origin.to_str().unwrap(),
            "--json",
            "nameWithOwner",
            "--jq",
            ".nameWithOwner"
        ])
    );
    assert!(
        calls[1][1]
            .as_str()
            .unwrap()
            .contains(&format!("head_sha={}&event=push", f.sha))
    );
    assert!(calls[3][1].as_str().unwrap().contains("/attempts/1/jobs?"));
    let out = f.release("success", &["--remote"]);
    success(&out);
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("already exists at the CI-verified commit")
    );
    assert!(!f.release("failure", &["--remote"]).status.success());
}
#[test]
fn incorrect_ci_and_races_never_push() {
    for scenario in [
        "missing",
        "pending",
        "failure",
        "other_sha",
        "pull_request",
        "missing_job",
        "failed_job",
        "api_error",
        "repo_error",
        "rerun",
        "head_changed",
    ] {
        let f = Fixture::new();
        let out = f.release(scenario, &["--remote"]);
        assert!(!out.status.success(), "{scenario}");
        f.no_tag();
    }
    let f = Fixture::new();
    f.git(&["tag", "-a", TAG, "-m", TAG]);
    assert!(
        !f.release("local_tag_changed", &["--remote"])
            .status
            .success()
    );
    assert!(
        f.git(&["ls-remote", "--tags", "origin", "refs/tags/v1.2.3"])
            .is_empty()
    );
}
#[test]
fn metadata_conflicts_lookup_and_dry_run_fail_before_writes() {
    let f = Fixture::new();
    fs::write(f.repo.join("Cargo.toml"), "[package]\nversion='9.9.9'\n").unwrap();
    success(&f.release("success", &["--remote"]));
    let f = Fixture::new();
    fs::write(f.repo.join("Cargo.toml"), "[package]\nversion='9.9.9'\n").unwrap();
    f.git(&["add", "Cargo.toml"]);
    f.git(&["commit", "-m", "wrong version"]);
    assert!(!f.release("success", &["--remote"]).status.success());
    f.no_tag();
    assert!(f.calls().is_empty());
    let f = Fixture::new();
    f.git(&["tag", "-a", TAG, "HEAD^", "-m", TAG]);
    assert!(!f.release("success", &["--remote"]).status.success());
    f.git(&["push", "origin", "refs/tags/v1.2.3"]);
    f.git(&["tag", "-d", TAG]);
    assert!(!f.release("success", &["--remote"]).status.success());
    let f = Fixture::new();
    f.git(&[
        "remote",
        "set-url",
        "origin",
        f.temp.path().join("missing.git").to_str().unwrap(),
    ]);
    assert!(!f.release("success", &["--remote"]).status.success());
    assert!(
        !f.git_raw(&["show-ref", "--verify", "refs/tags/v1.2.3"])
            .status
            .success()
    );
    assert!(f.calls().is_empty());
    let f = Fixture::new();
    assert!(
        !f.release("success", &["--remote", "--dry-run"])
            .status
            .success()
    );
    f.no_tag();
    assert!(f.calls().is_empty());
}
#[test]
fn local_staging_guards_and_shared_gate() {
    let script = read("scripts/quick-release.sh");
    assert!(!script.contains("--draft=false"));
    assert_eq!(script.matches("    tag_and_push\n").count(), 3);
    assert!(script.contains("\ntag_and_push\n"));
    for scenario in ["success", "dirty", "dirty_during_ci", "public_release"] {
        let mut f = Fixture::new();
        f.prepare_standard();
        f.sha = f.git(&["rev-parse", "HEAD"]);
        if scenario == "dirty" {
            fs::write(f.repo.join("Cargo.toml"), "# Local edit\n").unwrap();
        }
        let out = f.release(scenario, &[]);
        if scenario == "success" {
            success(&out);
            assert!(
                f.calls()
                    .iter()
                    .any(|c| c[0] == "release" && c[1] == "upload")
            );
        } else {
            assert!(!out.status.success(), "{scenario}");
            if scenario != "public_release" {
                f.no_tag();
            }
            assert!(
                !f.calls()
                    .iter()
                    .any(|c| c[0] == "release" && (c[1] == "upload" || c[1] == "edit"))
            );
        }
        assert!(
            !f.calls()
                .iter()
                .any(|c| c.as_array().unwrap().contains(&json!("--draft=false")))
        );
    }
}
