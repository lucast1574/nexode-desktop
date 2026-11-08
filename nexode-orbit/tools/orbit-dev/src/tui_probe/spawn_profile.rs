//! Cold client/server resource profiling, isolated from existing daemons.
use super::{proc_monitor::Monitor, screen::Client, single::Server};
use anyhow::{Context, Result, ensure};
use clap::Parser;
use regex::Regex;
use serde_json::{Value, json};
use std::{
    fs,
    os::unix::{net::UnixStream, process::CommandExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
#[derive(Parser)]
struct Args {
    binary: Option<PathBuf>,
    #[arg(long, default_value_t = 5)]
    runs: usize,
    #[arg(long, default_value_t = 2.0)]
    sample_interval_ms: f64,
    #[arg(long, default_value_t = 2.5)]
    client_hold_s: f64,
    #[arg(long)]
    json: Option<PathBuf>,
    #[arg(long)]
    skip_client: bool,
    #[arg(long)]
    skip_server: bool,
}
struct Workspace(tempfile::TempDir);
impl Drop for Workspace {
    fn drop(&mut self) {
        if let Err(e) = crate::detached_acceptance::cleanup_workspace(self.0.path()) {
            eprintln!("profile cleanup: {e:#}");
        }
    }
}
impl Workspace {
    fn new() -> Result<Self> {
        let w = Self(
            tempfile::Builder::new()
                .prefix("orbit-spawn-profile-")
                .tempdir()?,
        );
        for d in ["home", "run"] {
            fs::create_dir(w.0.path().join(d))?;
        }
        Ok(w)
    }
    fn socket(&self) -> PathBuf {
        self.0.path().join("run/nexode.sock")
    }
    fn command(&self, binary: &Path) -> Command {
        let mut c = Command::new(binary);
        c.env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("HOME", self.0.path().join("home"))
            .env("NEXODE_HOME", self.0.path().join("home"))
            .env("NEXODE_RUNTIME_DIR", self.0.path().join("run"))
            .env("NEXODE_SOCKET", self.socket())
            .env("NEXODE_TEST_WORKSPACE", self.0.path())
            .env("NEXODE_NO_TELEMETRY", "1")
            .env("NEXODE_TEMP_SERVER", "1")
            .env("NEXODE_SERVER_OWNER_PID", std::process::id().to_string())
            .env("ANTHROPIC_API_KEY", "sk-ant-local-ui-fixture")
            .env("TERM", "xterm-256color")
            .args(["--no-update", "--no-selfdev", "--socket"])
            .arg(self.socket());
        c
    }
    fn child_server(&self, exclude: u32) -> Option<u32> {
        let marker = format!("NEXODE_TEST_WORKSPACE={}", self.0.path().display());
        fs::read_dir("/proc")
            .ok()?
            .filter_map(Result::ok)
            .find_map(|e| {
                let pid = e.file_name().to_str()?.parse().ok()?;
                if pid == exclude {
                    return None;
                }
                let env = fs::read(e.path().join("environ")).ok()?;
                let cmd = fs::read(e.path().join("cmdline")).ok()?;
                (env.split(|b| *b == 0).any(|v| v == marker.as_bytes())
                    && cmd.split(|b| *b == 0).any(|v| v == b"serve"))
                .then_some(pid)
            })
    }
}
fn check() -> Result<()> {
    ensure!(
        crate::ci_suites::unix::interrupted().is_none(),
        "spawn profile interrupted"
    );
    Ok(())
}
fn startup(text: &str) -> Result<Option<Value>> {
    let total = Regex::new(r"Startup Profile \(([0-9.]+)ms total\)")?;
    let phase = Regex::new(r"\[INFO\]\s+([0-9.]+)ms\s+([0-9.]+)ms\s+[0-9.]+%\s+([a-zA-Z0-9_]+)")?;
    let history = Regex::new(r"remote bootstrap: history after ([0-9.]+)ms")?;
    let Some(index) = text.rfind("=== Startup Profile (") else {
        return Ok(None);
    };
    let block = text[index..]
        .lines()
        .take(40)
        .collect::<Vec<_>>()
        .join("\n");
    let deltas = phase
        .captures_iter(&block)
        .map(|m| Ok((m[3].to_owned(), json!(m[2].parse::<f64>()?))))
        .collect::<Result<serde_json::Map<_, _>>>()?;
    Ok(Some(
        json!({"total_ms":total.captures(&block).map(|m|m[1].parse::<f64>()).transpose()?.unwrap_or(0.0),"deltas_ms":deltas,
        "remote_history_ms":history.captures_iter(text).last().map(|m|m[1].parse::<f64>()).transpose()?}),
    ))
}
fn server_run(binary: &Path, interval: Duration) -> Result<Value> {
    let w = Workspace::new()?;
    let start = Instant::now();
    let mut server = Server(
        w.command(binary)
            .arg("serve")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()?,
    );
    let mut m = Monitor::new(server.0.id());
    let mut ready = None;
    loop {
        check()?;
        ensure!(
            server.0.try_wait()?.is_none() && m.poll(),
            "server exited before profiling completed"
        );
        if ready.is_none() && UnixStream::connect(w.socket()).is_ok() {
            ready = Some(start.elapsed().as_secs_f64() * 1000.0);
        }
        if ready.is_some_and(|r| start.elapsed().as_secs_f64() * 1000.0 >= r + 400.0) {
            break;
        }
        ensure!(
            start.elapsed() < Duration::from_secs(8),
            "server socket readiness timed out"
        );
        std::thread::sleep(interval);
    }
    let mut report = m.report();
    report["ready_ms"] = json!(ready);
    Ok(report)
}
fn client_run(binary: &Path, interval: Duration, hold: Duration) -> Result<Value> {
    let w = Workspace::new()?;
    let start = Instant::now();
    let client = Client::spawn(&mut w.command(binary), 40, 120)?;
    let mut cm = Monitor::new(client.pid);
    let mut sm: Option<Monitor> = None;
    let mut ready = None;
    while start.elapsed() < hold {
        check()?;
        let s = client.snapshot()?;
        ensure!(
            !s.eof && s.error.is_none() && cm.poll(),
            "client exited before profiling completed"
        );
        if sm.is_none() {
            sm = w.child_server(client.pid).map(Monitor::new);
        }
        if let Some(m) = &mut sm {
            ensure!(m.poll(), "spawned server exited");
            if ready.is_none() && UnixStream::connect(w.socket()).is_ok() {
                ready = Some(start.elapsed().as_secs_f64() * 1000.0);
            }
        }
        std::thread::sleep(interval);
    }
    ensure!(
        ready.is_some(),
        "cold client did not start its daemon within hold window"
    );
    let mut logs = fs::read_dir(w.0.path().join("home/logs"))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|s| s == "log"))
        .collect::<Vec<_>>();
    logs.sort();
    let profile = if let Some(p) = logs.last() {
        startup(&fs::read_to_string(p)?)?
    } else {
        None
    };
    Ok(
        json!({"client":cm.report(),"server":sm.map(|m|m.report()),"server_ready_ms":ready,"startup_profile":profile}),
    )
}
pub(super) fn run(args: impl Iterator<Item = String>) -> Result<i32> {
    let a = Args::try_parse_from(std::iter::once("profile-spawn".into()).chain(args))?;
    ensure!((1..=1000).contains(&a.runs), "runs must be 1..1000");
    ensure!(
        a.sample_interval_ms.is_finite() && (0.1..=1000.0).contains(&a.sample_interval_ms),
        "sample interval must be 0.1..1000 ms"
    );
    ensure!(
        a.client_hold_s.is_finite() && (0.1..=3600.0).contains(&a.client_hold_s),
        "client hold must be 0.1..3600 seconds"
    );
    ensure!(
        !(a.skip_client && a.skip_server),
        "cannot skip both profiles"
    );
    let binary = a
        .binary
        .or_else(|| std::env::var_os("NEXODE_BENCH_BIN").map(PathBuf::from))
        .unwrap_or_else(|| "nexode".into());
    let binary = if binary.exists() {
        binary
    } else {
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
            .map(|p| p.join(&binary))
            .find(|p| p.is_file())
            .context("binary not found")?
    }
    .canonicalize()?;
    let interval = Duration::from_secs_f64(a.sample_interval_ms / 1000.0);
    let mut report =
        json!({"binary":binary,"binary_size_bytes":fs::metadata(&binary)?.len(),"runs":a.runs});
    let mut warm = Command::new(&binary);
    warm.arg("--version");
    ensure!(
        crate::process::capture(&mut warm, Duration::from_secs(10))?
            .status
            .success(),
        "binary version check failed"
    );
    if !a.skip_server {
        let mut values = vec![];
        for _ in 0..a.runs {
            values.push(server_run(&binary, interval)?);
        }
        report["server"] = json!(values);
    }
    if !a.skip_client {
        let mut values = vec![];
        for _ in 0..a.runs {
            values.push(client_run(
                &binary,
                interval,
                Duration::from_secs_f64(a.client_hold_s),
            )?);
        }
        report["client_full"] = json!(values);
    }
    let text = serde_json::to_string_pretty(&report)?;
    if let Some(path) = a.json {
        fs::write(path, &text)?;
    }
    println!("{text}");
    Ok(0)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_last_profile_and_remote_history() {
        let s = "=== Startup Profile (2.0ms total)\n=== Startup Profile (12.5ms total)\n[INFO] 10ms 3.5ms 28% args_parse\nremote bootstrap: history after 8ms";
        let r = startup(s).unwrap().unwrap();
        assert_eq!(r["total_ms"], 12.5);
        assert_eq!(r["deltas_ms"]["args_parse"], 3.5);
        assert_eq!(r["remote_history_ms"], 8.0);
        assert!(startup("no block").unwrap().is_none());
    }
}
