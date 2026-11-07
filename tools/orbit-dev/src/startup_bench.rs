//! Isolated startup measurements; no shell, timeout utility or Python runtime.
use anyhow::{Context, Result, ensure};
use clap::Parser;
use regex::Regex;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
#[derive(Parser)]
struct Args {
    #[arg(default_value = "./target/release/nexode")]
    binary: PathBuf,
    #[arg(long, default_value_t = 5)]
    runs: usize,
    #[arg(long)]
    check: bool,
    #[arg(long, default_value_t = 20.0)]
    max_help_ms: f64,
    #[arg(long, default_value_t = 20.0)]
    max_version_ms: f64,
    #[arg(long, default_value_t = 80.0)]
    max_server_ready_ms: f64,
    #[arg(long, default_value_t = 150.0)]
    max_cold_total_ms: f64,
    #[arg(long, default_value_t = 20.0)]
    max_cold_server_check_ms: f64,
    #[arg(long, default_value_t = 20.0)]
    max_cold_server_spawn_ms: f64,
    #[arg(long, default_value_t = 20.0)]
    max_cold_app_new_ms: f64,
    #[arg(long, default_value_t = 250.0)]
    max_remote_bootstrap_history_ms: f64,
}
#[derive(Debug)]
struct Profile {
    total: f64,
    phases: BTreeMap<String, f64>,
    history: Option<f64>,
}
fn parse_profile(text: &str) -> Result<Profile> {
    let total = Regex::new(r"Startup Profile \(([0-9.]+)ms total\)")?;
    let phase = Regex::new(r"\[INFO\]\s+([0-9.]+)ms\s+([0-9.]+)ms\s+[0-9.]+%\s+([a-zA-Z0-9_]+)")?;
    let history = Regex::new(r"remote bootstrap: history after ([0-9.]+)ms")?;
    let lines: Vec<_> = text.lines().collect();
    let start = lines
        .iter()
        .rposition(|s| s.contains("=== Startup Profile ("))
        .context("no startup profile found")?;
    let mut value = None;
    let mut phases = BTreeMap::new();
    for line in lines.iter().skip(start).take(40) {
        if let Some(c) = total.captures(line) {
            value = Some(c[1].parse()?);
        }
        if let Some(c) = phase.captures(line) {
            phases.insert(c[3].into(), c[2].parse()?);
        }
    }
    let history = history
        .captures_iter(text)
        .last()
        .map(|c| c[1].parse())
        .transpose()?;
    Ok(Profile {
        total: value.context("invalid startup profile total")?,
        phases,
        history,
    })
}
fn stats(name: &str, values: &[f64]) -> Result<f64> {
    ensure!(!values.is_empty(), "{name}: no successful runs");
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let n = sorted.len();
    let median = if n.is_multiple_of(2) {
        (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0
    } else {
        sorted[n / 2]
    };
    let mean = values.iter().sum::<f64>() / n as f64;
    println!(
        "\n{name}:\n  Min:    {:.2} ms\n  Max:    {:.2} ms\n  Mean:   {mean:.2} ms\n  Median: {median:.2} ms",
        sorted[0],
        sorted[n - 1]
    );
    if n > 1 {
        println!(
            "  Stdev:  {:.2} ms",
            (values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n - 1) as f64).sqrt()
        );
    }
    Ok(median)
}
struct Sandbox {
    root: tempfile::TempDir,
}
impl Sandbox {
    fn new() -> Result<Self> {
        let s = Self {
            root: tempfile::Builder::new()
                .prefix("orbit-startup-")
                .tempdir()?,
        };
        for dir in ["home", "run"] {
            fs::create_dir(s.root.path().join(dir))?;
        }
        Ok(s)
    }
    fn command(&self, binary: &Path) -> Command {
        let mut c = Command::new(binary);
        c.env("NEXODE_HOME", self.root.path().join("home"))
            .env("NEXODE_RUNTIME_DIR", self.root.path().join("run"))
            .env("NEXODE_SOCKET", self.socket())
            .env("NEXODE_TEST_WORKSPACE", self.root.path())
            .env("NEXODE_NO_TELEMETRY", "1");
        c
    }
    fn socket(&self) -> PathBuf {
        self.root.path().join("run/nexode.sock")
    }
}
impl Drop for Sandbox {
    fn drop(&mut self) {
        if let Err(e) = crate::detached_acceptance::cleanup_workspace(self.root.path()) {
            eprintln!("startup sandbox cleanup: {e:#}");
        }
    }
}
fn simple(binary: &Path, arg: &str, runs: usize) -> Result<Vec<f64>> {
    let mut times = Vec::new();
    for _ in 0..runs {
        let sandbox = Sandbox::new()?;
        let mut c = sandbox.command(binary);
        c.arg(arg)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let start = Instant::now();
        let mut child = c.spawn()?;
        loop {
            if let Some(status) = child.try_wait()? {
                ensure!(status.success(), "{arg} failed: {status}");
                break;
            }
            if start.elapsed() > Duration::from_secs(10) {
                child.kill()?;
                child.wait()?;
                anyhow::bail!("{arg} timed out");
            }
            std::thread::sleep(Duration::from_micros(100));
        }
        times.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    Ok(times)
}
#[cfg(target_os = "linux")]
fn server(binary: &Path, runs: usize) -> Result<Vec<f64>> {
    let mut times = Vec::new();
    for _ in 0..runs {
        let sandbox = Sandbox::new()?;
        let mut c = sandbox.command(binary);
        c.arg("serve");
        let start = Instant::now();
        let mut terminal = crate::pty::Terminal::spawn(&mut c, 40, 120)?;
        loop {
            if std::os::unix::net::UnixStream::connect(sandbox.socket()).is_ok() {
                times.push(start.elapsed().as_secs_f64() * 1000.0);
                break;
            }
            let _ = terminal.read(Duration::from_millis(5))?;
            ensure!(
                terminal.status()?.is_none(),
                "server exited before socket was connectable"
            );
            ensure!(
                start.elapsed() < Duration::from_secs(5),
                "server socket readiness timed out"
            );
        }
        terminal.stop()?;
    }
    Ok(times)
}
#[cfg(target_os = "linux")]
fn cold(binary: &Path, runs: usize) -> Result<Vec<Profile>> {
    let mut profiles = Vec::new();
    for _ in 0..runs {
        let sandbox = Sandbox::new()?;
        let mut c = sandbox.command(binary);
        c.args(["--no-update", "--debug-socket", "--socket"])
            .arg(sandbox.socket())
            .env("TERM", "xterm-256color");
        let mut terminal = crate::pty::Terminal::spawn(&mut c, 40, 120)?;
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(3) {
            let chunk = terminal.read(Duration::from_millis(20))?;
            if chunk
                .windows(b"\x1b]11;?\x07".len())
                .any(|w| w == b"\x1b]11;?\x07")
            {
                terminal.write(b"\x1b]11;rgb:0000/0000/0000\x07")?;
            }
            if terminal.eof || terminal.status()?.is_some() {
                break;
            }
        }
        terminal.stop()?;
        let dir = sandbox.root.path().join("home/logs");
        let mut logs = fs::read_dir(&dir)
            .with_context(|| format!("no logs in {}", dir.display()))?
            .collect::<std::io::Result<Vec<_>>>()?;
        logs.sort_by_key(|e| e.file_name());
        let mut text = String::new();
        for e in logs {
            let n = e.file_name();
            let n = n.to_string_lossy();
            if n.starts_with("nexode-") && n.ends_with(".log") {
                text.push_str(&fs::read_to_string(e.path())?);
                text.push('\n');
            }
        }
        profiles.push(parse_profile(&text)?);
    }
    Ok(profiles)
}
#[cfg(not(target_os = "linux"))]
fn server(_binary: &Path, _runs: usize) -> Result<Vec<f64>> {
    anyhow::bail!("startup process isolation currently requires Linux")
}
#[cfg(not(target_os = "linux"))]
fn cold(_binary: &Path, _runs: usize) -> Result<Vec<Profile>> {
    anyhow::bail!("startup process isolation currently requires Linux")
}
pub fn run(args: impl Iterator<Item = String>) -> Result<()> {
    let args = Args::parse_from(std::iter::once("bench-startup".into()).chain(args));
    ensure!(
        cfg!(target_os = "linux"),
        "startup process isolation currently requires Linux"
    );
    ensure!(args.runs > 0, "runs must be positive");
    let limits = [
        args.max_help_ms,
        args.max_version_ms,
        args.max_server_ready_ms,
        args.max_cold_total_ms,
        args.max_cold_server_check_ms,
        args.max_cold_server_spawn_ms,
        args.max_cold_app_new_ms,
        args.max_remote_bootstrap_history_ms,
    ];
    ensure!(
        limits.iter().all(|v| v.is_finite() && *v >= 0.0),
        "budgets must be finite and nonnegative"
    );
    let binary = args
        .binary
        .canonicalize()
        .context("binary not found; build it first")?;
    println!("Benchmarking: {}", binary.display());
    let mut budgets = Vec::new();
    for (name, flag, limit) in [
        ("--help median", "--help", args.max_help_ms),
        ("--version median", "--version", args.max_version_ms),
    ] {
        budgets.push((
            name,
            stats(name, &simple(&binary, flag, args.runs)?)?,
            limit,
        ));
    }
    budgets.push((
        "server ready median",
        stats(
            "Server ready (isolated socket connectable)",
            &server(&binary, args.runs)?,
        )?,
        args.max_server_ready_ms,
    ));
    let profiles = cold(&binary, args.runs)?;
    budgets.push((
        "cold startup total median",
        stats(
            "Cold client startup total",
            &profiles.iter().map(|p| p.total).collect::<Vec<_>>(),
        )?,
        args.max_cold_total_ms,
    ));
    for (phase, limit) in [
        ("server_check", args.max_cold_server_check_ms),
        ("server_spawn_start", args.max_cold_server_spawn_ms),
        ("app_new_for_remote", args.max_cold_app_new_ms),
    ] {
        let values = profiles
            .iter()
            .map(|p| {
                p.phases
                    .get(phase)
                    .copied()
                    .with_context(|| format!("missing startup phase {phase}"))
            })
            .collect::<Result<Vec<_>>>()?;
        budgets.push((phase, stats(phase, &values)?, limit));
    }
    let ready: Vec<_> = profiles
        .iter()
        .filter_map(|p| p.phases.get("server_ready").copied())
        .collect();
    if !ready.is_empty() {
        stats("Cold client phase: server_ready", &ready)?;
    }
    let history: Vec<_> = profiles.iter().filter_map(|p| p.history).collect();
    if !history.is_empty() {
        budgets.push((
            "remote bootstrap history",
            stats("Remote bootstrap history", &history)?,
            args.max_remote_bootstrap_history_ms,
        ));
    }
    if args.check {
        println!("\nBudget check:");
        for (name, actual, limit) in &budgets {
            println!(
                "  [{}] {name}: {actual:.1} ms <= {limit:.1} ms",
                if actual > limit { "FAIL" } else { "PASS" }
            );
        }
        if budgets.iter().any(|(_, a, l)| a > l) {
            eprintln!("Startup regression detected.");
            std::process::exit(2);
        }
        println!("All startup budgets passed.");
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn latest_profile_uses_delta_and_latest_history() {
        let p=parse_profile("=== Startup Profile (999ms total)\n[INFO] 10ms 2ms 20% server_check\nremote bootstrap: history after 90ms\n=== Startup Profile (12.5ms total)\n[INFO] 11ms 3.5ms 28% server_check\nremote bootstrap: history after 7ms").unwrap();
        assert_eq!(p.total, 12.5);
        assert_eq!(p.phases["server_check"], 3.5);
        assert_eq!(p.history, Some(7.0));
        assert!(parse_profile("no profile").is_err());
    }
}
