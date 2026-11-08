#![cfg(unix)]
mod support;
use serde_json::Value;
use std::{
    fs,
    io::{BufRead, BufReader},
    os::unix::fs::PermissionsExt,
    process::{Child, Command, Stdio},
    time::Duration,
};
struct Guard(Child);
impl Drop for Guard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[test]
fn real_http_auth_and_native_command_output_preserve_pairing_contract() {
    let d = tempfile::tempdir().unwrap();
    let token = d.path().join("token");
    fs::write(&token, "fixture-private-token").unwrap();
    fs::set_permissions(&token, fs::Permissions::from_mode(0o600)).unwrap();
    let cli = d.path().join("nexode");
    support::executable(
        &cli,
        r#"#!/bin/sh
[ "$1" = pair ] || exit 2
printf '\033[32mPairing code: 123 456\n'
"#,
    );
    let mut child = Guard(
        Command::new(env!("CARGO_BIN_EXE_orbit-dev"))
            .args([
                "pair-service",
                "--host",
                "gateway.example",
                "--bind",
                "127.0.0.1:0",
                "--token-file",
            ])
            .arg(&token)
            .arg("--binary")
            .arg(&cli)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let mut line = String::new();
    BufReader::new(child.0.stderr.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let endpoint = format!(
        "http://{}",
        line.trim()
            .strip_prefix("pair service listening on ")
            .unwrap()
    );
    let http = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap();
    let denied = http.get(format!("{endpoint}/pair-code")).send().unwrap();
    assert_eq!(denied.status(), 403);
    let ok = http
        .get(format!("{endpoint}/pair-code"))
        .bearer_auth("fixture-private-token")
        .send()
        .unwrap();
    assert_eq!(ok.status(), 200);
    assert_eq!(ok.headers()["cache-control"], "no-store");
    let v: Value = ok.json().unwrap();
    assert_eq!(v["code"], "123456");
    assert_eq!(v["host"], "gateway.example");
    assert_eq!(v["port"], 7643);
    assert_eq!(v["expires_in"], 300);
    assert_eq!(
        http.get(format!("{endpoint}/pair-code?t=fixture-private-token"))
            .send()
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        http.get(format!("{endpoint}/missing"))
            .bearer_auth("fixture-private-token")
            .send()
            .unwrap()
            .status(),
        404
    );
}
