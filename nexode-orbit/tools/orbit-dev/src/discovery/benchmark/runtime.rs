use super::*;
use std::{
    fs::File,
    io::{BufRead, BufReader, Read, Seek, SeekFrom},
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
pub(super) struct Owned(pub Child);
impl Drop for Owned {
    fn drop(&mut self) {
        #[cfg(unix)]
        unsafe {
            libc::kill(-(self.0.id() as i32), libc::SIGKILL);
        }
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
pub(super) fn spawn(c: &mut Command) -> Result<Owned> {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        c.process_group(0);
    }
    Ok(Owned(c.spawn()?))
}
pub(super) struct Server {
    pub binary: PathBuf,
    pub socket: PathBuf,
    pub root: tempfile::TempDir,
    _child: Owned,
}
impl Server {
    pub fn start(binary: &Path, provider: Option<&str>) -> Result<Self> {
        let root = tempfile::Builder::new()
            .prefix("orbit-discovery-")
            .tempdir()?;
        let socket = root.path().join("nexode.sock");
        let log = File::create(root.path().join("server.log"))?;
        let mut cmd = command(binary, &socket);
        cmd.args(["--no-selfdev", "--no-update"]);
        if let Some(p) = provider {
            cmd.args(["--provider", p]);
        }
        cmd.args(["serve", "--server-name", "discovery-benchmark"])
            .stdout(log.try_clone()?)
            .stderr(log);
        let child = spawn(&mut cmd)?;
        let s = Self {
            binary: binary.to_owned(),
            socket,
            root,
            _child: child,
        };
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(20) {
            if s.debug("server:info", None, None, 2.).is_ok() {
                return Ok(s);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        anyhow::bail!(
            "benchmark daemon did not become ready: {}",
            std::fs::read_to_string(s.root.path().join("server.log"))?
        )
    }
    pub fn debug(
        &self,
        command_name: &str,
        arg: Option<&str>,
        session: Option<&str>,
        timeout: f64,
    ) -> Result<Value> {
        let mut c = command(&self.binary, &self.socket);
        c.arg("debug");
        if let Some(id) = session {
            c.args(["-S", id]);
        }
        c.arg(command_name);
        if let Some(a) = arg {
            c.arg(a);
        }
        let o = crate::process::capture(&mut c, Duration::from_secs_f64(timeout))?;
        ensure!(
            o.status.success(),
            "debug failed: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        Ok(serde_json::from_slice(&o.stdout)
            .unwrap_or_else(|_| json!(String::from_utf8_lossy(&o.stdout).trim())))
    }
    pub fn model_command(
        &self,
        model: &str,
        provider: Option<&str>,
        workspace: &Path,
        prompt: &str,
        only: bool,
    ) -> Command {
        let mut c = command(&self.binary, &self.socket);
        c.args(["--no-selfdev", "--no-update", "--model", model, "-C"])
            .arg(workspace);
        if let Some(p) = provider {
            c.args(["--provider", p]);
        }
        if only {
            c.args(["--disable-base-tools", "--tools", "integration_tools"]);
        }
        c.args(["run", "--ndjson", prompt]);
        c
    }
}
pub(super) fn command(binary: &Path, socket: &Path) -> Command {
    let mut c = Command::new(binary);
    c.arg("--socket")
        .arg(socket)
        .env_remove("NEXODE_SOCKET")
        .env_remove("NEXODE_SESSION_ID")
        .env_remove("NEXODE_PARENT_SESSION_ID")
        .env("NEXODE_DEBUG_CONTROL", "1")
        .env("NEXODE_DISCOVERY_BENCHMARK", "1")
        .env("NEXODE_RUNTIME_DIR", socket.parent().unwrap());
    c
}
pub(super) fn stream(
    c: &mut Command,
    timeout: f64,
    mut event: impl FnMut(Value, f64) -> bool,
) -> Result<Value> {
    ensure!(
        timeout.is_finite() && timeout > 0.,
        "timeout must be positive and finite"
    );
    let out = tempfile::NamedTempFile::new()?;
    let err = tempfile::NamedTempFile::new()?;
    c.stdin(Stdio::null())
        .stdout(File::create(out.path())?)
        .stderr(File::create(err.path())?);
    let mut child = spawn(c)?;
    let start = Instant::now();
    let mut reader = BufReader::new(File::open(out.path())?);
    let mut pending = String::new();
    let mut timed_out = false;
    let mut decided = false;
    loop {
        // A producer that continuously writes must not bypass the deadline.
        if start.elapsed().as_secs_f64() >= timeout {
            timed_out = true;
            break;
        }
        ensure!(
            out.as_file().metadata()?.len() < 64 * 1024 * 1024
                && err.as_file().metadata()?.len() < 16 * 1024 * 1024,
            "benchmark output limit exceeded"
        );
        let mut line = String::new();
        let n = reader.read_line(&mut line)?;
        if n > 0 {
            pending.push_str(&line);
            if pending.ends_with('\n') {
                if let Ok(v) = serde_json::from_str(&pending)
                    && event(v, start.elapsed().as_secs_f64())
                {
                    decided = true;
                    break;
                }
                pending.clear();
            }
            continue;
        }
        if child.0.try_wait()?.is_some() {
            if !pending.is_empty()
                && let Ok(v) = serde_json::from_str(&pending)
            {
                event(v, start.elapsed().as_secs_f64());
            }
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    if decided || timed_out {
        #[cfg(unix)]
        unsafe {
            libc::kill(-(child.0.id() as i32), libc::SIGTERM);
        }
        let until = Instant::now();
        while child.0.try_wait()?.is_none() && until.elapsed() < Duration::from_secs(2) {
            std::thread::sleep(Duration::from_millis(10));
        }
        if child.0.try_wait()?.is_none() {
            let _ = child.0.kill();
        }
    }
    let status = child.0.wait()?;
    let mut stderr = Vec::new();
    let mut f = File::open(err.path())?;
    f.seek(SeekFrom::Start(f.metadata()?.len().saturating_sub(4000)))?;
    f.read_to_end(&mut stderr)?;
    let stderr = String::from_utf8_lossy(&stderr);
    #[cfg(unix)]
    let code = {
        use std::os::unix::process::ExitStatusExt;
        status.code().or_else(|| status.signal().map(|s| -s))
    };
    #[cfg(not(unix))]
    let code = status.code();
    Ok(
        json!({"exit_code":code,"timed_out":timed_out,"elapsed_seconds":round(start.elapsed().as_secs_f64(),3),"stderr_tail":stderr}),
    )
}
