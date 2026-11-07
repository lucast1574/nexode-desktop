use super::*;
use std::{
    net::TcpListener,
    os::unix::process::CommandExt,
    process::{Child, Stdio},
};
fn quote(text: impl AsRef<str>) -> String {
    format!("'{}'", text.as_ref().replace('\'', "'\\''"))
}
fn path_quote(path: &Path) -> String {
    quote(path.to_string_lossy())
}
fn env_args(env: &Environment) -> String {
    env.iter()
        .map(|(k, v)| quote(format!("{k}={v}")))
        .collect::<Vec<_>>()
        .join(" ")
}
fn executable_file(path: &Path, text: String) -> Result<()> {
    fs::write(path, text)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}
pub(super) fn wait<T>(
    mut predicate: impl FnMut() -> Result<Option<T>>,
    description: &str,
    timeout: u64,
) -> Result<T> {
    let deadline = Instant::now() + Duration::from_secs(timeout);
    while Instant::now() < deadline {
        ensure!(
            crate::ci_suites::unix::interrupted().is_none(),
            "acceptance interrupted"
        );
        if let Some(value) = predicate()? {
            return Ok(value);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    anyhow::bail!("Timed out waiting for {description}")
}
pub(super) fn records(path: &Path) -> Result<Vec<Value>> {
    let text = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(e.into()),
    };
    text.lines()
        .filter(|line| line.ends_with('}'))
        .map(serde_json::from_str::<Value>)
        .filter_map(|result| match result {
            Ok(v) if v["agent"] == true => Some(Ok(v)),
            Ok(_) => None,
            Err(e) => Some(Err(e.into())),
        })
        .collect()
}
pub(super) fn fresh_prompt(record: &Value, needle: &str) -> bool {
    record["messages"]
        .as_array()
        .and_then(|messages| messages.iter().rev().find(|m| m["role"] == "user"))
        .is_some_and(|m| m["content"].to_string().contains(needle))
}
pub(super) fn attached(frame: &str, state: &Value, sid: &str, cloud: bool) -> bool {
    frame
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with("server:"))
        .is_some_and(|header| header.starts_with("server: SSH ") == cloud)
        && state["remote_session_id"] == sid
}
pub(super) struct CloudTui {
    pub root: PathBuf,
    pub repo: PathBuf,
    pub cloud_repo: PathBuf,
    pub control: PathBuf,
    pub sid: Option<String>,
    binary: PathBuf,
    runtime: tempfile::TempDir,
    socket: PathBuf,
    env: Environment,
    children: Vec<Child>,
    provider: Option<crate::fake_openai::Server>,
    closed: bool,
}
impl CloudTui {
    pub fn new(binary: &Path, root: &Path) -> Result<Self> {
        fs::create_dir_all(root)?;
        let root = root.canonicalize()?;
        let runtime = tempfile::Builder::new().prefix("jca-").tempdir_in("/tmp")?;
        for side in ["local", "cloud"] {
            fs::create_dir(runtime.path().join(side))?;
        }
        for name in [
            "local-repo",
            "cloud-repo",
            "local-home",
            "cloud-home",
            "control",
            "ssh",
            "bin",
        ] {
            fs::create_dir(root.join(name))?;
        }
        let env = Environment::from([
            ("PATH".into(), "/usr/bin:/bin".into()),
            ("LANG".into(), "C.UTF-8".into()),
            ("TERM".into(), "xterm-256color".into()),
            (
                "HOME".into(),
                root.join("local-home").to_string_lossy().into_owned(),
            ),
            ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
            ("GIT_CONFIG_GLOBAL".into(), "/dev/null".into()),
            ("GIT_TERMINAL_PROMPT".into(), "0".into()),
            (
                "NEXODE_TEST_WORKSPACE".into(),
                root.to_string_lossy().into_owned(),
            ),
        ]);
        Ok(Self {
            binary: binary.into(),
            repo: root.join("local-repo"),
            cloud_repo: root.join("cloud-repo"),
            control: root.join("control"),
            socket: runtime.path().join("tmux.sock"),
            root,
            runtime,
            env,
            children: vec![],
            provider: None,
            sid: None,
            closed: false,
        })
    }
    fn run(&self, program: impl AsRef<std::ffi::OsStr>, args: &[&str]) -> Result<String> {
        checked(
            Command::new(program).args(args).env_clear().envs(&self.env),
            Duration::from_secs(20),
        )
    }
    pub fn tmux(&self, args: &[&str]) -> Result<String> {
        checked(
            Command::new("tmux")
                .arg("-S")
                .arg(&self.socket)
                .args(args)
                .env_clear()
                .envs(&self.env),
            Duration::from_secs(20),
        )
    }
    pub fn frame(&self) -> Result<String> {
        let frame = self.tmux(&["capture-pane", "-p", "-S", "-1000", "-t", "acceptance"])?;
        fs::write(self.root.join("frame.txt"), &frame)?;
        Ok(frame)
    }
    pub fn checkpoint(&self, message: &str) {
        println!(
            "NEXODE_CHECKPOINT {}",
            json!({"message":message,"scenario":self.root.file_name().unwrap_or_default().to_string_lossy()})
        );
    }
    pub fn wait_text(&self, text: &str) -> Result<()> {
        wait(
            || {
                let frame = self.frame()?;
                ensure!(
                    !frame.contains("NEXODE_ACCEPTANCE_EXIT="),
                    "TUI exited unexpectedly; see frame.txt"
                );
                Ok(frame.contains(text).then_some(()))
            },
            text,
            45,
        )?;
        let safe: String = text
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '_' })
            .take(50)
            .collect();
        fs::write(self.root.join(format!("frame-{safe}.txt")), self.frame()?)?;
        Ok(())
    }
    pub fn send(&self, text: &str, submit: bool) -> Result<()> {
        self.tmux(&["send-keys", "-t", "acceptance", "-l", text])?;
        if submit {
            self.tmux(&["send-keys", "-t", "acceptance", "Enter"])?;
        }
        Ok(())
    }
    pub fn records(&self) -> Result<Vec<Value>> {
        records(&self.root.join("model.jsonl"))
    }
    pub fn wait_request(&self, needle: &str, start: usize) -> Result<Value> {
        wait(
            || {
                Ok(self
                    .records()?
                    .into_iter()
                    .skip(start)
                    .find(|r| fresh_prompt(r, needle)))
            },
            &format!("fresh provider prompt containing {needle:?}"),
            45,
        )
    }
    pub fn lease(&self) -> Value {
        self.sid
            .as_ref()
            .and_then(|id| {
                fs::read(
                    self.root
                        .join("local-home/session_leases")
                        .join(format!("{id}.json")),
                )
                .ok()
            })
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or(json!({}))
    }
    pub fn discover_session(&mut self) -> Result<()> {
        self.sid = Some(wait(
            || {
                let path = self.root.join("local-home/sessions");
                if !path.exists() {
                    return Ok(None);
                }
                for entry in fs::read_dir(path)? {
                    let path = entry?.path();
                    if path.extension().is_some_and(|s| s == "json")
                        && let Ok(data) = serde_json::from_slice::<Value>(&fs::read(path)?)
                        && data["working_dir"] == self.repo.to_string_lossy().as_ref()
                        && data["messages"].as_array().is_some_and(|m| !m.is_empty())
                    {
                        return Ok(data["id"].as_str().map(String::from));
                    }
                }
                Ok(None)
            },
            "persisted source session",
            45,
        )?);
        Ok(())
    }
    pub fn session(&self, cloud: bool) -> Result<Value> {
        Ok(serde_json::from_slice(&fs::read(
            self.root
                .join(if cloud {
                    "cloud-home/sessions"
                } else {
                    "local-home/sessions"
                })
                .join(format!(
                    "{}.json",
                    self.sid.as_ref().context("session not discovered")?
                )),
        )?)?)
    }
    pub fn state(&self) -> Result<Value> {
        let response = self.control.join("debug.response");
        match fs::remove_file(&response) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        let temp = self.control.join("debug.next");
        fs::write(&temp, "state")?;
        fs::rename(temp, self.control.join("debug.cmd"))?;
        let state = wait(
            || {
                Ok(fs::read(&response)
                    .ok()
                    .and_then(|b| serde_json::from_slice::<Value>(&b).ok()))
            },
            "read-only client state",
            10,
        )?;
        fs::write(
            self.root.join("state.json"),
            serde_json::to_vec_pretty(&state)?,
        )?;
        Ok(state)
    }
    pub fn wait_idle(&self) -> Result<()> {
        wait(
            || {
                let state = self.state()?;
                Ok((state["processing"] == false && state["queued_messages"] == 0).then_some(()))
            },
            "idle client",
            45,
        )
    }
    pub fn wait_attachment(&self, cloud: bool) -> Result<()> {
        wait(
            || {
                let frame = self.frame()?;
                let header = frame
                    .lines()
                    .map(str::trim)
                    .find(|line| line.starts_with("server:"));
                if header.is_none_or(|h| h.starts_with("server: SSH ") != cloud) {
                    return Ok(None);
                }
                Ok(attached(
                    &frame,
                    &self.state()?,
                    self.sid.as_deref().context("missing session")?,
                    cloud,
                )
                .then_some(()))
            },
            if cloud {
                "cloud TUI reattachment"
            } else {
                "local TUI reattachment"
            },
            45,
        )
    }
    pub fn release(&self, token: &str) -> Result<()> {
        fs::write(self.control.join(format!("{token}.release")), "")?;
        Ok(())
    }
    pub fn held_turn(&mut self, token: &str, todo: bool) -> Result<()> {
        self.send(
            &format!(
                "{}[e2e:hold:{token}] continue this task",
                if todo { "[e2e:todo] " } else { "" }
            ),
            true,
        )?;
        wait(
            || {
                Ok(self
                    .control
                    .join(format!("{token}.held"))
                    .exists()
                    .then_some(()))
            },
            "held provider turn",
            45,
        )?;
        self.discover_session()
    }
    pub fn prepare_while_held(&self) -> Result<()> {
        self.send("/cloud", true)?;
        self.wait_text("destination ready")?;
        ensure!(
            self.lease()["away_host"].as_str().is_none_or(str::is_empty),
            "source ownership changed during active turn"
        );
        self.checkpoint("Destination prepared while source turn remains active");
        Ok(())
    }
    pub fn await_cloud(&self, start: usize) -> Result<()> {
        wait(
            || {
                ensure!(
                    !self.frame()?.contains("Pending cloud handoff canceled"),
                    "TUI canceled prepared handoff"
                );
                Ok((self.lease()["away_host"] == "cloud-acceptance").then_some(()))
            },
            "cloud ownership",
            45,
        )?;
        self.wait_request("[nexode cloud]", start)?;
        self.wait_text("saw_cloud_notice=True")?;
        self.wait_attachment(true)?;
        self.wait_idle()?;
        ensure!(
            !self.frame()?.contains("Pending cloud handoff canceled"),
            "handoff canceled"
        );
        ensure!(
            fs::read_to_string(self.cloud_repo.join("file.txt"))?
                .contains("local uncommitted edit"),
            "workspace edit lost"
        );
        self.checkpoint("Destination agent continued with transferred workspace");
        Ok(())
    }
    pub fn go_cloud(&self, session_only: bool) -> Result<()> {
        self.wait_idle()?;
        let start = self.records()?.len();
        self.send(
            if session_only {
                "/cloud --session-only"
            } else {
                "/cloud"
            },
            true,
        )?;
        self.await_cloud(start)
    }
    pub fn go_local(&self) -> Result<()> {
        let start = self.records()?.len();
        self.send("/local", true)?;
        wait(
            || {
                let lease = self.lease();
                Ok((lease.as_object().is_some_and(|m| !m.is_empty())
                    && lease["away_host"].as_str().is_none_or(str::is_empty))
                .then_some(()))
            },
            "local ownership",
            45,
        )?;
        self.wait_attachment(false)?;
        self.wait_idle()?;
        self.send("acceptance back on laptop", true)?;
        self.wait_request("acceptance back on laptop", start)?;
        self.wait_idle()?;
        self.wait_text("saw_cloud_notice=True")?;
        ensure!(
            self.session(false)?
                .to_string()
                .contains("returned from cloud host"),
            "missing return history notice"
        );
        self.checkpoint("Source agent answered a fresh prompt after return");
        Ok(())
    }
    pub fn start(&mut self) -> Result<()> {
        let repo = self.repo.to_str().context("repo path encoding")?;
        self.run("git", &["init", "-q", "-b", "main", repo])?;
        self.run(
            "git",
            &[
                "-C",
                repo,
                "config",
                "user.name",
                "Cloud acceptance fixture",
            ],
        )?;
        self.run(
            "git",
            &[
                "-C",
                repo,
                "config",
                "user.email",
                "fixture@example.invalid",
            ],
        )?;
        fs::write(self.repo.join("file.txt"), "base\n")?;
        self.run("git", &["-C", repo, "add", "."])?;
        self.run("git", &["-C", repo, "commit", "-qm", "fixture"])?;
        fs::write(self.repo.join("file.txt"), "base\nlocal uncommitted edit\n")?;
        let provider = crate::fake_openai::Server::start(
            0,
            crate::fake_openai::Options {
                log: self.root.join("model.jsonl"),
                label: "model".into(),
                control: Some(self.control.clone()),
            },
        )?;
        let mut model = Environment::from([
            (
                "NEXODE_OPENAI_COMPAT_API_BASE".into(),
                format!("http://127.0.0.1:{}/v1", provider.port),
            ),
            (
                "OPENAI_COMPAT_API_KEY".into(),
                "synthetic-acceptance-key".into(),
            ),
            (
                "NEXODE_OPENAI_COMPAT_DEFAULT_MODEL".into(),
                "fake-model".into(),
            ),
            ("NEXODE_NO_TELEMETRY".into(), "1".into()),
            ("NEXODE_TELEMETRY".into(), "off".into()),
            ("NEXODE_NO_AUTO_UPDATE".into(), "1".into()),
        ]);
        self.provider = Some(provider);
        for key in ["host_key", "client_key"] {
            checked(
                Command::new("ssh-keygen")
                    .args(["-q", "-t", "ed25519", "-N", "", "-f"])
                    .arg(self.root.join("ssh").join(key))
                    .env_clear()
                    .envs(&self.env),
                Duration::from_secs(20),
            )?;
        }
        fs::copy(
            self.root.join("ssh/client_key.pub"),
            self.root.join("ssh/authorized_keys"),
        )?;
        let port = TcpListener::bind("127.0.0.1:0")?.local_addr()?.port();
        let public = fs::read_to_string(self.root.join("ssh/host_key.pub"))?;
        let key = public
            .split_whitespace()
            .take(2)
            .collect::<Vec<_>>()
            .join(" ");
        fs::write(
            self.root.join("ssh/known_hosts"),
            format!("[127.0.0.1]:{port} {key}\n"),
        )?;
        let mut cloud = self.env.clone();
        cloud.extend(model.clone());
        for (k, v) in [
            ("HOME", self.root.join("cloud-home")),
            ("NEXODE_HOME", self.root.join("cloud-home")),
            ("NEXODE_RUNTIME_DIR", self.runtime.path().join("cloud")),
        ] {
            cloud.insert(k.into(), v.to_string_lossy().into_owned());
        }
        cloud.insert("NEXODE_CLOUD_HOST_LABEL".into(), "cloud-acceptance".into());
        let mount = format!(
            "mount --bind {} {}; exec /bin/bash -c \"$SSH_ORIGINAL_COMMAND\"",
            path_quote(&self.cloud_repo),
            path_quote(&self.repo)
        );
        let shell = self.root.join("cloud-shell");
        executable_file(
            &shell,
            format!(
                "#!/bin/bash\nset -e\nif [[ -e {} ]]; then echo 'Injected acceptance SSH outage' >&2; exit 43; fi\nexec env -i {} \"SSH_ORIGINAL_COMMAND=$SSH_ORIGINAL_COMMAND\" unshare --user --map-root-user --mount /bin/bash -ec {}\n",
                path_quote(&self.control.join("ssh-fail")),
                env_args(&cloud),
                quote(mount)
            ),
        )?;
        fs::write(
            self.root.join("ssh/sshd_config"),
            format!(
                "Port {port}\nListenAddress 127.0.0.1\nHostKey {0}/ssh/host_key\nPidFile {0}/ssh/sshd.pid\nAuthorizedKeysFile {0}/ssh/authorized_keys\nPasswordAuthentication no\nKbdInteractiveAuthentication no\nUsePAM no\nStrictModes no\nForceCommand {1}\nLogLevel VERBOSE\n",
                self.root.display(),
                shell.display()
            ),
        )?;
        let log = fs::File::create(self.root.join("sshd.log"))?;
        self.children.push(
            Command::new(super::sshd().context("sshd not found")?)
                .args(["-D", "-e", "-f"])
                .arg(self.root.join("ssh/sshd_config"))
                .env_clear()
                .envs(&self.env)
                .stdin(Stdio::null())
                .stdout(log.try_clone()?)
                .stderr(log)
                .process_group(0)
                .spawn()?,
        );
        let username = self.run("id", &["-un"])?;
        let config = format!(
            "Host cloud-acceptance\n HostName 127.0.0.1\n Port {port}\n User {}\n IdentityFile {}/ssh/client_key\n IdentitiesOnly yes\n StrictHostKeyChecking yes\n UserKnownHostsFile {}/ssh/known_hosts\n ConnectTimeout 3\n BatchMode yes\n",
            username.trim(),
            self.root.display(),
            self.root.display()
        );
        fs::write(self.root.join("ssh/config"), config)?;
        let ssh = self.root.join("bin/ssh");
        executable_file(
            &ssh,
            format!(
                "#!/bin/bash\nexec /usr/bin/ssh -F {} \"$@\"\n",
                path_quote(&self.root.join("ssh/config"))
            ),
        )?;
        let remote = self.root.join("bin/remote-nexode");
        executable_file(
            &remote,
            format!(
                "#!/bin/bash\nif [[ -e {} ]]; then echo 'injected missing destination binary' >&2; exit 127; fi\nif [[ -e {} ]]; then\n if [[ \"$*\" == *--version* ]]; then echo 'nexode legacy acceptance fixture'; exit 0; fi\n echo 'unrecognized cloud protocol command' >&2; exit 2\nfi\nexec {} \"$@\"\n",
                path_quote(&self.control.join("binary-missing")),
                path_quote(&self.control.join("binary-legacy")),
                path_quote(&self.binary)
            ),
        )?;
        let mut local = self.env.clone();
        local.append(&mut model);
        for (key, path) in [
            ("HOME", self.root.join("local-home")),
            ("NEXODE_HOME", self.root.join("local-home")),
            ("NEXODE_RUNTIME_DIR", self.runtime.path().join("local")),
            ("NEXODE_CLOUD_REMOTE_BINARY", remote),
            ("NEXODE_DEBUG_CMD_PATH", self.control.join("debug.cmd")),
            (
                "NEXODE_DEBUG_RESPONSE_PATH",
                self.control.join("debug.response"),
            ),
        ] {
            local.insert(key.into(), path.to_string_lossy().into_owned());
        }
        local.insert(
            "PATH".into(),
            format!("{}/bin:/usr/bin:/bin", self.root.display()),
        );
        local.insert("NEXODE_CLOUD_HOST".into(), "cloud-acceptance".into());
        local.insert("NEXODE_CLOUD_HOST_LABEL".into(), "laptop-acceptance".into());
        wait(
            || {
                let output = crate::process::capture(
                    Command::new(&ssh)
                        .args(["cloud-acceptance", "printf ready"])
                        .env_clear()
                        .envs(&local),
                    Duration::from_secs(5),
                )?;
                Ok(output.status.success().then_some(()))
            },
            "isolated real SSH host",
            15,
        )?;
        self.checkpoint("Isolated SSH host is ready");
        let command = format!(
            "cd {} && env -i {} {} --no-update --no-selfdev --provider openai-compatible --model fake-model; result=$?; printf '\\nNEXODE_ACCEPTANCE_EXIT=%s\\n' \"$result\"; sleep 600",
            path_quote(&self.repo),
            env_args(&local),
            path_quote(&self.binary)
        );
        let command = format!("/bin/bash -c {}", quote(command));
        self.tmux(&[
            "-f",
            "/dev/null",
            "new-session",
            "-d",
            "-s",
            "acceptance",
            "-x",
            "160",
            "-y",
            "48",
            &command,
        ])?;
        wait(
            || {
                let frame = self.frame()?;
                ensure!(
                    !frame.contains("NEXODE_ACCEPTANCE_EXIT="),
                    "TUI exited during startup; see frame.txt"
                );
                Ok(
                    (frame.contains("Welcome to nexode") || frame.contains("Ready to use"))
                        .then_some(()),
                )
            },
            "TUI startup",
            45,
        )?;
        if self.frame()?.contains("Welcome to nexode") {
            self.tmux(&["send-keys", "-t", "acceptance", "Right", "Enter"])?;
        }
        self.wait_text("Ready to use")?;
        self.checkpoint("Real TUI is ready");
        Ok(())
    }
    pub fn close(&mut self) -> Result<()> {
        if self.closed {
            return Ok(());
        }
        self.closed = true;
        let _ = self.frame();
        let _ = crate::process::capture(
            Command::new("tmux")
                .arg("-S")
                .arg(&self.socket)
                .arg("kill-server")
                .env_clear()
                .envs(&self.env),
            Duration::from_secs(5),
        );
        for side in ["local", "cloud"] {
            let _ = crate::process::capture(
                Command::new(&self.binary)
                    .args(["--no-update", "server", "stop"])
                    .env_clear()
                    .envs(&self.env)
                    .env("HOME", self.root.join(format!("{side}-home")))
                    .env("NEXODE_HOME", self.root.join(format!("{side}-home")))
                    .env("NEXODE_RUNTIME_DIR", self.runtime.path().join(side)),
                Duration::from_secs(10),
            );
        }
        for child in self.children.iter_mut().rev() {
            unsafe {
                libc::kill(-(child.id() as i32), libc::SIGTERM);
            }
            let deadline = Instant::now() + Duration::from_secs(5);
            while Instant::now() < deadline && child.try_wait()?.is_none() {
                std::thread::sleep(Duration::from_millis(20));
            }
            unsafe {
                libc::kill(-(child.id() as i32), libc::SIGKILL);
            }
            child.wait()?;
        }
        let cleanup = crate::detached_acceptance::cleanup_workspace(&self.root);
        self.provider.take();
        cleanup
    }
}
impl Drop for CloudTui {
    fn drop(&mut self) {
        if let Err(e) = self.close() {
            eprintln!("cloud fixture cleanup: {e:#}");
        }
    }
}

