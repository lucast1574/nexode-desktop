#![cfg(unix)]
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixListener,
    process::Command,
    time::{Duration, Instant},
};
fn run(fail_link: bool) {
    let root = tempfile::tempdir().unwrap();
    let socket = root.path().join("debug");
    let listener = UnixListener::bind(&socket).unwrap();
    listener.set_nonblocking(true).unwrap();
    let worker = std::thread::spawn(move || {
        let mut memories = BTreeMap::new();
        let mut messages = 0;
        let mut id = 0;
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let (mut s, _) = match listener.accept() {
                Ok(v) => v,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline);
                    std::thread::sleep(Duration::from_millis(5));
                    continue;
                }
                Err(e) => panic!("{e}"),
            };
            s.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
            let mut line = String::new();
            BufReader::new(s.try_clone().unwrap())
                .read_line(&mut line)
                .unwrap();
            let v: Value = serde_json::from_str(&line).unwrap();
            let cmd = v["command"].as_str().unwrap();
            let mut ok = true;
            let output = if cmd.starts_with("create_session:") {
                json!({"session_id":"session_probe"}).to_string()
            } else if cmd.starts_with("set_provider:") {
                "{}".into()
            } else if let Some(input) = cmd.strip_prefix("tool:memory ") {
                let p: Value = serde_json::from_str(input).unwrap();
                assert_eq!(p["scope"], "project");
                let result = match p["action"].as_str().unwrap() {
                    "remember" => {
                        id += 1;
                        let key = format!("mem_{id}");
                        memories.insert(key.clone(), p["content"].as_str().unwrap().to_owned());
                        format!("Remembered [id: {key}]")
                    }
                    "list" | "search" | "recall" | "related" => {
                        memories.values().cloned().collect::<Vec<_>>().join("\n")
                    }
                    "tag" => "Tagged".into(),
                    "link" => {
                        ok = !fail_link;
                        "Linked".into()
                    }
                    "forget" => {
                        assert!(memories.remove(p["id"].as_str().unwrap()).is_some());
                        "Forgot".into()
                    }
                    other => panic!("unexpected action {other}"),
                };
                json!({"output":result}).to_string()
            } else if cmd.starts_with("message:") {
                messages += 1;
                "ok".into()
            } else if cmd == "trigger_extraction" {
                assert_eq!(messages, 4);
                json!({"extracted":1,"message_count":8}).to_string()
            } else if cmd == "destroy_session:session_probe" {
                assert!(memories.is_empty());
                writeln!(
                    s,
                    "{}",
                    json!({"type":"debug_response","ok":true,"output":"ok"})
                )
                .unwrap();
                return messages;
            } else {
                panic!("unexpected command {cmd}")
            };
            writeln!(s, "{}", json!({"type":"ack"})).unwrap();
            writeln!(
                s,
                "{}",
                json!({"type":"debug_response","ok":ok,"output":output})
            )
            .unwrap();
        }
    });
    let output = Command::new(env!("CARGO_BIN_EXE_orbit-dev"))
        .args(["test-memory", "--socket"])
        .arg(socket)
        .args(["--provider", "claude", "--live"])
        .output()
        .unwrap();
    assert_eq!(
        output.status.success(),
        !fail_link,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(worker.join().unwrap(), if fail_link { 0 } else { 4 });
}
#[test]
fn live_contract_and_owned_memory_cleanup() {
    run(false);
}
#[test]
fn tool_failure_is_failure_and_still_cleans_owned_memories() {
    run(true);
}
