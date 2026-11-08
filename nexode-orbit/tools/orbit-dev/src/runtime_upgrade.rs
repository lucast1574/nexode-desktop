//! Real exec/reconnect acceptance, isolated from host credentials and network.
use anyhow::{Context, Result, ensure};
use clap::Parser;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::{
        fs::{DirBuilderExt, PermissionsExt},
        net::UnixStream,
        process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

#[derive(Parser)]
struct Args {
    #[arg(long)]
    binary: PathBuf,
    #[arg(long)]
    artifacts: Option<PathBuf>,
}
struct Wire(BufReader<UnixStream>);
impl Wire {
    fn connect(path: &Path) -> Result<Self> {
        let socket = UnixStream::connect(path)?;
        socket.set_write_timeout(Some(Duration::from_secs(15)))?;
        Ok(Self(BufReader::new(socket)))
    }
    fn send(&mut self, value: Value) -> Result<()> {
        writeln!(self.0.get_mut(), "{value}")?;
        Ok(())
    }
    fn line(&mut self, deadline: Instant) -> Result<Vec<u8>> {
        let mut line = Vec::new();
        loop {
            ensure!(
                crate::ci_suites::unix::interrupted().is_none(),
                "interrupted"
            );
            ensure!(Instant::now() < deadline, "wire response timed out");
            self.0.get_ref().set_read_timeout(Some(
                deadline
                    .saturating_duration_since(Instant::now())
                    .max(Duration::from_millis(1)),
            ))?;
            let bytes = self.0.fill_buf()?;
            if bytes.is_empty() {
                return Ok(line);
            }
            let n = bytes
                .iter()
                .position(|b| *b == b'\n')
                .map_or(bytes.len(), |n| n + 1);
            ensure!(line.len() + n <= 1024 * 1024, "wire response exceeds 1 MiB");
            line.extend_from_slice(&bytes[..n]);
            self.0.consume(n);
            if line.last() == Some(&b'\n') {
                return Ok(line);
            }
        }
    }
    fn until(&mut self, kind: &str, id: Option<u64>) -> Result<Value> {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let line = self.line(deadline)?;
            ensure!(!line.is_empty(), "disconnected waiting for {kind}");
            let event: Value = serde_json::from_slice(&line)?;
            ensure!(event["type"] != "error", "native error: {event}");
            if event["type"] == kind && id.is_none_or(|id| event["id"] == id) {
                return Ok(event);
            }
        }
    }
    fn closed(&mut self) -> Result<()> {
        ensure!(
            self.line(Instant::now() + Duration::from_secs(15))?
                .is_empty(),
            "exec must close old connection"
        );
        Ok(())
    }
}
fn wait<T>(mut poll: impl FnMut() -> Result<Option<T>>) -> Result<T> {
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut last = None;
    while Instant::now() < deadline {
        ensure!(
            crate::ci_suites::unix::interrupted().is_none(),
            "interrupted"
        );
        match poll() {
            Ok(Some(value)) => return Ok(value),
            Ok(None) => {}
            Err(e) => last = Some(e),
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    anyhow::bail!("readiness timeout: {last:?}")
}
fn private_dir(path: &Path) -> Result<()> {
    fs::DirBuilder::new().mode(0o700).create(path)?;
    Ok(())
}
struct Daemon {
    root: PathBuf,
    socket: PathBuf,
    debug: PathBuf,
    child: Child,
}
impl Drop for Daemon {
    fn drop(&mut self) {
        unsafe {
            libc::kill(-(self.child.id() as i32), libc::SIGTERM);
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if self.child.try_wait().ok().flatten().is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        unsafe {
            libc::kill(-(self.child.id() as i32), libc::SIGKILL);
        }
        let _ = self.child.wait();
    }
}
impl Daemon {
    fn start(binary: &Path, root: PathBuf, version: &str) -> Result<Self> {
        private_dir(&root)?;
        for name in [
            "home", "data", "runtime", "config", "cache", "state", "work", "install",
        ] {
            private_dir(&root.join(name))?;
        }
        fs::write(root.join("data/no_telemetry"), "")?;
        let socket = root.join("runtime/nexode.sock");
        let debug = root.join("runtime/nexode-debug.sock");
        let log = fs::File::create(root.join("daemon.log"))?;
        let mut cmd = Command::new("/usr/bin/unshare");
        cmd.args(["--user", "--map-root-user", "--net"])
            .arg(binary)
            .arg("--no-update")
            .arg("--socket")
            .arg(&socket)
            .arg("serve")
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("LANG", "C.UTF-8")
            .env("SHELL", "/bin/bash")
            .env("NEXODE_SOCKET", &socket)
            .env("NEXODE_RUNTIME_RELEASE_SEMVER", version)
            .current_dir(root.join("work"))
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log)
            .process_group(0);
        for (key, dir) in [
            ("HOME", "home"),
            ("NEXODE_HOME", "data"),
            ("NEXODE_RUNTIME_DIR", "runtime"),
            ("XDG_RUNTIME_DIR", "runtime"),
            ("XDG_CONFIG_HOME", "config"),
            ("XDG_CACHE_HOME", "cache"),
            ("XDG_STATE_HOME", "state"),
            ("XDG_DATA_HOME", "data"),
            ("NEXODE_INSTALL_DIR", "install"),
        ] {
            cmd.env(key, root.join(dir));
        }
        for (key, value) in [
            ("NEXODE_NO_AUTO_UPDATE", "1"),
            ("NEXODE_DEBUG_CONTROL", "1"),
            ("NEXODE_MEMORY_ENABLED", "0"),
            ("NEXODE_MEMORY_SIDECAR_ENABLED", "0"),
            ("NEXODE_NON_INTERACTIVE", "1"),
        ] {
            cmd.env(key, value);
        }
        let daemon = Self {
            child: cmd.spawn()?,
            root,
            socket,
            debug,
        };
        wait(|| {
            let mut wire = Wire::connect(&daemon.socket)?;
            wire.send(json!({"type":"ping","id":1}))?;
            Ok(Some(wire.until("pong", Some(1))?))
        })
        .with_context(|| fs::read_to_string(daemon.root.join("daemon.log")).unwrap_or_default())?;
        Ok(daemon)
    }
    fn info(&self) -> Result<Value> {
        let mut wire = Wire::connect(&self.debug)?;
        wire.send(json!({"type":"debug_command","id":2,"command":"server:info"}))?;
        let result = wire.until("debug_response", Some(2))?;
        ensure!(result["ok"] == true, "server:info failed: {result}");
        Ok(serde_json::from_str(
            result["output"].as_str().context("missing server info")?,
        )?)
    }
    fn upgrade(&self, candidate: &Path) -> Result<Value> {
        let mut wire = Wire::connect(&self.socket)?;
        wire.send(json!({"type":"upgrade_runtime","id":11,"candidate":candidate}))?;
        let result = wire.until("reload_progress", None)?;
        if result["step"] == "upgrading" {
            wire.closed()?;
        }
        Ok(result)
    }
}
pub fn run(args: impl Iterator<Item = String>) -> Result<()> {
    let args = Args::parse_from(std::iter::once("runtime-upgrade-acceptance".into()).chain(args));
    let _signals = crate::ci_suites::unix::Signals::install()?;
    let binary = args.binary.canonicalize()?;
    ensure!(
        binary.is_file() && fs::metadata(&binary)?.permissions().mode() & 0o111 != 0,
        "binary must be executable"
    );
    let artifacts = if let Some(path) = args.artifacts {
        private_dir(&path)?;
        path.canonicalize()?
    } else {
        let scratch = std::env::var_os("NEXODE_SCRATCH_DIR")
            .map(PathBuf::from)
            .unwrap_or(
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .context("HOME required")?
                    .join(".nexode/scratch"),
            );
        fs::create_dir_all(&scratch)?;
        tempfile::Builder::new()
            .prefix("upg-")
            .tempdir_in(scratch)?
            .keep()
    };
    ensure!(
        artifacts
            .join("downgrade/runtime/nexode-debug.sock")
            .as_os_str()
            .len()
            < 104,
        "artifact root too long for Unix socket"
    );
    let output = crate::process::capture(
        Command::new("/usr/bin/unshare").args(["--user", "--map-root-user", "--net", "/bin/true"]),
        Duration::from_secs(10),
    )?;
    ensure!(
        output.status.success(),
        "private network namespace unavailable: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut file = fs::File::open(&binary)?;
    let mut digest = Sha256::new();
    std::io::copy(&mut file, &mut digest)?;
    let mut evidence = json!({"binary":binary,"sha256":format!("{:x}",digest.finalize())});
    exercise(&binary, &artifacts, &mut evidence)?;
    fs::write(
        artifacts.join("acceptance.json"),
        format!("{}\n", serde_json::to_string_pretty(&evidence)?),
    )?;
    println!(
        "{}",
        json!({"result":"passed","artifacts":artifacts,"session":evidence["session_id"],"checks":["private network namespace","credential-free environment","invalid/native-nonnexode refusal","busy refusal","uninterrupted shell","real exec/reconnect","history exact preservation","same-version refusal","downgrade refusal"]})
    );
    Ok(())
}
fn exercise(binary: &Path, artifacts: &Path, evidence: &mut Value) -> Result<()> {
    let mut daemon = Daemon::start(binary, artifacts.join("upgrade"), "0.0.1")?;
    let before = daemon.info()?;
    evidence["before"] = before.clone();
    let mut client = Wire::connect(&daemon.socket)?;
    client.send(json!({"type":"subscribe","id":30,"working_dir":daemon.root.join("work")}))?;
    client.send(json!({"type":"input_shell","id":3,"command":"readlink /proc/self/ns/net"}))?;
    let namespace = client.until("input_shell_result", None)?["result"].clone();
    client.until("done", Some(3))?;
    ensure!(
        namespace["exit_code"] == 0,
        "namespace command failed: {namespace}"
    );
    let network = namespace["output"]
        .as_str()
        .context("missing namespace")?
        .trim();
    ensure!(
        network != fs::read_link("/proc/self/ns/net")?.to_string_lossy(),
        "daemon shares host network"
    );
    evidence["isolated_network"] = json!(network);
    for (id, text) in [
        (4, "upgrade acceptance: preserved context A"),
        (5, "upgrade acceptance: preserved context B"),
    ] {
        client.send(json!({"type":"message","id":id,"content":text,"no_reply":true}))?;
        client.until("context_message_added", Some(id))?;
    }
    client.send(json!({"type":"get_history","id":6}))?;
    let history = client.until("history", Some(6))?;
    evidence["session_id"] = history["session_id"].clone();
    evidence["history_before"] = history["messages"].clone();
    for text in ["preserved context A", "preserved context B"] {
        ensure!(
            history["messages"].to_string().contains(text),
            "missing context: {history}"
        );
    }
    let invalid = artifacts.join("not-a-native-binary");
    fs::write(&invalid, "#!/bin/sh\nexit 0\n")?;
    fs::set_permissions(&invalid, fs::Permissions::from_mode(0o700))?;
    let nonnexode = artifacts.join("native-nonnexode");
    fs::copy("/usr/bin/true", &nonnexode)?;
    fs::set_permissions(&nonnexode, fs::Permissions::from_mode(0o700))?;
    let mut refusals = vec![];
    for path in [
        PathBuf::from("relative-candidate"),
        artifacts.join("missing-binary"),
        invalid,
        nonnexode,
    ] {
        let result = daemon.upgrade(&path)?;
        ensure!(
            result["step"] == "invalid",
            "invalid candidate accepted: {result}"
        );
        ensure!(
            daemon.info()?["id"] == before["id"],
            "invalid candidate replaced daemon"
        );
        refusals.push(result);
    }
    ensure!(
        refusals
            .last()
            .and_then(|r| r["message"].as_str())
            .is_some_and(|m| m.contains("not a nexode version report")),
        "wrong native refusal"
    );
    evidence["invalid_refusals"] = json!(refusals);
    let marker = daemon.root.join("shell-started");
    let quoted = format!("'{}'", marker.to_string_lossy().replace('\'', "'\\''"));
    client.send(json!({"type":"input_shell","id":7,"command":format!("printf started > {quoted}; sleep 3; printf uninterrupted-shell-ok")}))?;
    wait(|| Ok(marker.exists().then_some(())))?;
    let result = daemon.upgrade(binary)?;
    ensure!(
        result["step"] == "busy",
        "busy runtime accepted upgrade: {result}"
    );
    ensure!(
        daemon.info()?["id"] == before["id"],
        "busy upgrade replaced daemon"
    );
    evidence["busy_refusal"] = result;
    let shell = client.until("input_shell_result", None)?["result"].clone();
    client.until("done", Some(7))?;
    ensure!(
        shell["exit_code"] == 0 && shell["output"] == "uninterrupted-shell-ok",
        "shell interrupted: {shell}"
    );
    evidence["uninterrupted_shell"] = shell;
    client.send(json!({"type":"get_history","id":8}))?;
    ensure!(
        client.until("history", Some(8))?["messages"] == history["messages"],
        "refusals mutated history"
    );
    let result = wait(|| {
        let reply = daemon.upgrade(binary)?;
        Ok((reply["step"] != "busy").then_some(reply))
    })?;
    ensure!(
        result["step"] == "upgrading",
        "upgrade did not start: {result}"
    );
    evidence["upgrade"] = result;
    client.closed()?;
    drop(client);
    evidence["after"] = wait(|| {
        let info = daemon.info()?;
        Ok((info["id"] != before["id"]).then_some(info))
    })?;
    ensure!(
        daemon.child.try_wait()?.is_none(),
        "daemon exited instead of exec"
    );
    ensure!(
        daemon.upgrade(binary)?["step"] == "unchanged",
        "replacement version not confirmed"
    );
    let mut client = Wire::connect(&daemon.socket)?;
    client.send(json!({"type":"subscribe","id":31,"working_dir":daemon.root.join("work"),"target_session_id":history["session_id"]}))?;
    client.send(json!({"type":"get_history","id":9}))?;
    let restored = client.until("history", Some(9))?;
    ensure!(
        restored["session_id"] == history["session_id"]
            && restored["messages"] == history["messages"],
        "history changed across exec"
    );
    evidence["history_after"] = restored["messages"].clone();
    evidence["pid_preserved_across_exec"] = json!(daemon.child.id());
    drop(client);
    drop(daemon);
    let newer = Daemon::start(binary, artifacts.join("downgrade"), "999.0.0")?;
    let before = newer.info()?;
    let result = newer.upgrade(binary)?;
    ensure!(
        result["step"] == "unchanged" && newer.info()?["id"] == before["id"],
        "downgrade replaced newer daemon"
    );
    evidence["downgrade_refusal"] = result;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wire_filters_events_ids_and_reports_disconnects() {
        let (a, mut b) = UnixStream::pair().unwrap();
        b.write_all(
            b"{\"type\":\"pong\",\"id\":9}\n{\"type\":\"ack\"}\n{\"type\":\"pong\",\"id\":1}\n",
        )
        .unwrap();
        drop(b);
        let mut wire = Wire(BufReader::new(a));
        assert_eq!(wire.until("pong", Some(1)).unwrap()["id"], 1);
        assert!(
            wire.until("pong", None)
                .unwrap_err()
                .to_string()
                .contains("disconnected")
        );
    }
    #[test]
    fn wire_rejects_native_errors() {
        let (a, mut b) = UnixStream::pair().unwrap();
        b.write_all(b"{\"type\":\"error\",\"message\":\"denied\"}\n")
            .unwrap();
        assert!(
            Wire(BufReader::new(a))
                .until("pong", None)
                .unwrap_err()
                .to_string()
                .contains("denied")
        );
    }
}
