use super::*;
use serde_json::json;
use std::{net::TcpListener, thread};
#[test]
#[allow(
    clippy::result_large_err,
    reason = "Tungstenite requires an unboxed HTTP response in handshake callbacks"
)]
fn websocket_auth_masking_multiline_ping_and_timeout() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let worker = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut ws = tungstenite::accept_hdr(
            stream,
            |request: &tungstenite::handshake::server::Request,
             response: tungstenite::handshake::server::Response| {
                assert_eq!(request.headers()["Authorization"], "Bearer fixture-secret");
                Ok(response)
            },
        )
        .unwrap();
        let v: Value = serde_json::from_str(ws.read().unwrap().to_text().unwrap()).unwrap();
        assert_eq!(v["type"], "subscribe");
        ws.send(Message::Ping(vec![1, 2, 3])).unwrap();
        assert_eq!(ws.read().unwrap(), Message::Pong(vec![1, 2, 3]));
        ws.send(Message::Text(
            "{\"type\":\"ack\"}\n{\"type\":\"done\",\"id\":1}\n".into(),
        ))
        .unwrap();
        thread::sleep(Duration::from_millis(150));
    });
    let mut g =
        Gateway::connect("127.0.0.1", port, "fixture-secret", Duration::from_secs(2)).unwrap();
    assert_eq!(g.send(json!({"type":"subscribe"})).unwrap(), 1);
    let deadline = Instant::now() + Duration::from_secs(2);
    assert_eq!(g.next(deadline).unwrap().unwrap()["type"], "ack");
    assert_eq!(g.next(deadline).unwrap().unwrap()["id"], 1);
    assert!(
        g.next(Instant::now() + Duration::from_millis(20))
            .unwrap()
            .is_none()
    );
    worker.join().unwrap();
}
#[test]
fn endpoints_reject_injection_and_support_ipv6() {
    assert!(endpoint("evil/route", 7643, "/ws").is_err());
    assert!(endpoint("user@host", 7643, "/ws").is_err());
    assert!(endpoint("127.0.0.1", 0, "/ws").is_err());
    assert_eq!(
        endpoint("::1", 7643, "/health").unwrap().as_str(),
        "http://[::1]:7643/health"
    );
}
