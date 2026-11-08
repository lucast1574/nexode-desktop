use super::*;
#[test]
fn composer_anchors_prompt_and_removes_status_without_header_false_positive() {
    assert_eq!(
        composer(&[
            "path/to/project".into(),
            " 1> hello  3.1k/1.0M ▱▱ 2%".into()
        ]),
        Some("hello".into())
    );
    assert_eq!(composer(&["/some/path".into()]), None);
    let mut p = vt100::Parser::new(NonZeroU16::new(3).unwrap(), NonZeroU16::new(40).unwrap(), 0);
    p.process(b"\x1b[2J\x1b[H1> h\xc3");
    p.process(b"\xa9");
    assert_eq!(
        composer(&p.screen().rows(0, 40).collect::<Vec<_>>()),
        Some("hé".into())
    );
    p.process(b"\x1b[H\x1b[2K1> ");
    assert_eq!(
        composer(&p.screen().rows(0, 40).collect::<Vec<_>>()),
        Some("".into())
    );
}
#[test]
fn fragmented_queries_answer_once() {
    let q = replies(48, 160);
    let mut pending = vec![];
    assert!(answers(&mut pending, b"\x1b[18", &q).is_empty());
    assert_eq!(
        answers(&mut pending, b"t", &q),
        vec![b"\x1b[8;48;160t".to_vec()]
    );
    assert!(answers(&mut pending, b"x", &q).is_empty());
}
#[test]
fn pty_echo_is_application_rendered_and_unread_input_never_appears() {
    let mut c = Command::new("/bin/bash");
    c.args(["-c",r#"printf '\033[2J\033[H1> '; IFS= read -r -n1 c; printf '\033[3;1Htick'; sleep 0.05; printf '\033[1;1H1> %s' "$c"; sleep 2"#]);
    let client = Client::spawn(&mut c, 3, 40).unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while client.snapshot().unwrap().frames == 0 {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    let start = Instant::now();
    client.send(b"x").unwrap();
    loop {
        let s = client.snapshot().unwrap();
        if s.history
            .iter()
            .any(|(_, _, text)| text.as_deref() == Some("x"))
        {
            assert!(start.elapsed() >= Duration::from_millis(40));
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    drop(client);
    let mut c = Command::new("/bin/bash");
    c.args(["-c", r"printf '\033[2J\033[H1> '; sleep 2"]);
    let client = Client::spawn(&mut c, 3, 40).unwrap();
    client.send(b"not-rendered").unwrap();
    std::thread::sleep(Duration::from_millis(50));
    assert!(
        !client
            .snapshot()
            .unwrap()
            .rows
            .join("\n")
            .contains("not-rendered")
    );
}

#[test]
fn query_responses_follow_terminal_stream_order() {
    let q = replies(24, 80);
    let mut pending = vec![];
    assert_eq!(
        answers(&mut pending, b"\x1b]11;?\x07\x1b[c", &q),
        vec![
            b"\x1b]11;rgb:0000/0000/0000\x07".to_vec(),
            b"\x1b[?62;c".to_vec()
        ]
    );
    assert_eq!(
        answers(&mut pending, b"\x1b]4;0;?\x07", &q),
        vec![b"\x1b]4;0;rgb:0000/0000/0000\x07".to_vec()]
    );
}
