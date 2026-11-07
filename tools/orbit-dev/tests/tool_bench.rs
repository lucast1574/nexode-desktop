#![cfg(unix)]
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixListener,
    process::Command,
    time::{Duration, Instant},
};
#[test]
fn benchmark_resets_every_edit_and_writes_real_csv() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("src")).unwrap();
    fs::write(root.path().join("src/main.rs"), "fn main() {}\n").unwrap();
    let socket = root.path().join("debug");
    let csv = root.path().join("output.csv");
    let listener = UnixListener::bind(&socket).unwrap();
    listener.set_nonblocking(true).unwrap();
    let thread = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut edits = 0;
        loop {
            let (mut stream, _) = match listener.accept() {
                Ok(v) => v,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline);
                    std::thread::sleep(Duration::from_millis(2));
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
            let v: Value = serde_json::from_str(&line).unwrap();
            let cmd = v["command"].as_str().unwrap();
            let mut ok = true;
            let output = if cmd.starts_with("create_session:") {
                json!({"session_id":"session_bench"}).to_string()
            } else if cmd == "destroy_session:session_bench" {
                writeln!(
                    stream,
                    "{}",
                    json!({"type":"debug_response","ok":true,"output":"ok"})
                )
                .unwrap();
                return edits;
            } else {
                assert_eq!(v["session_id"], "session_bench");
                let (tool, input) = cmd.strip_prefix("tool:").unwrap().split_once(' ').unwrap();
                let input: Value = serde_json::from_str(input).unwrap();
                if tool == "edit" {
                    let path = input["file_path"].as_str().unwrap();
                    assert!(fs::read_to_string(path).unwrap().contains("quick brown"));
                    fs::write(path, "slow red").unwrap();
                    edits += 1;
                }
                if tool == "memory" {
                    assert_eq!(input["action"], "list");
                }
                ok = tool != "orbit_nonexistent_tool";
                json!({"output":"fixture","metadata":{"exit_code":0}}).to_string()
            };
            writeln!(
                stream,
                "{}",
                json!({"type":"debug_response","ok":ok,"output":output})
            )
            .unwrap();
        }
    });
    let out = Command::new(env!("CARGO_BIN_EXE_orbit-dev"))
        .args(["benchmark-tools", "2", "--socket"])
        .arg(socket)
        .arg("--repo")
        .arg(root.path())
        .arg("--csv")
        .arg(&csv)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(thread.join().unwrap(), 2);
    let report: Value = serde_json::from_slice(&out.stdout).unwrap();
    let rows = fs::read_to_string(csv).unwrap();
    assert_eq!(
        rows.lines().count(),
        1 + report["summary"].as_array().unwrap().len() * 2
    );
}
