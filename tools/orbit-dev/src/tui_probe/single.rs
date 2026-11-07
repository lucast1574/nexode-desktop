//! One isolated server/client startup, retaining logs only after a completed run.
use super::{io, screen::Client};
use anyhow::{Context, Result, ensure};
use clap::Parser;
use serde_json::json;
use std::{
    fs,
    os::unix::{net::UnixStream, process::CommandExt},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
struct Server(Child);
impl Drop for Server {
    fn drop(&mut self) {
        unsafe {
            libc::kill(-(self.0.id() as i32), libc::SIGTERM);
        }
        let deadline = Instant::now() + Duration::from_secs(1);
        while Instant::now() < deadline {
            if self.0.try_wait().ok().flatten().is_some() {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        unsafe {
            libc::kill(-(self.0.id() as i32), libc::SIGKILL);
        }
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn wait_socket(path: &Path, server: &mut Server, timeout: Duration) -> Result<()> {
    let deadline = Instant::now() + timeout;
    loop {
        ensure!(
            crate::ci_suites::unix::interrupted().is_none(),
            "startup profile interrupted"
        );
        ensure!(
            server.0.try_wait()?.is_none(),
            "isolated server exited before readiness"
        );
        if UnixStream::connect(path).is_ok() {
            return Ok(());
        }
        ensure!(
            Instant::now() < deadline,
            "socket not ready: {}",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn timing_lines(path: &Path) -> Result<Vec<String>> {
    Ok(String::from_utf8_lossy(&fs::read(path)?)
        .lines()
        .filter(|s| s.contains("[TIMING]") || s.contains("Startup Profile ("))
        .map(str::to_owned)
        .collect())
}
fn latest_log(dir: &Path) -> Result<PathBuf> {
    fs::read_dir(dir)?
        .map(|entry| -> Result<Option<_>> {
            let e = entry?;
            let n = e.file_name();
            let n = n.to_string_lossy();
            if !e.file_type()?.is_file() || !n.starts_with("nexode-") || !n.ends_with(".log") {
                return Ok(None);
            }
            Ok(Some((e.metadata()?.modified()?, e.path())))
        })
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .flatten()
        .max_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, p)| p)
        .context("no Nexode startup log found")
}
#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "./target/selfdev/nexode")]
    binary: PathBuf,
    #[arg(long, default_value_t = 20.0)]
    timeout: f64,
    #[arg(long)]
    cwd: Option<PathBuf>,
    #[arg(long)]
    json: bool,
}
pub(super) fn run(args: impl Iterator<Item = String>) -> Result<i32> {
    let a = Args::parse_from(std::iter::once("profile-single-spawn".into()).chain(args));
    ensure!(
        a.timeout.is_finite() && a.timeout > 0.0 && a.timeout <= 3600.0,
        "timeout must be positive and at most 3600 seconds"
    );
    let timeout = Duration::from_secs_f64(a.timeout);
    let binary = a.binary.canonicalize()?;
    let cwd = a.cwd.unwrap_or(std::env::current_dir()?).canonicalize()?;
    let root = tempfile::Builder::new()
        .prefix("orbit-single-profile-")
        .tempdir()?;
    let home = root.path().join("home");
    let runtime = root.path().join("run");
    fs::create_dir(&home)?;
    fs::create_dir(&runtime)?;
    let socket = runtime.join("nexode.sock");
    let debug = runtime.join("nexode-debug.sock");
    let make = || {
        let mut c = Command::new(&binary);
        c.current_dir(&cwd)
            .env("NEXODE_HOME", &home)
            .env("NEXODE_RUNTIME_DIR", &runtime)
            .env("NEXODE_SOCKET", &socket)
            .env("NEXODE_DEBUG_CONTROL", "1")
            .env("NEXODE_SWARM_ENABLED", "0")
            .env("NEXODE_NO_TELEMETRY", "1")
            .env("NEXODE_TRACE", "1")
            .env("NEXODE_TEMP_SERVER", "1")
            .env("NEXODE_SERVER_OWNER_PID", std::process::id().to_string())
            .env("TERM", "xterm-256color");
        c
    };
    let mut server_cmd = make();
    server_cmd
        .args(["--no-update", "--no-selfdev", "serve", "--socket"])
        .arg(&socket)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(fs::File::create(
            root.path().join("server-stderr.log"),
        )?))
        .process_group(0);
    let mut server = Server(server_cmd.spawn().context("launch isolated server")?);
    wait_socket(&socket, &mut server, timeout.min(Duration::from_secs(10)))?;
    wait_socket(&debug, &mut server, timeout.min(Duration::from_secs(10)))?;
    let session = io::create_session(&debug, &cwd)?;
    let mut command = make();
    command
        .args(["--no-update", "--no-selfdev", "--socket"])
        .arg(&socket)
        .args(["--fresh-spawn", "--resume", &session]);
    let start = Instant::now();
    let client = Client::spawn(&mut command, 24, 80)?;
    let mut first = None;
    while start.elapsed() < timeout {
        ensure!(
            crate::ci_suites::unix::interrupted().is_none(),
            "startup profile interrupted"
        );
        let s = client.snapshot()?;
        if first.is_none() {
            first = s
                .first_output
                .map(|at| at.duration_since(start).as_secs_f64() * 1000.0);
        }
        if s.eof
            || s.last_output
                .is_some_and(|at| at.elapsed() >= Duration::from_millis(200))
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    drop(client);
    drop(server);
    let log = latest_log(&home.join("logs"))?;
    let timings = timing_lines(&log)?;
    let root = root.keep();
    let result = json!({"temp_root":root,"log_path":log,"session_id":session,"first_output_ms":first,"timing_lines":timings});
    if a.json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        println!(
            "temp root: {}\nsession id: {session}\nlog path: {}\nfirst output: {}ms\ntimings:",
            root.display(),
            log.display(),
            first
                .map(|v| format!("{v:.1}"))
                .unwrap_or_else(|| "none".into())
        );
        for line in timings {
            println!("  {line}");
        }
    }
    Ok(i32::from(first.is_none()))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_matching_regular_logs_and_timing_lines_are_collected() {
        let d = tempfile::tempdir().unwrap();
        assert!(latest_log(d.path()).is_err());
        fs::write(
            d.path().join("nexode-one.log"),
            b"ordinary\n[TIMING] ready\nStartup Profile (2ms total)\ninvalid \xff",
        )
        .unwrap();
        fs::write(d.path().join("other.log"), "wrong").unwrap();
        fs::create_dir(d.path().join("nexode-directory.log")).unwrap();
        let p = latest_log(d.path()).unwrap();
        assert_eq!(p, d.path().join("nexode-one.log"));
        assert_eq!(
            timing_lines(&p).unwrap(),
            vec!["[TIMING] ready", "Startup Profile (2ms total)"]
        );
    }
    #[test]
    fn owned_server_is_reaped_after_socket_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let mut child = Command::new("/bin/sleep");
        child.arg("60").process_group(0);
        let mut server = Server(child.spawn().unwrap());
        let pid = server.0.id() as i32;
        assert!(
            wait_socket(
                &dir.path().join("missing"),
                &mut server,
                Duration::from_millis(30)
            )
            .is_err()
        );
        drop(server);
        assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
    }
}
