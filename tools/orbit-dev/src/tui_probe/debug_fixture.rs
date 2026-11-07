//! Disposable debug server. Used for tools that do not require real inference.
use super::{
    io,
    single::{Server, wait_socket},
};
use anyhow::Result;
use std::{
    fs,
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};
pub(super) struct Fixture {
    pub socket: PathBuf,
    pub workspace: PathBuf,
    _server: Server,
    _root: tempfile::TempDir,
}
impl Fixture {
    pub fn new(binary: &Path) -> Result<Self> {
        let root = tempfile::Builder::new()
            .prefix("orbit-debug-fixture-")
            .tempdir()?;
        let home = root.path().join("home");
        let run = root.path().join("run");
        let work = root.path().join("workspace");
        for p in [&home, &run, &work] {
            fs::create_dir(p)?;
        }
        let socket = run.join("nexode.sock");
        let debug = run.join("nexode-debug.sock");
        let mut c = Command::new(binary);
        c.env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("HOME", &home)
            .env("NEXODE_HOME", &home)
            .env("NEXODE_RUNTIME_DIR", &run)
            .env("NEXODE_SOCKET", &socket)
            .env("NEXODE_DEBUG_CONTROL", "1")
            .env("NEXODE_NO_TELEMETRY", "1")
            .env("NEXODE_TEMP_SERVER", "1")
            .env("NEXODE_SERVER_OWNER_PID", std::process::id().to_string())
            .env("ANTHROPIC_API_KEY", "sk-ant-local-ui-fixture")
            .env("OPENAI_API_KEY", "sk-local-ui-fixture")
            .args(["--no-update", "--no-selfdev", "--socket"])
            .arg(&socket)
            .args(["serve", "--debug-socket"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0);
        let mut server = Server(c.spawn()?);
        wait_socket(&debug, &mut server, Duration::from_secs(15))?;
        Ok(Self {
            socket: debug,
            workspace: work,
            _server: server,
            _root: root,
        })
    }
}
pub(super) struct OwnedSession {
    pub socket: PathBuf,
    pub id: String,
    closed: bool,
}
impl OwnedSession {
    pub fn new(socket: &Path, cwd: &Path) -> Result<Self> {
        Ok(Self {
            socket: socket.into(),
            id: io::create_session(socket, cwd)?,
            closed: false,
        })
    }
    pub fn command(&self, command: &str) -> Result<String> {
        io::debug_session(
            &self.socket,
            command,
            Some(&self.id),
            Duration::from_secs(120),
        )
    }
    pub fn close(&mut self) -> Result<()> {
        if !self.closed {
            io::debug(
                &self.socket,
                &format!("destroy_session:{}", self.id),
                Duration::from_secs(10),
            )?;
            self.closed = true;
        }
        Ok(())
    }
}
impl Drop for OwnedSession {
    fn drop(&mut self) {
        if let Err(e) = self.close() {
            eprintln!("session cleanup failed: {e:#}");
        }
    }
}
