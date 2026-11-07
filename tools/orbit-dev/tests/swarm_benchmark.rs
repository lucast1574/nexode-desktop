#![cfg(unix)]
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixListener,
    process::Command,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
#[test]
fn both_arms_route_jobs_cleanup_owned_sessions_and_preserve_source() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("artifact"), "source").unwrap();
    let output = root.path().join("results");
    let socket = root.path().join("debug.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    listener.set_nonblocking(true).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let stopping = stop.clone();
    let commands = Arc::new(Mutex::new(vec![]));
    let recorded = commands.clone();
    let thread = std::thread::spawn(move || {
        let mut next = 0;
        while !stopping.load(Ordering::Relaxed) {
            let Ok((mut stream, _)) = listener.accept() else {
                std::thread::sleep(Duration::from_millis(5));
                continue;
            };
            let mut line = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut line)
                .unwrap();
            let req: Value = serde_json::from_str(&line).unwrap();
            let cmd = req["command"].as_str().unwrap();
            recorded.lock().unwrap().push(req.clone());
            let response = if cmd.starts_with("create_session:") {
                next += 1;
                json!({"session_id":format!("session_{next}")})
            } else if cmd.starts_with("swarm:session:") {
                json!({"swarm_id":format!("owned:{next}")})
            } else if cmd.starts_with("message_async:") || cmd.starts_with("swarm_message_async:") {
                json!({"job_id":format!("job_{next}")})
            } else if cmd.starts_with("job_status:") {
                json!({"status":"completed"})
            } else if cmd == "swarm:members" {
                json!([])
            } else {
                json!({"ok":true})
            };
            writeln!(stream, "{}", json!({"type":"ack","id":1})).unwrap();
            writeln!(
                stream,
                "{}",
                json!({"type":"debug_response","id":1,"ok":true,"output":response.to_string()})
            )
            .unwrap();
        }
    });
    let result = Command::new(env!("CARGO_BIN_EXE_orbit-dev"))
        .args(["benchmark-swarm", "both", "--socket"])
        .arg(&socket)
        .arg("--source")
        .arg(&source)
        .arg("--output")
        .arg(&output)
        .args([
            "--evaluator-command",
            "sh -c 'echo CYCLES: 123'",
            "--timeout",
            "0.1",
            "--check-interval",
            "0.01",
        ])
        .output()
        .unwrap();
    stop.store(true, Ordering::Relaxed);
    thread.join().unwrap();
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    let report: Value =
        serde_json::from_slice(&fs::read(output.join("results.json")).unwrap()).unwrap();
    for mode in ["single", "swarm"] {
        assert_eq!(report[mode]["cycles"], 123);
        assert_eq!(report[mode]["passed"], true);
    }
    let cmds = commands.lock().unwrap();
    assert!(cmds.iter().any(
        |r| r["command"].as_str().unwrap().starts_with("message_async:")
            && r["session_id"] == "session_1"
    ));
    assert!(cmds.iter().any(|r| {
        r["command"]
            .as_str()
            .unwrap()
            .starts_with("swarm_message_async:")
            && r["session_id"] == "session_2"
    }));
    for n in 1..=2 {
        assert!(
            cmds.iter()
                .any(|r| r["command"] == format!("destroy_session:session_{n}"))
        );
    }
    assert_eq!(
        fs::read_to_string(source.join("artifact")).unwrap(),
        "source"
    );
    assert!(!source.join(".git").exists());
}
