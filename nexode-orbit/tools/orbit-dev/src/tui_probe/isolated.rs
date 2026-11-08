//! Shared disposable real-server UI fixture. Never reuses the user's daemon/home.
use super::{
    io,
    screen::Client,
    single::{Server, wait_socket},
};
use anyhow::{Result, ensure};
use std::{
    fs,
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
pub(super) struct Session {
    pub client: Client,
    pub launched: Instant,
    _server: Option<Server>,
    _root: Option<tempfile::TempDir>,
    pub session_id: Option<String>,
    cmd: PathBuf,
    resp: PathBuf,
}
pub(super) fn pause(seconds: f64) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs_f64(seconds);
    while Instant::now() < deadline {
        ensure!(
            crate::ci_suites::unix::interrupted().is_none(),
            "UI probe interrupted"
        );
        std::thread::sleep(
            (deadline - Instant::now().min(deadline)).min(Duration::from_millis(20)),
        );
    }
    Ok(())
}
impl Session {
    pub fn new(binary: &Path, rows: u16, cols: u16, fresh: bool) -> Result<Self> {
        Self::with_environment(binary, rows, cols, fresh, &[])
    }
    pub fn with_environment(
        binary: &Path,
        rows: u16,
        cols: u16,
        fresh: bool,
        environment: &[(&str, &str)],
    ) -> Result<Self> {
        Self::configured(binary, rows, cols, fresh, environment, None, false)
    }
    pub fn with_palette(binary: &Path, light: bool) -> Result<Self> {
        Self::configured(binary, 24, 80, false, &[], Some(light), false)
    }
    pub fn returning(
        binary: &Path,
        rows: u16,
        cols: u16,
        environment: &[(&str, &str)],
    ) -> Result<Self> {
        Self::configured(binary, rows, cols, false, environment, None, true)
    }
    fn configured(
        binary: &Path,
        rows: u16,
        cols: u16,
        fresh: bool,
        environment: &[(&str, &str)],
        light: Option<bool>,
        returning: bool,
    ) -> Result<Self> {
        ensure!(
            rows > 0 && cols > 0 && rows <= 512 && cols <= 1024,
            "terminal dimensions out of range"
        );
        let root = tempfile::Builder::new()
            .prefix("orbit-ui-probe-")
            .tempdir()?;
        let home = root.path().join("home");
        let runtime = root.path().join("run");
        let work = root.path().join("workspace");
        for p in [&home, &runtime, &work] {
            fs::create_dir(p)?;
        }
        if returning {
            fs::write(
                home.join("setup_hints.json"),
                br#"{"launch_count":10,"onboarding_start_choice_shown":true}"#,
            )?;
        }
        let socket = runtime.join("nexode.sock");
        let debug = runtime.join("nexode-debug.sock");
        let cmd = runtime.join("client_cmd");
        let resp = runtime.join("client_resp");
        let command = || {
            let mut c = Command::new(binary);
            c.env_clear()
                .env("PATH", std::env::var_os("PATH").unwrap_or_default())
                .env("HOME", &home)
                .env("NEXODE_HOME", &home)
                .env("NEXODE_RUNTIME_DIR", &runtime)
                .env("NEXODE_SOCKET", &socket)
                .env("NEXODE_NO_TELEMETRY", "1")
                .env("NEXODE_DEBUG_CONTROL", "1")
                .env("NEXODE_TEMP_SERVER", "1")
                .env("NEXODE_SERVER_OWNER_PID", std::process::id().to_string())
                .env("NEXODE_PERF_TIER", "full")
                .env("NEXODE_THEME", "dark")
                .env("TERM", "xterm-256color")
                .env("ANTHROPIC_API_KEY", "sk-ant-local-ui-fixture")
                .envs(environment.iter().copied())
                .current_dir(&work)
                .args(["--no-update", "--no-selfdev", "--socket"])
                .arg(&socket);
            if light.is_some() {
                c.env_remove("NEXODE_THEME").env("COLORTERM", "truecolor");
            }
            c
        };
        let mut launch = command();
        launch
            .args(["serve", "--debug-socket"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::from(fs::File::create(
                root.path().join("server.log"),
            )?))
            .process_group(0);
        let mut server = Server(launch.spawn()?);
        wait_socket(&socket, &mut server, Duration::from_secs(15))?;
        wait_socket(&debug, &mut server, Duration::from_secs(15))?;
        let mut launch = command();
        let session_id = if fresh {
            None
        } else {
            Some(io::create_session(&debug, &work)?)
        };
        if let Some(id) = &session_id {
            launch.args(["--resume", id]);
        }
        launch
            .env("NEXODE_DEBUG_CMD_PATH", &cmd)
            .env("NEXODE_DEBUG_RESPONSE_PATH", &resp);
        let launched = Instant::now();
        let client = Client::spawn_palette(&mut launch, rows, cols, light)?;
        let session = Self {
            client,
            launched,
            _server: Some(server),
            _root: Some(root),
            session_id,
            cmd,
            resp,
        };
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            if session.command("input").is_ok() {
                break;
            }
            ensure!(
                Instant::now() < deadline && !session.client.snapshot()?.eof,
                "client never became ready"
            );
            pause(0.1)?;
        }
        pause(3.0)?;
        Ok(session)
    }
    pub fn retain_artifacts(&mut self) -> Option<PathBuf> {
        self._root.take().map(|root| root.keep())
    }
    pub fn live(binary: &Path, work: &Path, socket: &Path, debug: &Path) -> Result<Self> {
        let session_id = io::create_session(debug, work)?;
        Self::resume(binary, work, socket, &session_id, 48, 160)
    }
    pub fn resume(
        binary: &Path,
        work: &Path,
        socket: &Path,
        session_id: &str,
        rows: u16,
        cols: u16,
    ) -> Result<Self> {
        ensure!(
            rows > 0 && cols > 0 && rows <= 512 && cols <= 1024,
            "invalid terminal size"
        );
        let root = tempfile::Builder::new()
            .prefix("orbit-live-probe-")
            .tempdir()?;
        let cmd = root.path().join("cmd");
        let resp = root.path().join("resp");
        let mut launch = Command::new(binary);
        launch
            .current_dir(work)
            .args(["--no-update", "--no-selfdev", "--socket"])
            .arg(socket)
            .args(["--resume", session_id])
            .env("NEXODE_DEBUG_CONTROL", "1")
            .env("NEXODE_DEBUG_CMD_PATH", &cmd)
            .env("NEXODE_DEBUG_RESPONSE_PATH", &resp)
            .env("NEXODE_THEME", "dark")
            .env("TERM", "xterm-256color");
        let launched = Instant::now();
        let client = Client::spawn(&mut launch, rows, cols)?;
        let session = Self {
            client,
            launched,
            _server: None,
            _root: Some(root),
            session_id: Some(session_id.to_owned()),
            cmd,
            resp,
        };
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            if session.command("input").is_ok() {
                break;
            }
            ensure!(
                Instant::now() < deadline && !session.client.snapshot()?.eof,
                "live client never became ready"
            );
            pause(0.1)?;
        }
        Ok(session)
    }
    pub fn command(&self, command: &str) -> Result<String> {
        io::client_command(&self.cmd, &self.resp, command, Duration::from_secs(6))
    }
}
