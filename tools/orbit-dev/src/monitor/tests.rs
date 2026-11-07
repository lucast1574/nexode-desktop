use super::*;
#[test]
fn fragment_unicode_frames_bounded_state_and_terminal_controls() {
    let mut s = State::default();
    let mut f = Frames::default();
    let bytes = b"{\"type\":\"text_delta\",\"text\":\"\xc3\xa1\"}\n";
    for b in bytes {
        f.feed(&[*b], &mut s).unwrap();
    }
    assert_eq!(s.text, "á");
    assert_eq!(s.events, 1);
    f.feed(b"invalid\n{\"type\":\"ack\"}\n", &mut s).unwrap();
    assert!(s.processing);
    assert_eq!(s.events, 2);
    s.event(&json!({"type":"tokens","input":123,"output":456}));
    assert_eq!((s.input, s.output), (123, 456));
    for i in 0..30 {
        s.event(&json!({"type":"tool_start","id":i.to_string(),"name":"test"}));
        s.event(&json!({"type":"tool_done","id":i.to_string(),"name":"test","output":"ok"}));
        s.event(&json!({"type":"error","message":"fail"}));
    }
    assert!(s.tools.is_empty());
    assert_eq!(s.history.len(), 20);
    assert_eq!(s.errors.len(), 10);
    s.event(&json!({"type":"text_delta","text":"á".repeat(3000)}));
    assert_eq!(s.text.chars().count(), 2000);
    s.event(&json!({"type":"session","session_id":"\u{1b}[31mBAD"}));
    assert!(!s.render().contains("\u{1b}[31mBAD"));
    s.event(&json!({"type":"done"}));
    assert!(!s.processing);
    assert!(s.text.is_empty());
    assert!(f.feed(&vec![b'x'; 1024 * 1024 + 1], &mut s).is_err());
}
#[cfg(unix)]
#[test]
fn real_unix_connection_subscribes_handles_fragments_and_detects_eof() {
    use std::os::unix::net::UnixListener;
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("debug.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let mut id = 1;
    let mut c = Connection::connect(&path, &mut id).unwrap();
    let (mut peer, _) = listener.accept().unwrap();
    peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let mut s = State::default();
    c.tick(&mut s).unwrap();
    let mut reader = std::io::BufReader::new(peer.try_clone().unwrap());
    for kind in ["subscribe", "state"] {
        let mut line = String::new();
        std::io::BufRead::read_line(&mut reader, &mut line).unwrap();
        let v: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["type"], kind);
    }
    peer.write_all(b"{\"type\":\"state\",\"session_id\":")
        .unwrap();
    c.tick(&mut s).unwrap();
    assert_eq!(s.events, 0);
    peer.write_all(b"\"session_test\",\"is_processing\":true}\n")
        .unwrap();
    c.tick(&mut s).unwrap();
    assert_eq!(s.session, "session_test");
    assert!(s.processing);
    c.request("ping", &mut id);
    c.tick(&mut s).unwrap();
    let mut line = String::new();
    std::io::BufRead::read_line(&mut reader, &mut line).unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&line).unwrap()["type"],
        "ping"
    );
    drop(reader);
    drop(peer);
    assert!(c.tick(&mut s).is_err());
    let mut replacement = Connection::connect(&path, &mut id).unwrap();
    let (_peer, _) = listener.accept().unwrap();
    replacement.tick(&mut s).unwrap();
    assert!(replacement.frames.bytes.is_empty());
}
