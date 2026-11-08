#![cfg(unix)]
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::{fs::PermissionsExt, net::UnixListener},
    process::Command,
    time::{Duration, Instant},
};
fn run(wrong_focus: bool) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let socket = root.join("debug");
    let listener = UnixListener::bind(&socket).unwrap();
    listener.set_nonblocking(true).unwrap();
    for (name, script) in [
        (
            "kitty",
            r#"#!/bin/sh
case "$4" in
launch)
  test "$7" = --os-window-title || exit 2
  printf '%s' "$8" > "$HEADED_FIXTURE/title"
  echo 42;;
focus-window) test "$6" = id:42;;
close-window) test "$6" = id:42 && touch "$HEADED_FIXTURE/closed";;
*) exit 3;;
esac
"#,
        ),
        (
            "niri",
            r#"#!/bin/sh
if test "$2" = --json; then
  if test -f "$HEADED_FIXTURE/title"; then
    printf '[{"id":7,"title":"%s","is_focused":%s}]' "$(cat "$HEADED_FIXTURE/title")" "$FOCUS_OK"
  else printf '[{"id":3,"title":"previous","is_focused":true}]'; fi
else
  printf '%s\n' "$*" >> "$HEADED_FIXTURE/focus"
fi
"#,
        ),
        (
            "wtype",
            r#"#!/bin/sh
test "$*" = '-M alt -M shift e -m shift -m alt' || exit 4
touch "$HEADED_FIXTURE/typed"
"#,
        ),
    ] {
        let path = root.join(name);
        fs::write(&path, script).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let work = root.to_owned();
    let server = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let (mut stream, _) = match listener.accept() {
                Ok(pair) => pair,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline, "fixture cleanup never arrived");
                    std::thread::sleep(Duration::from_millis(5));
                    continue;
                }
                Err(e) => panic!("{e}"),
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut line = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut line)
                .unwrap();
            let request: Value = serde_json::from_str(&line).unwrap();
            let command = request["command"].as_str().unwrap();
            let value = if command.starts_with("create_session:selfdev:") {
                json!({"session_id":"session_headed"})
            } else if command == "clients:map" {
                json!({"clients":[{"session_id":"session_headed"}]})
            } else if command == "client:expand-badge-state" {
                json!({"diff_mode":if work.join("typed").exists(){"FullInline"}else{"Inline"},"input":""})
            } else if command == "client:expand-badge-fixture"
                || command == "destroy_session:session_headed"
            {
                json!({})
            } else {
                panic!("unexpected request")
            };
            writeln!(stream, "{}", json!({"ok":true,"output":value.to_string()})).unwrap();
            if command == "destroy_session:session_headed" {
                break;
            }
        }
    });
    let output = Command::new(env!("CARGO_BIN_EXE_orbit-dev"))
        .args([
            "expand-badge-headed-wtype",
            "--binary",
            "/bin/true",
            "--debug-socket",
        ])
        .arg(&socket)
        .arg("--repo")
        .arg(root)
        .arg("--kitty-socket")
        .arg(root.join("kitty.sock"))
        .env("PATH", format!("{}:/usr/bin:/bin", root.display()))
        .env("HEADED_FIXTURE", root)
        .env("FOCUS_OK", if wrong_focus { "false" } else { "true" })
        .output()
        .unwrap();
    server.join().unwrap();
    assert_eq!(
        output.status.success(),
        !wrong_focus,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(root.join("typed").exists(), !wrong_focus);
    assert!(root.join("closed").exists());
    assert!(
        fs::read_to_string(root.join("focus"))
            .unwrap()
            .contains("--id 3")
    );
}
#[test]
fn headed_probe_types_only_into_verified_window_and_cleans_up() {
    run(false);
}
#[test]
fn headed_probe_refuses_wrong_focus_and_still_cleans_up() {
    run(true);
}
