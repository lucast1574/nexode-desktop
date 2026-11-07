//! Disposable resumed-session burst, with per-process (not tree) CPU/RSS samples.
use super::{
    io,
    screen::Client,
    single::{Server, wait_socket},
};
use anyhow::{Context, Result, ensure};
use clap::Parser;
use serde_json::json;
use std::{
    fs,
    os::unix::process::CommandExt,
    path::PathBuf,
    process::{Command, Stdio},
    time::{Duration, Instant},
};
#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "./target/release/nexode")]
    binary: PathBuf,
    #[arg(long, default_value_t = 20)]
    burst: usize,
    #[arg(long, default_value_t = 15.0)]
    timeout: f64,
    #[arg(long, default_value_t = 0.0)]
    stagger_ms: f64,
    #[arg(long, default_value = "/tmp/nexode_remote_burst_profile.json")]
    json_out: PathBuf,
}
#[derive(Default)]
struct Tracker {
    first: Option<u64>,
    last: u64,
    peak: u64,
}
fn parse_stat(s: &str, page: u64) -> Option<(u64, u64)> {
    let f: Vec<_> = s.rsplit_once(") ")?.1.split_whitespace().collect();
    Some((
        f.get(11)?
            .parse::<u64>()
            .ok()?
            .checked_add(f.get(12)?.parse().ok()?)?,
        f.get(21)?.parse::<u64>().ok()?.checked_mul(page)? / 1024,
    ))
}
impl Tracker {
    fn sample(&mut self, pid: u32, page: u64) -> Option<u64> {
        let (ticks, rss) =
            parse_stat(&fs::read_to_string(format!("/proc/{pid}/stat")).ok()?, page)?;
        self.first.get_or_insert(ticks);
        self.last = ticks;
        self.peak = self.peak.max(rss);
        Some(rss)
    }
    fn cpu(&self, hz: f64) -> f64 {
        self.last.saturating_sub(self.first.unwrap_or(self.last)) as f64 * 1000.0 / hz
    }
}
struct Live {
    client: Client,
    id: String,
    started: Instant,
    tracker: Tracker,
    done: bool,
}
pub(super) fn run(args: impl Iterator<Item = String>) -> Result<i32> {
    let a =
        Args::try_parse_from(std::iter::once("profile-remote-resume-burst".into()).chain(args))?;
    ensure!(
        (1..=1000).contains(&a.burst),
        "burst must be between 1 and 1000"
    );
    ensure!(
        a.timeout.is_finite() && a.timeout > 0.0 && a.timeout <= 3600.0,
        "timeout must be in (0,3600]"
    );
    ensure!(
        a.stagger_ms.is_finite() && (0.0..=3600000.0).contains(&a.stagger_ms),
        "invalid stagger"
    );
    let binary = a.binary.canonicalize().context("binary not found")?;
    let hz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    ensure!(hz > 0 && page > 0, "invalid process accounting units");
    let root = tempfile::Builder::new()
        .prefix("orbit-resume-burst-")
        .tempdir()?;
    let home = root.path().join("home");
    let runtime = root.path().join("run");
    fs::create_dir(&home)?;
    fs::create_dir(&runtime)?;
    let socket = runtime.join("nexode.sock");
    let debug = runtime.join("nexode-debug.sock");
    let command = || {
        let mut c = Command::new(&binary);
        c.env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("HOME", &home)
            .env("NEXODE_HOME", &home)
            .env("NEXODE_RUNTIME_DIR", &runtime)
            .env("NEXODE_SOCKET", &socket)
            .env("NEXODE_DEBUG_CONTROL", "1")
            .env("NEXODE_NO_TELEMETRY", "1")
            .env("NEXODE_TEMP_SERVER", "1")
            .env("NEXODE_SERVER_OWNER_PID", std::process::id().to_string())
            .env("ANTHROPIC_API_KEY", "sk-ant-local-ui-fixture")
            .env("TERM", "xterm-256color")
            .args(["--no-update", "--no-selfdev", "--socket"])
            .arg(&socket);
        c
    };
    let mut server = Server(
        command()
            .args(["serve", "--debug-socket"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()?,
    );
    wait_socket(&socket, &mut server, Duration::from_secs_f64(a.timeout))?;
    wait_socket(&debug, &mut server, Duration::from_secs_f64(a.timeout))?;
    let cwd = std::env::current_dir()?;
    let ids = (0..a.burst)
        .map(|_| io::create_session(&debug, &cwd))
        .collect::<Result<Vec<_>>>()?;
    let mut clients: Vec<Live> = vec![];
    let mut tracker = Tracker::default();
    let start = Instant::now();
    let deadline = start + Duration::from_secs_f64(a.timeout);
    let (mut peak_rss, mut peak_live) = (0u64, 0usize);
    loop {
        ensure!(
            crate::ci_suites::unix::interrupted().is_none(),
            "burst interrupted"
        );
        ensure!(server.0.try_wait()?.is_none(), "server exited during burst");
        if Instant::now() >= deadline {
            break;
        }
        while clients.len() < ids.len()
            && start.elapsed().as_secs_f64() * 1000.0 >= clients.len() as f64 * a.stagger_ms
            && Instant::now() < deadline
        {
            let id = ids[clients.len()].clone();
            let started = Instant::now();
            let client = Client::spawn(command().args(["--fresh-spawn", "--resume", &id]), 24, 80)?;
            clients.push(Live {
                client,
                id,
                started,
                tracker: Tracker::default(),
                done: false,
            });
        }
        tracker.sample(server.0.id(), page as u64);
        let (mut rss, mut live) = (0, 0);
        for c in &mut clients {
            if let Some(r) = c.tracker.sample(c.client.pid, page as u64) {
                rss += r;
                live += 1;
            }
            if !c.done {
                let s = c.client.snapshot()?;
                ensure!(s.error.is_none(), "PTY reader failed");
                ensure!(!s.eof, "resume client exited before observation completed");
                let text = s.rows.join("\n").to_lowercase();
                c.done = s.first_output.is_some()
                    && (text.contains("loading session")
                        || text.contains("nexode")
                        || s.bytes > 2048
                        || s.last_output
                            .is_some_and(|t| t.elapsed() >= Duration::from_millis(150)));
            }
        }
        peak_rss = peak_rss.max(rss);
        peak_live = peak_live.max(live);
        if clients.len() == a.burst && clients.iter().all(|c| c.done) {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let wall = start.elapsed().as_secs_f64() * 1000.0;
    let complete = clients.len() == a.burst && clients.iter().all(|c| c.done);
    let mut firsts = vec![];
    let mut results = vec![];
    let mut bytes = 0;
    let mut cpu = 0.0;
    for c in &clients {
        let s = c.client.snapshot()?;
        let first = s
            .first_output
            .map(|t| t.saturating_duration_since(c.started).as_secs_f64() * 1000.0);
        if let Some(f) = first {
            firsts.push(f);
        }
        bytes += s.bytes;
        cpu += c.tracker.cpu(hz as f64);
        results.push(json!({"session_id":c.id,"pid":c.client.pid,"first_output_ms":first,"buffer_bytes":s.bytes,"cpu_ms":c.tracker.cpu(hz as f64),"peak_rss_kb":c.tracker.peak,"complete":c.done}));
    }
    firsts.sort_by(f64::total_cmp);
    let median = if firsts.is_empty() {
        None
    } else {
        Some((firsts[(firsts.len() - 1) / 2] + firsts[firsts.len() / 2]) / 2.0)
    };
    let server_cpu = tracker.cpu(hz as f64);
    let output = json!({"burst":a.burst,"stagger_ms":a.stagger_ms,"wall_ms":wall,"complete":complete,"timed_out":!complete,
        "server_cpu_ms":server_cpu,"clients_cpu_ms":cpu,"total_cpu_ms":cpu+server_cpu,"cpu_utilization_ratio":(cpu+server_cpu)/wall,
        "first_output_ms":{"min":firsts.first(),"p50":median,"max":firsts.last()},"buffer_bytes_total":bytes,"buffer_bytes_include_terminal_queries":true,
        "server_peak_rss_kb":tracker.peak,"clients_peak_rss_kb":peak_rss,"peak_live_clients":peak_live,"results":results});
    let text = serde_json::to_string_pretty(&output)?;
    fs::write(&a.json_out, &text)?;
    println!("{text}");
    Ok(if complete { 0 } else { 1 })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stat_handles_parentheses_and_tracker_uses_delta() {
        let mut fields = vec!["0"; 22];
        fields[11] = "12";
        fields[12] = "3";
        fields[21] = "7";
        assert_eq!(
            parse_stat(&format!("12 (odd ) process) {}", fields.join(" ")), 4096),
            Some((15, 28))
        );
        assert_eq!(parse_stat("malformed", 4096), None);
        let t = Tracker {
            first: Some(15),
            last: 25,
            peak: 28,
        };
        assert_eq!(t.cpu(100.0), 100.0);
    }
}
