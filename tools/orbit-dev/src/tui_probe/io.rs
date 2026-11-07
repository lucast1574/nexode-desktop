use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixStream,
    path::Path,
    time::{Duration, Instant},
};
pub(super) fn debug(socket: &Path, command: &str, timeout: Duration) -> Result<String> {
    debug_session(socket, command, None, timeout)
}
pub(super) fn debug_session(
    socket: &Path,
    command: &str,
    session: Option<&str>,
    timeout: Duration,
) -> Result<String> {
    let v = debug_response(socket, command, session, timeout)?;
    ensure!(
        v["type"] != "error" && v["ok"] != false,
        "debug command rejected"
    );
    Ok(v["output"].as_str().unwrap_or("").to_owned())
}
pub(super) fn debug_response(
    socket: &Path,
    command: &str,
    session: Option<&str>,
    timeout: Duration,
) -> Result<Value> {
    let mut stream = UnixStream::connect(socket)?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    let mut request = json!({"type":"debug_command","id":1,"command":command});
    if let Some(session) = session {
        request["session_id"] = json!(session);
    }
    writeln!(stream, "{request}")?;
    let mut reader = BufReader::new(stream);
    let deadline = Instant::now() + timeout;
    let mut bytes = vec![];
    loop {
        ensure!(Instant::now() < deadline, "debug response timed out");
        reader.get_ref().set_read_timeout(Some(
            deadline
                .saturating_duration_since(Instant::now())
                .max(Duration::from_millis(1)),
        ))?;
        let chunk = reader.fill_buf()?;
        ensure!(!chunk.is_empty(), "debug socket closed without response");
        let n = chunk
            .iter()
            .position(|b| *b == b'\n')
            .map(|n| n + 1)
            .unwrap_or(chunk.len());
        ensure!(bytes.len() + n <= 1024 * 1024, "debug response too large");
        bytes.extend_from_slice(&chunk[..n]);
        reader.consume(n);
        if bytes.last() == Some(&b'\n') {
            if bytes.iter().all(u8::is_ascii_whitespace) {
                bytes.clear();
                continue;
            }
            let v: Value = serde_json::from_slice(&bytes)?;
            bytes.clear();
            if matches!(v["type"].as_str(), Some("ack" | "pong")) {
                continue;
            }
            return Ok(v);
        }
    }
}
pub(super) fn client_command(
    command_path: &Path,
    response_path: &Path,
    command: &str,
    timeout: Duration,
) -> Result<String> {
    match fs::remove_file(response_path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    let parent = command_path.parent().context("command directory missing")?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.write_all(command.as_bytes())?;
    file.flush()?;
    file.persist(command_path).map_err(|e| e.error)?;
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        ensure!(
            crate::ci_suites::unix::interrupted().is_none(),
            "client command interrupted"
        );
        if response_path.exists() {
            std::thread::sleep(Duration::from_millis(20));
            return Ok(fs::read_to_string(response_path)?);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    anyhow::bail!("client did not answer command before timeout")
}
pub(super) fn create_session(debug_socket: &Path, work: &Path) -> Result<String> {
    let raw = debug(
        debug_socket,
        &format!("create_session:{}", work.display()),
        Duration::from_secs(30),
    )?;
    let session = if raw.trim_start().starts_with('{') {
        serde_json::from_str::<Value>(&raw)?["session_id"]
            .as_str()
            .unwrap_or("")
            .to_owned()
    } else {
        raw.split_whitespace().last().unwrap_or("").to_owned()
    };
    ensure!(
        session.starts_with("session_"),
        "unexpected create_session response"
    );
    Ok(session)
}
