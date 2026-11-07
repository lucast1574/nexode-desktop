#![cfg(unix)]
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixListener,
    process::Command,
};
#[test]
fn debug_cli_handles_ack_and_error_status() {
    for ok in [true, false] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("debug");
        let listener = UnixListener::bind(&path).unwrap();
        let worker = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(s.try_clone().unwrap())
                .read_line(&mut line)
                .unwrap();
            let v: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(v["command"], "state");
            assert_eq!(v["session_id"], "owned");
            writeln!(s, "{}", json!({"type":"ack"})).unwrap();
            writeln!(
                s,
                "{}",
                json!({"type":"debug_response","ok":ok,"output":"[]"})
            )
            .unwrap();
        });
        let out = Command::new(env!("CARGO_BIN_EXE_orbit-dev"))
            .args(["debug-call", "--socket"])
            .arg(path)
            .args(["--session", "owned", "state"])
            .output()
            .unwrap();
        worker.join().unwrap();
        assert_eq!(out.status.success(), ok);
        let v: Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(v["ok"], ok);
    }
}
#[test]
fn stress_rejects_invalid_counts() {
    for args in [
        vec!["0", "--fresh"],
        vec!["2", "--fresh", "--settle-seconds", "NaN"],
    ] {
        assert!(
            !Command::new(env!("CARGO_BIN_EXE_orbit-dev"))
                .arg("stress-test")
                .args(args)
                .output()
                .unwrap()
                .status
                .success()
        );
    }
}
