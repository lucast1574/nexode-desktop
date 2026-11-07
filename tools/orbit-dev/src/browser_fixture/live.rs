use super::Server;
use anyhow::{Context, Result, ensure};
use clap::Parser;
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fs::{File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};
#[derive(Parser)]
#[command(
    about = "Explicit live acceptance on an existing disposable browser tab; uses configured inference credentials"
)]
struct Args {
    #[arg(long)]
    tab_id: u64,
}
fn checkpoint(message: &str) -> Result<()> {
    println!("NEXODE_CHECKPOINT {}", json!({"message":message}));
    std::io::stdout().flush()?;
    Ok(())
}
fn executable(output: &str) -> Result<PathBuf> {
    let mut files = BTreeSet::new();
    for line in output.lines().filter(|l| !l.trim().is_empty()) {
        let data: Value = serde_json::from_str(line)?;
        if data["reason"] == "compiler-message" {
            if let Some(s) = data["message"]["rendered"].as_str() {
                eprint!("{s}");
            }
        } else if data["reason"] == "compiler-artifact"
            && data["target"]["name"] == "nexode_app_core"
            && data["target"]["kind"]
                .as_array()
                .is_some_and(|k| k.iter().any(|s| s == "lib"))
            && data["profile"]["test"] == true
            && let Some(p) = data["executable"].as_str()
        {
            files.insert(PathBuf::from(p));
        }
    }
    ensure!(
        files.len() == 1,
        "expected exactly one nexode-app-core library test executable"
    );
    let path = files.pop_first().unwrap();
    ensure!(path.is_file(), "Cargo test executable is missing");
    Ok(path)
}
fn disposable(tab: &Value) -> bool {
    let Some(raw) = tab["url"].as_str() else {
        return false;
    };
    if raw == "about:blank" {
        return true;
    }
    let Ok(url) = url::Url::parse(raw) else {
        return false;
    };
    matches!(url.scheme(), "http" | "https")
        && matches!(
            url.host_str(),
            Some("127.0.0.1" | "localhost" | "[::1]" | "::1")
        )
        && matches!(
            tab["title"].as_str(),
            Some("Nexode isolated browser fixture" | "Jev hybrid verified")
        )
}
#[cfg(unix)]
fn tab_lock(id: u64) -> Result<File> {
    use std::os::{fd::AsRawFd, unix::fs::OpenOptionsExt};
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(format!("/run/user/{}", unsafe { libc::getuid() })));
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(runtime.join(format!("nexode-browser-acceptance-tab-{id}.lock")))?;
    ensure!(
        unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0,
        "another acceptance runner owns this tab"
    );
    Ok(file)
}
#[cfg(not(unix))]
fn tab_lock(_id: u64) -> Result<File> {
    anyhow::bail!("live browser acceptance currently requires Unix")
}
struct Bridge {
    binary: PathBuf,
    tab: u64,
}
impl Bridge {
    fn call(&self, action: &str, mut params: Value) -> Result<Value> {
        params
            .as_object_mut()
            .context("bridge params")?
            .insert("tabId".into(), json!(self.tab));
        let mut c = Command::new(&self.binary);
        c.arg(action).arg(params.to_string());
        let output = crate::process::capture(&mut c, Duration::from_secs(20))?;
        ensure!(
            output.status.success(),
            "bridge {action} failed: {}",
            output.status
        );
        Ok(serde_json::from_slice(&output.stdout)?)
    }
    fn probe(&self) -> Result<Value> {
        Ok(self.call("evaluate",json!({"script":"return {url:location.href,title:document.title,ready:document.readyState};","frameId":0}))?["result"].clone())
    }
    fn navigate(&self, url: &str) -> Result<()> {
        self.call("navigate", json!({"url":url,"wait":true}))?;
        Ok(())
    }
}
struct Fixture {
    bridge: Bridge,
    server: Server,
    _lock: File,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if self.bridge.probe().is_ok_and(|s| {
            s["url"]
                .as_str()
                .is_some_and(|url| url.starts_with(&format!("{}/", self.server.origin)))
        }) && let Err(e) = self.bridge.navigate("about:blank")
        {
            eprintln!("Could not clear disposable fixture tab: {e:#}");
        }
    }
}
pub fn run(repo: &Path, args: impl Iterator<Item = String>) -> Result<()> {
    let args = Args::parse_from(std::iter::once("test-browser-handoff-live".into()).chain(args));
    ensure!(
        args.tab_id > 0
            && !std::env::var("BROWSER_SESSION")
                .unwrap_or_default()
                .trim()
                .is_empty(),
        "positive --tab-id and existing BROWSER_SESSION required"
    );
    checkpoint("Compiling live browser test harness once")?;
    let configure = |c: &mut Command| {
        c.env(
            "NEXODE_BROWSER_HANDOFF_TEST_TAB_ID",
            args.tab_id.to_string(),
        )
        .env(
            "NEXODE_BROWSER_HANDOFF_TEST_BLOCKED_TAB_ID",
            args.tab_id.to_string(),
        )
        .env("NEXODE_BROWSER_HANDOFF_TEST_TRACE", "1")
        .current_dir(repo);
    };
    let mut build = Command::new(repo.join("scripts/dev_cargo.sh"));
    build.args([
        "test",
        "--locked",
        "-p",
        "nexode-app-core",
        "--lib",
        "--no-run",
        "--message-format=json",
    ]);
    configure(&mut build);
    let result = crate::process::capture(&mut build, Duration::from_secs(1800))?;
    eprint!("{}", String::from_utf8_lossy(&result.stderr));
    ensure!(result.status.success(), "test harness build failed");
    let executable = executable(&String::from_utf8(result.stdout)?)?;
    let lock = tab_lock(args.tab_id)?;
    let home = std::env::var_os("NEXODE_HOME")
        .map(PathBuf::from)
        .unwrap_or(
            dirs::home_dir()
                .context("home unavailable")?
                .join(".nexode"),
        );
    let bridge = Bridge {
        binary: home.join("browser/browser"),
        tab: args.tab_id,
    };
    let tabs = bridge.call("listTabs", json!({}))?;
    let tab = tabs["windows"]
        .as_array()
        .context("missing windows")?
        .iter()
        .filter_map(|w| w["tabs"].as_array())
        .flatten()
        .find(|t| t["tabId"].as_u64() == Some(args.tab_id))
        .context("designated disposable tab no longer exists")?;
    ensure!(
        disposable(tab),
        "refusing to replace a non-fixture tab; use an existing disposable about:blank tab"
    );
    let fixture = Fixture {
        bridge,
        server: Server::start(0)?,
        _lock: lock,
    };
    println!(
        "Owned browser fixture: {}, disposable tab {}",
        fixture.server.origin, args.tab_id
    );
    for (name, path) in [
        ("live_jev_decision_smoke", "/"),
        ("live_browser_handoff_completes_local_navigation", "/"),
        (
            "live_browser_handoff_completes_search_and_nested_navigation",
            "/task",
        ),
        ("live_browser_handoff_requests_script_and_resumes", "/"),
        (
            "live_browser_handoff_sensitive_fixture_hands_back_without_actions",
            "/blocked",
        ),
    ] {
        let target = format!("{}{path}", fixture.server.origin);
        fixture.bridge.navigate(&target)?;
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let state = fixture.bridge.probe()?;
            if state["url"] == target && state["ready"] == "complete" {
                break;
            }
            ensure!(
                Instant::now() < deadline,
                "local fixture did not finish navigation"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        checkpoint(&format!("Running {name}"))?;
        let mut c = Command::new(&executable);
        c.args([name, "--ignored", "--nocapture"]);
        configure(&mut c);
        let out = crate::process::capture(&mut c, Duration::from_secs(300))?;
        print!("{}", String::from_utf8_lossy(&out.stdout));
        eprint!("{}", String::from_utf8_lossy(&out.stderr));
        ensure!(out.status.success(), "live acceptance case {name} failed");
    }
    checkpoint("All live browser handoff acceptance cases passed")?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    #[test]
    fn live_runner_rejects_normal_tabs_and_spoofed_loopback_hosts() {
        for url in [
            "https://example.com/",
            "http://localhost.example.com/",
            "file:///tmp/fixture",
        ] {
            assert!(!disposable(
                &json!({"url":url,"title":"Nexode isolated browser fixture"})
            ));
        }
        assert!(!disposable(
            &json!({"url":"http://127.0.0.1:3000/","title":"Personal app"})
        ));
        for url in ["about:blank", "http://127.0.0.1:80/", "http://[::1]:80/"] {
            assert!(disposable(
                &json!({"url":url,"title":"Nexode isolated browser fixture"})
            ));
        }
    }
    #[test]
    fn selects_only_exact_library_test_artifact() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("test-bin");
        fs::write(&path, "").unwrap();
        let record = json!({"reason":"compiler-artifact","target":{"name":"nexode_app_core","kind":["lib"]},"profile":{"test":true},"executable":path});
        assert_eq!(executable(&record.to_string()).unwrap(), path);
        let mut wrong = record;
        wrong["profile"]["test"] = json!(false);
        assert!(executable(&wrong.to_string()).is_err());
    }
}
