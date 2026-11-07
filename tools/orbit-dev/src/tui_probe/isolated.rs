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
    _server: Server,
    _root: tempfile::TempDir,
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
        Self::configured(binary, rows, cols, fresh, environment, None)
    }
    pub fn with_palette(binary: &Path, light: bool) -> Result<Self> {
        Self::configured(binary, 24, 80, false, &[], Some(light))
    }
    fn configured(
        binary: &Path,
        rows: u16,
        cols: u16,
        fresh: bool,
        environment: &[(&str, &str)],
        light: Option<bool>,
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
        if !fresh {
            launch.args(["--resume", &io::create_session(&debug, &work)?]);
        }
        launch
            .env("NEXODE_DEBUG_CMD_PATH", &cmd)
            .env("NEXODE_DEBUG_RESPONSE_PATH", &resp);
        let client = Client::spawn_palette(&mut launch, rows, cols, light)?;
        let session = Self {
            client,
            _server: server,
            _root: root,
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
    pub fn command(&self, command: &str) -> Result<String> {
        io::client_command(&self.cmd, &self.resp, command, Duration::from_secs(6))
    }
}
