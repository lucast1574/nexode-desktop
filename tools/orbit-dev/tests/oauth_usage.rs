#![cfg(unix)]
mod support;
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixListener,
    process::Command,
    time::Duration,
};
#[test]
fn comparison_uses_three_quota_samples_and_cleans_the_session() {
    let d = tempfile::tempdir().unwrap();
    let http = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", http.server_addr());
    let quota = std::thread::spawn(move || {
        for usage in [10.0, 10.25, 10.5] {
            let request = http.recv_timeout(Duration::from_secs(15)).unwrap().unwrap();
            assert!(
                request
                    .headers()
                    .iter()
                    .any(|h| h.field.equiv("Authorization")
                        && h.value.as_str() == "Bearer fixture-token")
            );
            request
                .respond(tiny_http::Response::from_string(
                    json!({"five_hour":{"utilization":usage}}).to_string(),
                ))
                .unwrap();
        }
    });
    let socket = d.path().join("socket");
    let listener = UnixListener::bind(&socket).unwrap();
    let debug = std::thread::spawn(move || {
        for expected in [
            "create_session:",
            "state",
            "message:",
            "usage",
            "destroy_session:",
        ] {
            let (mut stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut line)
                .unwrap();
            let req: Value = serde_json::from_str(&line).unwrap();
            assert!(req["command"].as_str().unwrap().starts_with(expected));
            let output = match expected {
                "create_session:" => json!({"session_id":"session_fixture"}).to_string(),
                "state" => json!({"provider":"anthropic","model":"fixture"}).to_string(),
                "usage" => {
                    json!({"input_tokens":10,"output_tokens":1,"cache_read_input_tokens":null})
                        .to_string()
                }
                _ => "4".into(),
            };
            writeln!(
                stream,
                "{}",
                json!({"type":"debug_response","ok":true,"output":output})
            )
            .unwrap();
        }
    });
    let creds = d.path().join("credentials");
    fs::write(
        &creds,
        r#"{"claudeAiOauth":{"accessToken":"fixture-token"}}"#,
    )
    .unwrap();
    let cli = d.path().join("claude");
    support::executable(
        &cli,
        "#!/bin/sh\nprintf '%s\\n' '{\"result\":\"4\",\"usage\":{\"input_tokens\":20,\"output_tokens\":2}}'\n",
    );
    let out = Command::new(env!("CARGO_BIN_EXE_orbit-dev"))
        .args(["test-oauth-usage", "--debug-socket"])
        .arg(socket)
        .arg("--credentials")
        .arg(creds)
        .arg("--claude")
        .arg(cli)
        .arg("--usage-endpoint")
        .arg(endpoint)
        .output()
        .unwrap();
    let text = support::success(out);
    assert!(!text.contains("fixture-token"));
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["quota"]["cli_delta"], 0.25);
    assert_eq!(v["quota"]["nexode_delta"], 0.25);
    assert_eq!(v["comparison"]["token_savings_percent"], 50.0);
    quota.join().unwrap();
    debug.join().unwrap();
}
#[test]
fn credentials_are_not_read_for_an_untrusted_quota_endpoint() {
    let out = Command::new(env!("CARGO_BIN_EXE_orbit-dev"))
        .args([
            "test-oauth-usage",
            "--usage-endpoint",
            "https://example.com/usage",
            "--credentials",
            "/missing-fixture-credentials",
        ])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("loopback fixture"));
}
