#![cfg(unix)]
mod support;
use serde_json::Value;
use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
};
use support::*;
const IDENTITY: &str = "Developer ID Application: Fixture (ABCDEFGHIJ)";
const SECRETS: &[&str] = &[
    "APPLE_CERTIFICATE_P12_BASE64",
    "APPLE_CERTIFICATE_PASSWORD",
    "APPLE_SIGNING_IDENTITY",
    "APPLE_NOTARY_KEY_P8_BASE64",
    "APPLE_NOTARY_KEY_ID",
    "APPLE_NOTARY_ISSUER",
];
struct Fixture {
    temp: tempfile::TempDir,
    tools: PathBuf,
    log: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let tools = temp.path().join("tools");
        fs::create_dir(&tools).unwrap();
        let log = temp.path().join("calls.jsonl");
        for name in ["uname", "security", "codesign", "ditto", "xcrun"] {
            std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_orbit-fixture"), tools.join(name))
                .unwrap();
        }
        fs::create_dir(temp.path().join("handoff")).unwrap();
        fs::write(temp.path().join("handoff/nexode"),"#!/bin/sh\n[ -z \"${APPLE_CERTIFICATE_PASSWORD:-}\" ] || exit 99\necho 'nexode Usage: fixture'\n").unwrap();
        fs::write(temp.path().join("handoff/provenance.json"), "{}").unwrap();
        Self { temp, tools, log }
    }
    fn run(&self, extra: &[(&str, &str)]) -> Output {
        let mut cmd = Command::new("bash");
        cmd.arg(root().join("scripts/sign_native_handoff_macos.sh"))
            .current_dir(self.temp.path())
            .env_clear()
            .env("PATH", format!("{}:/usr/bin:/bin", self.tools.display()))
            .env("HOME", self.temp.path())
            .env("TEST_LOG", &self.log)
            .env("GITHUB_REPOSITORY", "nexode-agent/unconfigured")
            .env("GITHUB_REF", "refs/heads/master")
            .env("ORBIT_DEV_BIN", env!("CARGO_BIN_EXE_orbit-dev"));
        for key in SECRETS {
            cmd.env(key, "eA==");
        }
        cmd.env("APPLE_SIGNING_IDENTITY", IDENTITY);
        for (k, v) in extra {
            cmd.env(k, v);
        }
        cmd.output().unwrap()
    }
    fn calls(&self) -> Vec<Value> {
        fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }
}
fn good(out: &Output) {
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
#[test]
fn native_script_checks_chain_team_smoke_and_restores_search_list() {
    let f = Fixture::new();
    let out = f.run(&[]);
    good(&out);
    let calls = f.calls();
    let sign = &calls
        .iter()
        .find(|c| {
            c[0] == "codesign"
                && c[1]
                    .as_array()
                    .unwrap()
                    .contains(&Value::String("--sign".into()))
        })
        .unwrap()[1];
    let args = sign.as_array().unwrap();
    let index = args.iter().position(|v| v == "--sign").unwrap();
    assert_eq!(args[index + 1], "A".repeat(40));
    let verify = &calls
        .iter()
        .find(|c| {
            c[0] == "codesign"
                && c[1]
                    .as_array()
                    .unwrap()
                    .contains(&Value::String("--verify".into()))
        })
        .unwrap()[1];
    let args = verify.as_array().unwrap();
    let index = args.iter().position(|v| v == "-R").unwrap();
    let requirement = args[index + 1].as_str().unwrap();
    for text in [
        "=anchor apple generic",
        "certificate 1[field.1.2.840.113635.100.6.2.6] exists",
        "certificate leaf[field.1.2.840.113635.100.6.1.13] exists",
        "identifier \"com.nexode.cli\"",
        "certificate leaf[subject.OU] = \"ABCDEFGHIJ\"",
    ] {
        assert!(requirement.contains(text));
    }
    assert_eq!(
        String::from_utf8_lossy(&out.stdout)
            .matches("nexode Usage: fixture")
            .count(),
        3
    );
    assert!(
        f.temp
            .path()
            .join("signed-handoff/signing-provenance.json")
            .is_file()
    );
    let security: Vec<_> = calls
        .iter()
        .filter(|c| c[0] == "security")
        .map(|c| &c[1])
        .collect();
    assert_eq!(
        security[0],
        &serde_json::json!(["list-keychains", "-d", "user"])
    );
    let updates: Vec<_> = security
        .iter()
        .filter(|a| a[0] == "list-keychains" && a[3] == "-s")
        .collect();
    assert_eq!(updates.len(), 2);
    assert!(
        updates[0][4]
            .as_str()
            .unwrap()
            .ends_with("/handoff.keychain-db")
    );
    let originals = serde_json::json!([
        "/Users/runner/Library/Keychains/login.keychain-db",
        "/Library/Keychains/Fixture With Spaces.keychain-db"
    ]);
    assert_eq!(
        &updates[0].as_array().unwrap()[5..],
        originals.as_array().unwrap()
    );
    assert_eq!(
        &updates[1].as_array().unwrap()[4..],
        originals.as_array().unwrap()
    );
    assert_eq!(security.last().unwrap()[0], "delete-keychain");
}
#[test]
fn all_failure_paths_restore_keychain_and_do_not_notarize() {
    for flag in [
        "TEST_BAD_SEARCH_LIST",
        "TEST_WRONG_IDENTITY",
        "TEST_DUPLICATE_IDENTITY",
        "TEST_BAD_CHAIN",
        "runtime",
    ] {
        let f = Fixture::new();
        if flag == "runtime" {
            fs::write(f.temp.path().join("handoff/nexode"), "#!/bin/sh\nexit 1\n").unwrap();
        }
        let out = f.run(&[(flag, "1")]);
        assert!(!out.status.success(), "{flag}");
        let calls = f.calls();
        assert!(!calls.iter().any(|c| c[0] == "xcrun"));
        if [
            "TEST_BAD_SEARCH_LIST",
            "TEST_WRONG_IDENTITY",
            "TEST_DUPLICATE_IDENTITY",
        ]
        .contains(&flag)
        {
            assert!(!calls.iter().any(|c| c[0] == "codesign"));
        }
        let security: Vec<_> = calls.iter().filter(|c| c[0] == "security").collect();
        assert_eq!(security[security.len() - 2][1][0], "list-keychains");
        assert_eq!(
            security[security.len() - 2][1][4],
            "/Users/runner/Library/Keychains/login.keychain-db"
        );
        assert_eq!(security.last().unwrap()[1][0], "delete-keychain");
    }
}
#[test]
fn missing_or_nonexact_credentials_fail_before_tools() {
    for identity in [
        "-",
        "Fixture",
        &"A".repeat(40),
        "Apple Development: Fixture (ABCDEFGHIJ)",
        "Developer ID Application: Fixture (BAD)",
        &format!("{IDENTITY}\n"),
    ] {
        let f = Fixture::new();
        let out = f.run(&[("APPLE_SIGNING_IDENTITY", identity)]);
        assert!(!out.status.success());
        assert!(
            String::from_utf8_lossy(&out.stderr).contains("exact Developer ID Application name")
        );
        assert!(f.calls().is_empty());
    }
    for missing in SECRETS {
        let f = Fixture::new();
        let out = f.run(&[(missing, "")]);
        assert!(!out.status.success());
        assert!(
            String::from_utf8_lossy(&out.stderr)
                .contains(&format!("Required signing credential missing: {missing}"))
        );
        assert!(f.calls().is_empty());
    }
    for (repo, branch, event) in [
        ("fork/nexode", "refs/heads/master", "workflow_dispatch"),
        (
            "nexode-agent/unconfigured",
            "refs/heads/topic",
            "workflow_dispatch",
        ),
        ("nexode-agent/unconfigured", "refs/heads/master", "push"),
    ] {
        let out = Command::new("bash")
            .arg(root().join("scripts/check_signing_helper_macos.sh"))
            .arg("sign")
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("GITHUB_REPOSITORY", repo)
            .env("GITHUB_REF", branch)
            .env("GITHUB_EVENT_NAME", event)
            .output()
            .unwrap();
        assert!(!out.status.success());
        assert!(
            String::from_utf8_lossy(&out.stderr).contains("Requires canonical master dispatch")
        );
    }
}
#[test]
fn workflow_trust_and_credential_boundaries_preserved() {
    let wf = read(".github/workflows/native-handoff.yml");
    for text in [
        "nightly-2026-10-08",
        "ref: a7e7d02cbd50358d1aea056d3c17e5496586d62a",
        "test \"$(git rev-parse HEAD)\" = a7e7d02cbd50358d1aea056d3c17e5496586d62a",
        "justrust build --remote --locked --release --bin nexode --target aarch64-unknown-linux-gnu -Zbuild-std=std,panic_abort --features linux-compat-vendored-openssl",
        "justrust build --local --locked --release --bin nexode --target aarch64-apple-darwin",
        "artifact-ids: ${{ needs.producer.outputs.artifact_id }}",
        "environment: native-handoff-signing",
        "github.ref == 'refs/heads/master'",
    ] {
        assert!(wf.contains(text), "{text}");
    }
    for text in [
        "\n  workflow_run:",
        "id-token: write",
        "contents: write",
        "gh release",
        "python",
    ] {
        assert!(!wf.contains(text));
    }
    let wf = read(".github/workflows/signing-helper-validation.yml");
    assert!(wf.contains("environment: native-handoff-signing"));
    assert_eq!(wf.matches("${{ secrets.").count(), 3);
    for text in [
        "inputs:",
        "download-artifact",
        "python",
        "signing-helper-source",
    ] {
        assert!(!wf.contains(text));
    }
    assert!(wf.contains("path: signing-helper-validation/receipt.json"));
    assert!(wf.find("export ORBIT_DEV_BIN=") < wf.find("APPLE_CERTIFICATE_PASSWORD:"));
    let script = read("scripts/check_signing_helper_macos.sh");
    assert!(!script.contains("python"));
    assert!(script.contains("-R \"=$requirement\""));
    assert!(script.find("keychains.pending") < script.find("security create-keychain"));
    assert!(script.find("--output \"$fixture/wrong-hash\"") < script.find("args=(--input"));
    assert!(script.contains("[[ ! -e \"$fixture/wrong-hash\" && ! -L \"$fixture/wrong-hash\" ]]"));
}
