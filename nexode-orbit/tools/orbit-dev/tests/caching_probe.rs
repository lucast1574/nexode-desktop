#![cfg(unix)]
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixListener,
    process::Command,
};
#[test]
fn four_turn_usage_is_reported_and_failed_turn_still_destroys_session() {
    for fail in [false, true] {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("debug");
        let listener = UnixListener::bind(&path).unwrap();
        let thread = std::thread::spawn(move || {
            let mut turns = 0;
            for connection in listener.incoming() {
                let mut stream = connection.unwrap();
                let mut line = String::new();
                BufReader::new(stream.try_clone().unwrap())
                    .read_line(&mut line)
                    .unwrap();
                let req: Value = serde_json::from_str(&line).unwrap();
                let cmd = req["command"].as_str().unwrap();
                let (output, ok) = if cmd.starts_with("create_session:") {
                    (json!({"session_id":"session_fixture"}).to_string(), true)
                } else if cmd.starts_with("destroy_session:") {
                    writeln!(
                        stream,
                        "{}",
                        json!({"type":"debug_response","ok":true,"output":"deleted"})
                    )
                    .unwrap();
                    assert_eq!(turns, if fail { 1 } else { 4 });
                    return;
                } else if cmd.starts_with("message:") {
                    assert_eq!(req["session_id"], "session_fixture");
                    turns += 1;
                    ("response".into(), !fail)
                } else {
                    assert_eq!(cmd, "usage");
                    assert_eq!(req["session_id"], "session_fixture");
                    (json!({"input_tokens":100,"output_tokens":10,"cache_read_input_tokens":900,"cache_creation_input_tokens":null}).to_string(),true)
                };
                // Fragmented responses and a leading ACK must not corrupt the report.
                stream.write_all(b"{\"type\":\"ack\"}\n").unwrap();
                let reply = format!(
                    "{}\n",
                    json!({"type":"debug_response","ok":ok,"output":output})
                );
                stream.write_all(&reply.as_bytes()[..7]).unwrap();
                stream.write_all(&reply.as_bytes()[7..]).unwrap();
            }
            panic!("session was not destroyed");
        });
        let out = Command::new(env!("CARGO_BIN_EXE_orbit-dev"))
            .args(["test-caching-detailed", "--debug-socket"])
            .arg(path)
            .output()
            .unwrap();
        assert_eq!(
            out.status.success(),
            !fail,
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        if !fail {
            let v: Value = serde_json::from_slice(&out.stdout).unwrap();
            assert_eq!(v["turns"].as_array().unwrap().len(), 4);
            assert_eq!(v["total"]["input_tokens"], 400);
            assert_eq!(v["total"]["cache_hit_rate"], 90.0);
        }
        thread.join().unwrap();
    }
}
