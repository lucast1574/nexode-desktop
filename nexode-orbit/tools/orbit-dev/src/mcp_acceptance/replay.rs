use super::*;
use std::{
    collections::BTreeMap,
    os::unix::{fs::PermissionsExt, process::CommandExt},
    process::{Child, Command, Stdio},
};
pub(super) struct Replay<'a> {
    binary: &'a Path,
    root: &'a Path,
    pub home: PathBuf,
    pub fixture: &'a Fixture,
    env: BTreeMap<String, String>,
    browser_marker: PathBuf,
}
struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            unsafe {
                libc::kill(-(self.0.id() as i32), libc::SIGKILL);
            }
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}
impl<'a> Replay<'a> {
    pub fn new(binary: &'a Path, root: &'a Path, fixture: &'a Fixture) -> Result<Self> {
        let home = root.join("nexode-home");
        let guards = root.join("browser-guards");
        for path in [
            root.join("home"),
            home.clone(),
            guards.clone(),
            root.join("runtime"),
        ] {
            fs::create_dir(&path)?;
            fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
        }
        let mut env = BTreeMap::new();
        for (key, value) in [
            ("HOME", root.join("home")),
            ("NEXODE_HOME", home.clone()),
            ("XDG_CONFIG_HOME", root.join("xdg-config")),
            ("XDG_CACHE_HOME", root.join("xdg-cache")),
            ("XDG_DATA_HOME", root.join("xdg-data")),
            ("XDG_RUNTIME_DIR", root.join("runtime")),
        ] {
            env.insert(key.into(), value.to_string_lossy().into());
        }
        env.insert("PATH".into(), format!("{}:/usr/bin:/bin", guards.display()));
        for (key, value) in [
            ("LANG", "C.UTF-8"),
            ("TERM", "dumb"),
            ("NEXODE_DISABLE_CLAUDE_MCP", "1"),
            ("NO_PROXY", "127.0.0.1,localhost"),
            ("NEXODE_NO_TELEMETRY", "1"),
            ("NEXODE_NO_AUTO_UPDATE", "1"),
        ] {
            env.insert(key.into(), value.into());
        }
        let browser_marker = root.join("browser-invoked");
        env.insert(
            "MCP_BROWSER_MARKER".into(),
            browser_marker.to_string_lossy().into(),
        );
        for name in [
            "xdg-open",
            "gio",
            "sensible-browser",
            "firefox",
            "chromium",
            "google-chrome",
        ] {
            let path = guards.join(name);
            fs::write(
                &path,
                "#!/bin/sh\nprintf called >> \"$MCP_BROWSER_MARKER\"\nexit 91\n",
            )?;
            fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
        }
        env.insert(
            "BROWSER".into(),
            guards.join("xdg-open").to_string_lossy().into(),
        );
        let replay = Self {
            binary,
            root,
            home,
            fixture,
            env,
            browser_marker,
        };
        replay.config(
            json!({"remote":{"type":"http","url":format!("{}/mcp",fixture.base),"enabled":false}}),
        )?;
        Ok(replay)
    }
    fn process(&self, args: &[&str]) -> Command {
        let mut command = Command::new(self.binary);
        command
            .arg("mcp")
            .args(args)
            .current_dir(self.root)
            .env_clear()
            .envs(&self.env);
        command
    }
    pub fn config(&self, servers: Value) -> Result<()> {
        fs::write(
            self.home.join("mcp.json"),
            serde_json::to_vec(&json!({"servers":servers}))?,
        )?;
        Ok(())
    }
    pub fn command(&self, args: &[&str], success: bool, timeout: u64) -> Result<String> {
        let output =
            crate::process::capture(&mut self.process(args), Duration::from_secs(timeout))?;
        ensure!(
            output.status.success() == success,
            "CLI exit status mismatch"
        );
        ensure!(
            !self.browser_marker.exists(),
            "CLI attempted browser launch"
        );
        let mut text = String::from_utf8(output.stdout)?;
        text.push_str(&String::from_utf8(output.stderr)?);
        Ok(text)
    }
    pub fn login(&self) -> Result<()> {
        let stdout = tempfile::NamedTempFile::new_in(self.root)?;
        let stderr = tempfile::NamedTempFile::new_in(self.root)?;
        let mut command = self.process(&["login", "remote", "--no-browser"]);
        command
            .process_group(0)
            .stdin(Stdio::null())
            .stdout(stdout.reopen()?)
            .stderr(stderr.reopen()?);
        let mut child = ChildGuard(command.spawn()?);
        let regex = regex::Regex::new(r"http://127\.0\.0\.1:\d+/authorize\?\S+")?;
        let deadline = Instant::now() + Duration::from_secs(15);
        let url = loop {
            let text = format!(
                "{}\n{}",
                fs::read_to_string(stdout.path())?,
                fs::read_to_string(stderr.path())?
            );
            if let Some(found) = regex.find(&text) {
                break found.as_str().to_owned();
            }
            ensure!(
                child.0.try_wait()?.is_none() && Instant::now() < deadline,
                "CLI did not emit authorization URL before deadline"
            );
            std::thread::sleep(Duration::from_millis(50));
        };
        ensure!(
            url.starts_with(&format!("{}/authorize?", self.fixture.base)),
            "authorization endpoint is not fixture"
        );
        let client = reqwest::blocking::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                if attempt.previous().len() <= 1
                    && attempt.url().scheme() == "http"
                    && attempt.url().host_str() == Some("127.0.0.1")
                    && attempt.url().path() == "/callback"
                {
                    attempt.follow()
                } else {
                    attempt.stop()
                }
            }))
            .build()?;
        ensure!(
            client.get(url).send()?.status().as_u16() == 200,
            "callback rejected"
        );
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(status) = child.0.try_wait()? {
                ensure!(status.success(), "login CLI failed");
                break;
            }
            ensure!(Instant::now() < deadline, "login CLI did not exit");
            std::thread::sleep(Duration::from_millis(20));
        }
        ensure!(
            !self.browser_marker.exists(),
            "no-browser launched a browser"
        );
        Ok(())
    }
}
