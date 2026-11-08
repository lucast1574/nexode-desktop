mod support;
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    process::Command,
    time::Duration,
};
use support::*;
#[allow(
    clippy::result_large_err,
    reason = "Tungstenite requires an unboxed HTTP response in handshake callbacks"
)]
fn serve(listener: TcpListener) {
    for expected in ["/health", "/pair"] {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut first = String::new();
        reader.read_line(&mut first).unwrap();
        assert!(first.contains(expected));
        let mut size = 0;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if line == "\r\n" {
                break;
            }
            if let Some((k, v)) = line.split_once(':')
                && k.eq_ignore_ascii_case("content-length")
            {
                size = v.trim().parse().unwrap();
            }
        }
        let body = if expected == "/pair" {
            let mut bytes = vec![0; size];
            reader.read_exact(&mut bytes).unwrap();
            let v: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(v["code"], "123456");
            assert_eq!(v["device_id"], "nexode-remote-check");
            json!({"token":"fixture-secret","device_id":"fixture"})
        } else {
            json!({"status":"ok","gateway":true})
        }
        .to_string();
        write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
    }
    let (stream, _) = listener.accept().unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let mut ws = tungstenite::accept_hdr(
        stream,
        |req: &tungstenite::handshake::server::Request,
         res: tungstenite::handshake::server::Response| {
            assert_eq!(req.headers()["authorization"], "Bearer fixture-secret");
            Ok(res)
        },
    )
    .unwrap();
    for kind in ["subscribe", "get_history", "message"] {
        let v: Value = serde_json::from_str(ws.read().unwrap().to_text().unwrap()).unwrap();
        assert_eq!(v["type"], kind);
        let id = v["id"].as_u64().unwrap();
        let body = match kind {
            "subscribe" => {
                assert_eq!(v["working_dir"], "/fixture workspace");
                json!({"type":"done","id":id}).to_string()
            }
            "get_history" => format!(
                "{}\n{}",
                json!({"type":"history","messages":[],"available_models":["fixture"]}),
                json!({"type":"done","id":id})
            ),
            _ => format!(
                "{}\n{}\n{}",
                json!({"type":"text_delta","text":"fixture-secret Linux /fixture workspace"}),
                json!({"type":"tool_done","name":"shell"}),
                json!({"type":"done","id":id})
            ),
        };
        ws.send(tungstenite::Message::Text(body)).unwrap();
    }
    let _ = ws.read();
}
#[test]
fn actual_cli_health_pair_cache_upgrade_subscribe_history_turn_and_redaction() {
    let temp = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let worker = std::thread::spawn(move || serve(listener));
    let out = Command::new(binary("orbit-dev"))
        .args([
            "remote-check",
            "--host",
            "127.0.0.1",
            "--port",
            &port.to_string(),
            "--code",
            "123456",
            "--working-dir",
            "/fixture workspace",
            "--timeout",
            "3",
            "--token-dir",
        ])
        .arg(temp.path())
        .output()
        .unwrap();
    let text = success(out);
    assert!(text.contains("ALL CHECKS PASSED"));
    assert!(!text.contains("fixture-secret"));
    assert!(text.contains("[redacted]"));
    worker.join().unwrap();
    let cache = temp.path().join(format!("127.0.0.1_{port}.json"));
    let v: Value = serde_json::from_slice(&std::fs::read(&cache).unwrap()).unwrap();
    assert_eq!(v["token"], "fixture-secret");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(cache).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
