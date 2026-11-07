//! Session creation/load/recovery, with verified server ownership and real failures.
use super::{
    debug_fixture::{Fixture, OwnedSession},
    io, isolated,
};
use anyhow::{Context, Result, ensure};
use clap::Parser;
use serde_json::{Value, json};
use std::{
    fs,
    os::{fd::AsRawFd, unix::net::UnixStream},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
#[derive(Parser)]
struct Args {
    #[arg(default_value_t = 40)]
    sessions: usize,
    #[arg(long)]
    fresh: bool,
    #[arg(long, default_value = "./target/release/nexode")]
    binary: PathBuf,
    #[arg(long, required_unless_present = "fresh", conflicts_with = "fresh")]
    socket: Option<PathBuf>,
    #[arg(long, default_value_t = 10.0)]
    settle_seconds: f64,
    #[arg(long)]
    json: Option<PathBuf>,
    /// Make resource-growth warnings fail the run. Growth alone is not proof of a leak.
    #[arg(long)]
    strict_resources: bool,
}
fn peer_pid(socket: &Path) -> Result<u32> {
    let s = UnixStream::connect(socket)?;
    let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of_val(&cred) as libc::socklen_t;
    ensure!(
        unsafe {
            libc::getsockopt(
                s.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                (&mut cred as *mut libc::ucred).cast(),
                &mut len,
            )
        } == 0,
        "cannot identify socket peer"
    );
    ensure!(cred.pid > 0, "invalid socket peer pid");
    Ok(cred.pid as u32)
}
fn resources(pid: u32) -> Result<Value> {
    let status = fs::read_to_string(format!("/proc/{pid}/status"))?;
    let number = |key: &str| -> Result<u64> {
        status
            .lines()
            .find_map(|l| l.strip_prefix(key))
            .context("missing process statistic")?
            .split_whitespace()
            .next()
            .context("empty process statistic")?
            .parse()
            .map_err(Into::into)
    };
    Ok(
        json!({"rss_kb":number("VmRSS:")?,"vms_kb":number("VmSize:")?,"threads":number("Threads:")?,"fds":fs::read_dir(format!("/proc/{pid}/fd"))?.count()}),
    )
}
fn sessions(socket: &Path) -> Result<Vec<String>> {
    let value: Value =
        serde_json::from_str(&io::debug(socket, "sessions", Duration::from_secs(30))?)?;
    let rows = value
        .as_array()
        .or_else(|| value["sessions"].as_array())
        .context("unexpected session listing")?;
    rows.iter()
        .map(|v| {
            v["id"]
                .as_str()
                .or_else(|| v["session_id"].as_str())
                .map(str::to_owned)
                .context("session without id")
        })
        .collect()
}
fn summary(times: &[f64]) -> Value {
    let mut sorted = times.to_vec();
    sorted.sort_by(f64::total_cmp);
    json!({"min":sorted.first(),"max":sorted.last(),"avg":times.iter().sum::<f64>()/times.len().max(1) as f64,"p95":sorted.get((sorted.len()*95/100).min(sorted.len().saturating_sub(1))),"total":times.iter().sum::<f64>()})
}
pub(super) fn run(args: impl Iterator<Item = String>) -> Result<i32> {
    let a = Args::try_parse_from(std::iter::once("stress-test".into()).chain(args))?;
    ensure!((1..=1000).contains(&a.sessions), "sessions must be 1..1000");
    ensure!(
        a.settle_seconds.is_finite() && (0.0..=300.0).contains(&a.settle_seconds),
        "invalid settle interval"
    );
    let fixture = if a.fresh {
        Some(Fixture::new(&a.binary.canonicalize()?)?)
    } else {
        None
    };
    let tmp = tempfile::tempdir()?;
    let socket = fixture
        .as_ref()
        .map(|f| &f.socket)
        .or(a.socket.as_ref())
        .context("socket required")?;
    let cwd = fixture
        .as_ref()
        .map(|f| f.workspace.as_path())
        .unwrap_or(tmp.path());
    let pid = peer_pid(socket)?;
    let baseline = resources(pid)?;
    let existing = sessions(socket)?;
    let mut owned = vec![];
    let mut create = vec![];
    let mut timeline = vec![];
    for i in 0..a.sessions {
        ensure!(
            crate::ci_suites::unix::interrupted().is_none(),
            "stress test interrupted"
        );
        let t = Instant::now();
        owned.push(OwnedSession::new(socket, cwd)?);
        create.push(t.elapsed().as_secs_f64() * 1000.0);
        if (i + 1) % 10 == 0 || i + 1 == a.sessions {
            timeline.push(json!({"sessions":i+1,"resources":resources(pid)?}));
        }
    }
    for s in &owned {
        let state: Value = serde_json::from_str(&s.command("state")?)?;
        ensure!(
            state["session_id"] == s.id,
            "created session cannot be addressed"
        );
    }
    let listed = sessions(socket)?;
    let loaded: Value = serde_json::from_str(&io::debug(
        socket,
        "server:memory",
        Duration::from_secs(30),
    )?)?;
    let mut probes = vec![];
    for _ in 0..10 {
        let t = Instant::now();
        owned[0].command("state")?;
        probes.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    let mut tool_times = vec![];
    for (i, s) in owned.iter().take(5).enumerate() {
        let t = Instant::now();
        let token = format!("stress_test_{i}");
        let out = s.command(&format!(
            "tool:bash {}",
            json!({"command":format!("printf {token}")})
        ))?;
        ensure!(
            out.contains(&token),
            "tool output missing on selected session"
        );
        tool_times.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    let peak = resources(pid)?;
    let mut destroy = vec![];
    for s in &mut owned {
        let t = Instant::now();
        s.close()?;
        destroy.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    isolated::pause(a.settle_seconds)?;
    ensure!(peer_pid(socket)? == pid, "daemon changed during test");
    let t = Instant::now();
    let remaining = sessions(socket)?;
    let final_latency = t.elapsed().as_secs_f64() * 1000.0;
    for s in &owned {
        ensure!(
            !remaining.contains(&s.id),
            "destroyed session remains listed"
        );
    }
    for id in &existing {
        ensure!(remaining.contains(id), "pre-existing session disappeared");
    }
    let final_stats = resources(pid)?;
    let delta =
        |key: &str| final_stats[key].as_i64().unwrap_or(0) - baseline[key].as_i64().unwrap_or(0);
    let resources_ok = delta("fds").abs() <= 10
        && delta("threads").abs() <= 5
        && (delta("rss_kb") as f64) < baseline["rss_kb"].as_f64().unwrap_or(100000.0) * 0.2;
    let report = json!({"sessions":a.sessions,"server_pid":pid,"connected_sessions_under_load":listed.len(),"live_sessions_under_load":loaded["sessions"]["live_count"],"existing_sessions_preserved":true,"session_checks":"passed","create_ms":summary(&create),"destroy_ms":summary(&destroy),"debug_ms":summary(&probes),"tool_ms":summary(&tool_times),"final_socket_ms":final_latency,
        "baseline":baseline,"peak":peak,"final":final_stats,"timeline":timeline,"resource_growth_within_threshold":resources_ok,"rss_delta_kb":delta("rss_kb"),"fd_delta":delta("fds"),"thread_delta":delta("threads")});
    let text = serde_json::to_string_pretty(&report)?;
    if let Some(path) = a.json {
        fs::write(path, &text)?;
    }
    println!("{text}");
    Ok(if a.strict_resources && !resources_ok {
        1
    } else {
        0
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn socket_peer_is_real_process_and_resource_fields_exist() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("s");
        let _listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        assert_eq!(peer_pid(&path).unwrap(), std::process::id());
        assert!(
            resources(std::process::id()).unwrap()["rss_kb"]
                .as_u64()
                .unwrap()
                > 0
        );
    }
    #[test]
    fn latency_percentile_and_empty_samples() {
        let r = summary(&[3.0, 1.0, 2.0]);
        assert_eq!(r["p95"], 3.0);
        assert_eq!(r["avg"], 2.0);
        assert_eq!(summary(&[])["avg"], 0.0);
    }
}