#[cfg(test)]
mod isolation_tests {
    use super::*;
    #[test]
    fn long_artifact_paths_keep_unix_sockets_short() {
        let dir = tempfile::tempdir().unwrap();
        let mut fixture = CloudTui::new(
            Path::new("/bin/true"),
            &dir.path().join("long-artifacts-".repeat(12)),
        )
        .unwrap();
        assert!(fixture.socket.as_os_str().len() < 100);
        let listener = std::os::unix::net::UnixListener::bind(&fixture.socket).unwrap();
        drop(listener);
        fixture.close().unwrap();
    }
    #[test]
    fn cleanup_reaps_only_exact_workspace_marker() {
        let dir = tempfile::tempdir().unwrap();
        let mut fixture =
            CloudTui::new(Path::new("/bin/true"), &dir.path().join("fixture")).unwrap();
        let mut owned = Command::new("sleep")
            .arg("60")
            .env("NEXODE_TEST_WORKSPACE", &fixture.root)
            .spawn()
            .unwrap();
        let mut other = Command::new("sleep")
            .arg("60")
            .env(
                "NEXODE_TEST_WORKSPACE",
                format!("{}-other", fixture.root.display()),
            )
            .spawn()
            .unwrap();
        let result = fixture.close();
        let deadline = Instant::now() + Duration::from_secs(2);
        let owned_done = loop {
            if owned.try_wait().unwrap().is_some() {
                break true;
            }
            if Instant::now() >= deadline {
                break false;
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        let other_alive = other.try_wait().unwrap().is_none();
        let _ = owned.kill();
        let _ = other.kill();
        let _ = owned.wait();
        let _ = other.wait();
        result.unwrap();
        assert!(owned_done);
        assert!(other_alive, "neighboring fixture was killed");
    }
}
