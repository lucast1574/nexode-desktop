#![cfg(target_os = "linux")]
use serde_json::Value;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    process::Command,
    time::{Duration, Instant},
};
fn script(root: &std::path::Path, name: &str, body: &str) -> std::path::PathBuf {
    let p = root.join(name);
    fs::write(
        &p,
        format!(
            "#!/bin/sh\nif [ \"${{1:-}}\" = --version ]; then echo fixture-v1; exit 0; fi\n{body}\n"
        ),
    )
    .unwrap();
    fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
    p
}
fn tool() -> Command {
    Command::new(env!("CARGO_BIN_EXE_orbit-dev"))
}
#[test]
fn stdio_protocols_concurrent_turns_and_memory_samples() {
    let r = tempfile::tempdir().unwrap();
    let claude = script(
        r.path(),
        "claude",
        r#"while IFS= read -r line; do echo '{"type":"assistant","message":{"content":[{"type":"tool_use"}]}}'; echo '{"type":"result","result":"done","is_error":false}'; done"#,
    );
    let pi = script(
        r.path(),
        "pi",
        r#"while IFS= read -r line; do echo '{"type":"tool_execution_start"}'; echo '{"type":"agent_end","messages":[{"text":"done"}]}'; done"#,
    );
    let codex = script(
        r.path(),
        "codex",
        r#"read -r line; echo '{"id":1,"result":{}}'; read -r line; read -r line; echo '{"id":2,"result":{"thread":{"id":"fixture-thread"}}}'; while IFS= read -r line; do echo '{"method":"item/completed","params":{"item":{"type":"commandExecution"}}}'; echo '{"method":"item/completed","params":{"item":{"type":"agentMessage","text":"done"}}}'; echo '{"method":"turn/completed","params":{"turn":{"status":"completed"}}}'; done"#,
    );
    let output = r.path().join("results.json");
    let o = tool()
        .args([
            "bench-headless-memory",
            "--sessions",
            "2",
            "--turns",
            "2",
            "--tools",
            "claude",
            "codex",
            "pi",
            "--settle",
            "0",
            "--timeout",
            "3",
            "--claude-bin",
        ])
        .arg(claude)
        .arg("--codex-bin")
        .arg(codex)
        .arg("--pi-bin")
        .arg(pi)
        .arg("--cwd")
        .arg(r.path())
        .arg("--json-out")
        .arg(&output)
        .env("OPENAI_API_KEY", "synthetic-memory-key")
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let v: Value = serde_json::from_slice(&fs::read(output).unwrap()).unwrap();
    assert_eq!(v.as_array().unwrap().len(), 3);
    for result in v.as_array().unwrap() {
        assert_eq!(result["checks"]["tool_uses"], 2);
        assert_eq!(result["samples"].as_array().unwrap().len(), 3);
        assert!(result["samples"][0][1].as_f64().unwrap() > 0.);
        assert_eq!(result["errors"], serde_json::json!([]));
    }
}
#[test]
fn silent_stdio_is_bounded_and_fails() {
    let r = tempfile::tempdir().unwrap();
    let binary = script(r.path(), "silent", "sleep 30");
    let start = Instant::now();
    let o = tool()
        .args([
            "bench-headless-memory",
            "--sessions",
            "1",
            "--turns",
            "1",
            "--tools",
            "claude",
            "--settle",
            "0",
            "--timeout",
            "0.2",
            "--claude-bin",
        ])
        .arg(binary)
        .output()
        .unwrap();
    assert!(!o.status.success());
    assert!(start.elapsed() < Duration::from_secs(5));
    assert!(String::from_utf8_lossy(&o.stdout).contains("timed out"));
}
#[test]
fn interactive_native_pty_requires_real_input_echo() {
    let r = tempfile::tempdir().unwrap();
    script(
        r.path(),
        "claude",
        r#"printf 'fixture ready\n'; while :; do byte=$(dd bs=1 count=1 2>/dev/null) || exit; printf '%s' "$byte"; done"#,
    );
    let output = r.path().join("interactive.json");
    let o = tool()
        .args([
            "bench-memory-cli",
            "--sessions",
            "2",
            "--tools",
            "claude_code",
            "--settle",
            "0",
            "--timeout",
            "3",
            "--json-out",
        ])
        .arg(&output)
        .env("PATH", format!("{}:/usr/bin:/bin", r.path().display()))
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let v: Value = serde_json::from_slice(&fs::read(output).unwrap()).unwrap();
    assert_eq!(v[0]["ready"], true);
    assert!(v[0]["pss_mb"].as_f64().unwrap() > 0.);
}

