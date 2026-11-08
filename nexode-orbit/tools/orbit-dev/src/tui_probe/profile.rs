use super::{io, screen::Client};
use anyhow::{Context, Result, ensure};
use clap::Parser;
use serde_json::Value;
use std::{
    path::PathBuf,
    process::Command,
    time::{Duration, Instant},
};
pub(super) fn recent(value: &Value, work: &str) -> Option<String> {
    let rows = value.as_array().or_else(|| value["sessions"].as_array())?;
    rows.iter()
        .filter(|v| v["working_dir"].is_null() || v["working_dir"] == work)
        .filter_map(|v| {
            let id = v["id"]
                .as_str()
                .or_else(|| v["session_id"].as_str())
                .filter(|id| id.starts_with("session_"))?;
            let messages = v["message_count"]
                .as_u64()
                .filter(|n| *n > 0)
                .or_else(|| v["messages"].as_u64())
                .unwrap_or_else(|| v["messages"].as_array().map(Vec::len).unwrap_or(0) as u64);
            let date = v["updated_at"]
                .as_str()
                .or_else(|| v["last_active"].as_str())
                .unwrap_or("");
            Some(((messages > 0, date), id.to_owned()))
        })
        .max_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, id)| id)
}
#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "./target/selfdev/nexode")]
    binary: PathBuf,
    #[arg(long)]
    session: Option<String>,
    #[arg(long, default_value_t = 8.0)]
    seconds: f64,
    #[arg(long, default_value_t = 997)]
    freq: u32,
    #[arg(long)]
    socket: Option<PathBuf>,
    #[arg(long)]
    debug_socket: Option<PathBuf>,
    #[arg(long)]
    working_dir: Option<PathBuf>,
    #[arg(long, default_value = "perf")]
    perf: PathBuf,
}
pub(super) fn run(args: impl Iterator<Item = String>) -> Result<i32> {
    let a = Args::parse_from(std::iter::once("profile-real-spawn".into()).chain(args));
    ensure!(
        a.seconds.is_finite() && a.seconds > 0.0 && a.seconds <= 3600.0 && a.freq > 0,
        "positive duration <= 3600s and frequency required"
    );
    let binary = a.binary.canonicalize()?;
    let work = a.working_dir.unwrap_or(crate::root()?);
    let runtime = std::env::var_os("NEXODE_RUNTIME_DIR")
        .or_else(|| std::env::var_os("XDG_RUNTIME_DIR"))
        .map(PathBuf::from)
        .unwrap_or_else(|| format!("/run/user/{}", unsafe { libc::getuid() }).into());
    let socket = a
        .socket
        .or_else(|| std::env::var_os("NEXODE_SOCKET").map(PathBuf::from))
        .unwrap_or_else(|| runtime.join("nexode.sock"));
    let debug = a
        .debug_socket
        .unwrap_or_else(|| runtime.join("nexode-debug.sock"));
    let session = if let Some(s) = a.session {
        s
    } else {
        let previous = io::debug(&debug, "sessions", Duration::from_secs(30))
            .ok()
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .and_then(|v| recent(&v, &work.to_string_lossy()));
        match previous {
            Some(s) => s,
            None => io::create_session(&debug, &work)?,
        }
    };
    ensure!(session.starts_with("session_"), "invalid session ID");
    println!(
        "Profiling real spawn\n  binary: {}\n  session: {session}",
        binary.display()
    );
    let scratch = std::env::var_os("NEXODE_SCRATCH_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let root = tempfile::Builder::new()
        .prefix("orbit-profile-real-")
        .tempdir_in(scratch)?;
    let cmd = root.path().join("client_cmd");
    let resp = root.path().join("client_resp");
    let mut command = Command::new(binary);
    command
        .args(["--no-update", "--no-selfdev", "--socket"])
        .arg(socket)
        .args(["--resume", &session])
        .env("NEXODE_DEBUG_CONTROL", "1")
        .env("NEXODE_THEME", "dark")
        .env("NEXODE_DEBUG_CMD_PATH", &cmd)
        .env("NEXODE_DEBUG_RESPONSE_PATH", &resp)
        .env("TERM", "xterm-256color");
    let client = Client::spawn(&mut command, 48, 160)?;
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        ensure!(
            crate::ci_suites::unix::interrupted().is_none(),
            "profile interrupted"
        );
        if io::client_command(&cmd, &resp, "input", Duration::from_secs(2)).is_ok() {
            break;
        }
        if Instant::now() >= deadline || client.snapshot()?.eof {
            eprintln!("client never came up");
            return Ok(3);
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    std::thread::sleep(Duration::from_secs(3));
    let stats: Value = serde_json::from_str(&io::client_command(
        &cmd,
        &resp,
        "draw-stats 1",
        Duration::from_secs(8),
    )?)?;
    println!("  redraw schedule: {}", stats["redraw_schedule"]);
    let data = root.path().join("perf.data");
    let interrupt = || crate::ci_suites::unix::interrupted().is_some();
    let record = crate::process::capture_interruptible(
        Command::new(&a.perf)
            .args([
                "record",
                "-F",
                &a.freq.to_string(),
                "-g",
                "--pid",
                &client.pid.to_string(),
                "-o",
            ])
            .arg(&data)
            .args(["--", "sleep", &a.seconds.to_string()]),
        Duration::from_secs_f64(a.seconds + 10.0),
        interrupt,
    )?;
    if !record.status.success() || !data.is_file() {
        eprintln!(
            "perf record failed: {}",
            String::from_utf8_lossy(&record.stderr)
                .chars()
                .take(1500)
                .collect::<String>()
        );
        return Ok(3);
    }
    let report = crate::process::capture_interruptible(
        Command::new(&a.perf)
            .args(["report", "-i"])
            .arg(&data)
            .args([
                "--stdio",
                "--no-children",
                "--percent-limit",
                "1.0",
                "-g",
                "none",
            ]),
        Duration::from_secs(60),
        interrupt,
    )?;
    ensure!(report.status.success(), "perf report failed");
    println!("\n=== self time ===");
    for line in String::from_utf8(report.stdout)
        .context("perf output was not UTF-8")?
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
    {
        println!("{}", line.chars().take(150).collect::<String>());
    }
    Ok(0)
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn recent_prefers_content_and_respects_working_directory() {
        let v = json!({"sessions":[{"session_id":"session_empty","working_dir":"/work","updated_at":"2026-11-07","message_count":0},{"id":"session_content","working_dir":"/work","updated_at":"2026-11-01","message_count":2},{"id":"session_other","working_dir":"/other","updated_at":"2026-11-08","message_count":100}]});
        assert_eq!(recent(&v, "/work"), Some("session_content".into()));
        assert_eq!(recent(&v, "/missing"), None);
        assert_eq!(
            recent(
                &json!([{"session_id":"session_legacy","messages":[{}]}]),
                "/work"
            ),
            Some("session_legacy".into())
        );
    }
}
