//! Shared real PTY + native terminal-emulation diagnostics.
#[cfg(unix)]
mod animation;
#[cfg(unix)]
mod animation_verify;
#[cfg(unix)]
mod draws;
#[cfg(unix)]
mod expand_badge;
#[cfg(target_os = "linux")]
mod heap_diff;
#[cfg(unix)]
mod idle;
#[cfg(unix)]
mod idle_perf;
#[cfg(target_os = "linux")]
mod idle_rss;
#[cfg(unix)]
mod input_flicker;
#[cfg(unix)]
mod inspect;
#[cfg(unix)]
mod io;
#[cfg(unix)]
mod isolated;
#[cfg(unix)]
mod lag;
#[cfg(target_os = "linux")]
mod maki;
#[cfg(target_os = "linux")]
mod process_metrics;
#[cfg(unix)]
mod profile;
#[cfg(unix)]
mod screen;
#[cfg(unix)]
mod selfdev_status;
#[cfg(unix)]
mod single;
#[cfg(unix)]
mod slash;
#[cfg(unix)]
mod theme;
#[cfg(unix)]
mod usage_probe;
#[cfg(unix)]
mod visible;
use anyhow::Result;
pub fn run(command: &str, args: impl Iterator<Item = String>) -> Result<i32> {
    #[cfg(unix)]
    {
        let _signals = crate::ci_suites::unix::Signals::install()?;
        let result = if command == "test-selfdev-reload" {
            selfdev_status::run(args)
        } else if command == "expand-badge-headless" {
            expand_badge::run(args)
        } else if command == "repro-expand-edit-shortcut" {
            expand_badge::repro(args)
        } else if command == "test-oauth-usage" {
            usage_probe::oauth(args)
        } else if command == "test-caching-detailed" {
            usage_probe::run(args)
        } else if command == "heap-diff-idle-sessions" {
            #[cfg(target_os = "linux")]
            {
                heap_diff::run(args)
            }
            #[cfg(not(target_os = "linux"))]
            {
                anyhow::bail!("heap profiling requires Linux /proc");
            }
        } else if command == "repro-idle-session-rss" {
            #[cfg(target_os = "linux")]
            {
                idle_rss::run(args)
            }
            #[cfg(not(target_os = "linux"))]
            {
                anyhow::bail!("RSS probe requires Linux /proc")
            }
        } else if command == "bench-nexode-maki" {
            #[cfg(target_os = "linux")]
            {
                maki::run(args)
            }
            #[cfg(not(target_os = "linux"))]
            {
                anyhow::bail!("resource benchmark requires Linux /proc")
            }
        } else if matches!(command, "repro-startup-lag" | "repro-real-spawn-lag") {
            lag::run(command, args)
        } else if command == "repro-input-flicker" {
            input_flicker::run(args)
        } else if command == "repro-slash-flicker" {
            slash::run(args)
        } else if matches!(
            command,
            "check-donut-animates-live" | "verify-donut-still-animates"
        ) {
            animation_verify::run(command, args)
        } else if matches!(command, "diagnose-idle-render-cost" | "probe-idle-state") {
            idle::run(command, args)
        } else if command == "profile-idle-donut" {
            idle_perf::run(args)
        } else if command == "verify-light-theme" {
            theme::run(args)
        } else if matches!(
            command,
            "measure-animation-cpu-cost" | "sweep-animation-fps"
        ) {
            animation::run(command, args)
        } else if matches!(command, "count-idle-draws" | "pin-no-area-state") {
            draws::run(command, args)
        } else if matches!(
            command,
            "dump-fresh-spawn-screen" | "why-no-animation-area" | "which-overlay-blocks-donut"
        ) {
            inspect::run(command, args)
        } else if command == "profile-single-spawn" {
            single::run(args)
        } else if command == "bench-startup-visible-ready" {
            visible::run(args)
        } else if command == "profile-real-spawn" {
            profile::run(args)
        } else {
            echo(args)
        };
        if let Some(signal) = crate::ci_suites::unix::interrupted() {
            return Ok(128 + signal);
        }
        result
    }
    #[cfg(not(unix))]
    {
        let _ = (command, args);
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
