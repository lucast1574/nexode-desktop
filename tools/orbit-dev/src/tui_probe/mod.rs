//! Shared real PTY + native terminal-emulation diagnostics.
#[cfg(unix)]
mod io;
#[cfg(unix)]
mod screen;
use anyhow::Result;
pub fn run(args: impl Iterator<Item = String>) -> Result<i32> {
    #[cfg(unix)]
    {
        let _signals = crate::ci_suites::unix::Signals::install()?;
        let result = echo(args);
        if let Some(signal) = crate::ci_suites::unix::interrupted() {
            return Ok(128 + signal);
        }
        result
    }
    #[cfg(not(unix))]
    {
        let _ = args;
        anyhow::bail!("TUI probes require Unix PTYs");
    }
}
#[cfg(unix)]
fn echo(args: impl Iterator<Item = String>) -> Result<i32> {
    use anyhow::{Context, ensure};
    use clap::Parser;
    use serde_json::json;
    use std::{
        path::PathBuf,
        process::Command,
        time::{Duration, Instant},
    };
    #[derive(Parser)]
    struct Args {
        #[arg(default_value = "./target/selfdev/nexode")]
        binary: PathBuf,
        #[arg(long)]
        socket: Option<PathBuf>,
        #[arg(long)]
        debug_socket: Option<PathBuf>,
        #[arg(long)]
        working_dir: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    }
    let a = Args::parse_from(std::iter::once("measure-key-echo".into()).chain(args));
    let binary = a.binary.canonicalize()?;
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
    let work = a.working_dir.unwrap_or(crate::root()?);
    let root = tempfile::Builder::new().prefix("orbit-echo-").tempdir()?;
    let cmd_path = root.path().join("cmd");
    let resp_path = root.path().join("resp");
    let session = io::create_session(&debug, &work)?;
    let mut command = Command::new(binary);
    command
        .args(["--no-update", "--no-selfdev", "--socket"])
        .arg(socket)
        .args(["--resume", &session])
        .env("NEXODE_DEBUG_CONTROL", "1")
        .env("NEXODE_DEBUG_CMD_PATH", &cmd_path)
        .env("NEXODE_DEBUG_RESPONSE_PATH", &resp_path)
        .env("TERM", "xterm-256color")
        .env(
            "NEXODE_THEME",
            std::env::var("NEXODE_THEME").unwrap_or_else(|_| "dark".into()),
        );
    let client = screen::Client::spawn(&mut command, 48, 160)?;
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        ensure!(
            crate::ci_suites::unix::interrupted().is_none(),
            "terminal probe interrupted"
        );
        if io::client_command(&cmd_path, &resp_path, "input", Duration::from_secs(2)).is_ok() {
            break;
        }
        ensure!(
            Instant::now() < deadline && !client.snapshot()?.eof,
            "client did not become ready"
        );
        std::thread::sleep(Duration::from_millis(200));
    }
    std::thread::sleep(Duration::from_millis(1500));
    let mut typed = String::new();
    let mut results = vec![];
    let mut failed = false;
    for ch in "abcdefgh".chars() {
        typed.push(ch);
        let before = client.snapshot()?;
        let mark = before.history.back().map(|(n, _, _)| *n).unwrap_or(0);
        let start = Instant::now();
        client.send(ch.to_string().as_bytes())?;
        let mut first = None;
        let mut shown = None;
        while start.elapsed() < Duration::from_millis(1500) {
            ensure!(
                crate::ci_suites::unix::interrupted().is_none(),
                "terminal probe interrupted"
            );
            let state = client.snapshot()?;
            if first.is_none() && state.frames > before.frames {
                first = Some(start.elapsed().as_secs_f64() * 1000.0);
            }
            for (n, at, text) in state.history.iter().filter(|(n, _, _)| *n > mark) {
                let _ = n;
                if text.as_ref().is_some_and(|s| s.contains(&typed)) {
                    shown = Some(
                        at.checked_duration_since(start)
                            .context("history timestamp preceded input")?
                            .as_secs_f64()
                            * 1000.0,
                    );
                    break;
                }
            }
            if shown.is_some() || state.eof {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        failed |= shown.is_none();
        let result = json!({"typed":typed,"first_output_ms":first,"composer_shows_ms":shown});
        if !a.json {
            println!("{result}");
        }
        results.push(result);
    }
    if a.json {
        println!("{}", serde_json::to_string_pretty(&results)?);
    }
    Ok(i32::from(failed))
}