#[test]
fn opencode_fixture_server() {
    let Ok(port) = std::env::var("ORBIT_MEMORY_FIXTURE_PORT") else {
        return;
    };
    let server = tiny_http::Server::http(format!("127.0.0.1:{port}")).unwrap();
    let mut next = 0;
    for request in server.incoming_requests() {
        let body = match (request.method().as_str(), request.url()) {
            ("GET", "/session") => serde_json::json!([]),
            ("POST", "/session") => {
                next += 1;
                serde_json::json!({"id":format!("session-{next}")})
            }
            ("POST", _) => serde_json::json!({"parts":[{"type":"text","text":"done"}]}),
            _ => serde_json::json!([{"parts":[{"type":"tool"},{"type":"tool"}]}]),
        };
        request
            .respond(
                tiny_http::Response::from_string(body.to_string()).with_header(
                    tiny_http::Header::from_bytes("Content-Type", "application/json").unwrap(),
                ),
            )
            .unwrap();
    }
}
#[test]
fn opencode_http_sessions_and_tool_history() {
    let root = tempfile::tempdir().unwrap();
    let test_binary = std::env::current_exe().unwrap();
    let binary = script(
        root.path(),
        "opencode",
        &format!(
            "while [ $# -gt 0 ]; do if [ \"$1\" = --port ]; then shift; export ORBIT_MEMORY_FIXTURE_PORT=$1; fi; shift; done\nexec {} --exact opencode_fixture_server --nocapture",
            shlex::try_quote(&test_binary.to_string_lossy()).unwrap()
        ),
    );
    let out = tool()
        .args([
            "bench-headless-memory",
            "--sessions",
            "2",
            "--turns",
            "2",
            "--tools",
            "opencode",
            "--settle",
            "0",
            "--timeout",
            "3",
            "--opencode-bin",
        ])
        .arg(binary)
        .env("OPENAI_API_KEY", "synthetic-http-key")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value["checks"]["tool_uses"], 2);
    assert_eq!(value["checks"]["last_response"], "done");
}

#[test]
fn real_nexode_memory_with_loopback_provider() {
    let Ok(binary) = std::env::var("ORBIT_TEST_NEXODE_BINARY") else {
        eprintln!("SKIP real runtime memory test: set ORBIT_TEST_NEXODE_BINARY");
        return;
    };
    let root = tempfile::tempdir().unwrap();
    struct Provider(std::process::Child);
    impl Drop for Provider {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let port_file = root.path().join("port");
    let _provider = Provider(
        tool()
            .args(["fake-openai-server", "0"])
            .env("FAKE_LOG", root.path().join("provider.log"))
            .env("FAKE_PORT_FILE", &port_file)
            .spawn()
            .unwrap(),
    );
    let until = Instant::now() + Duration::from_secs(5);
    while !port_file.is_file() {
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(20));
    }
    let port = fs::read_to_string(port_file).unwrap();
    fs::write(
        root.path().join("config.toml"),
        "[provider]\ndefault_provider = 'openai-compatible'\ndefault_model = 'fake-model'\n",
    )
    .unwrap();
    let output = tool()
        .args([
            "bench-headless-memory",
            "--sessions",
            "2",
            "--turns",
            "2",
            "--tools",
            "nexode_memory_off",
            "--settle",
            "0.1",
            "--nexode-bin",
        ])
        .arg(binary)
        .args([
            "--nexode-model",
            "openai-compatible/fake-model",
            "--timeout",
            "20",
        ])
        .env("NEXODE_HOME", root.path())
        .env(
            "NEXODE_OPENAI_COMPAT_API_BASE",
            format!("http://127.0.0.1:{port}/v1"),
        )
        .env("OPENAI_COMPAT_API_KEY", "synthetic-memory-fixture")
        .env("OPENAI_API_KEY", "synthetic-memory-fixture")
        .env("ANTHROPIC_API_KEY", "synthetic-memory-fixture")
        .env("OPENAI_BASE_URL", "http://127.0.0.1:9/v1")
        .env("ANTHROPIC_BASE_URL", "http://127.0.0.1:9")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let v: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(v["samples"][0][1].as_f64().unwrap() > 0.);
    assert_eq!(v["sessions"], 2);
    assert_eq!(v["errors"], serde_json::json!([]));
    assert_eq!(v["samples"].as_array().unwrap().len(), 3);
    assert!(v["checks"]["history_messages"].as_u64().unwrap() >= 4);
}
