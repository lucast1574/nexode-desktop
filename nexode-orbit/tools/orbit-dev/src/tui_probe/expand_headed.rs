//! Explicit compositor smoke: only type after identifying and verifying the new window.
use super::{io, isolated::pause};
use anyhow::{Context, Result, ensure};
use clap::Parser;
use serde_json::Value;
use std::{
    fs,
    path::PathBuf,
    process::Command,
    time::{Duration, Instant},
};
#[derive(Parser)]
struct Args {
    #[arg(long)]
    binary: PathBuf,
    #[arg(long)]
    debug_socket: PathBuf,
    #[arg(long)]
    kitty_socket: Option<PathBuf>,
    #[arg(long, default_value = ".")]
    repo: PathBuf,
}
fn command(program: &str, args: &[&str]) -> Result<String> {
    let output =
        crate::process::capture(Command::new(program).args(args), Duration::from_secs(10))?;
    ensure!(output.status.success(), "{program} failed");
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}
fn windows() -> Result<Value> {
    Ok(serde_json::from_str(&command(
        "niri",
        &["msg", "--json", "windows"],
    )?)?)
}
fn window_id(value: &Value, title: &str) -> Result<Option<u64>> {
    let found = value
        .as_array()
        .context("invalid niri windows")?
        .iter()
        .filter(|w| w["title"] == title)
        .collect::<Vec<_>>();
    ensure!(found.len() <= 1, "ambiguous fixture window title");
    found
        .first()
        .map(|w| w["id"].as_u64().context("invalid window id"))
        .transpose()
}
fn focused(value: &Value, id: u64) -> bool {
    value.as_array().is_some_and(|windows| {
        windows
            .iter()
            .any(|w| w["id"] == id && w["is_focused"] == true)
    })
}
struct Fixture {
    socket: PathBuf,
    kitty: String,
    session: Option<String>,
    window: Option<String>,
    previous: Option<u64>,
}
impl Fixture {
    fn kitty(&self, args: &[&str]) -> Result<String> {
        let mut argv = vec!["@", "--to", &self.kitty];
        argv.extend_from_slice(args);
        command("kitty", &argv)
    }
    fn debug(&self, cmd: &str) -> Result<String> {
        io::debug_session(
            &self.socket,
            cmd,
            self.session.as_deref(),
            Duration::from_secs(10),
        )
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(id) = &self.window
            && let Err(error) = self.kitty(&["close-window", "--match", &format!("id:{id}")])
        {
            eprintln!("fixture window cleanup: {error:#}");
        }
        if let Some(id) = &self.session
            && let Err(error) = io::debug(
                &self.socket,
                &format!("destroy_session:{id}"),
                Duration::from_secs(5),
            )
        {
            eprintln!("fixture session cleanup: {error:#}");
        }
        if let Some(id) = self.previous {
            let _ = command(
                "niri",
                &["msg", "action", "focus-window", "--id", &id.to_string()],
            );
        }
    }
}
fn kitty_socket() -> Result<PathBuf> {
    let mut paths = fs::read_dir("/tmp")?
        .collect::<std::io::Result<Vec<_>>>()?
        .into_iter()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .is_some_and(|s| s.to_string_lossy().starts_with("kitty.sock-"))
        })
        .collect::<Vec<_>>();
    paths.sort();
    for path in paths {
        if command(
            "kitty",
            &["@", "--to", &format!("unix:{}", path.display()), "ls"],
        )
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .is_some_and(|v| v.as_array().is_some_and(|a| !a.is_empty()))
        {
            return Ok(path);
        }
    }
    anyhow::bail!("no Kitty remote socket found; pass --kitty-socket")
}
pub(super) fn run(args: impl Iterator<Item = String>) -> Result<i32> {
    let args =
        Args::try_parse_from(std::iter::once("expand-badge-headed-wtype".into()).chain(args))?;
    let binary = args.binary.canonicalize()?;
    let repo = args.repo.canonicalize()?;
    // Read compositor state before creating anything or sending keys.
    let previous = windows()?
        .as_array()
        .context("invalid niri windows")?
        .iter()
        .find(|w| w["is_focused"] == true)
        .and_then(|w| w["id"].as_u64());
    let socket = args.kitty_socket.map(Ok).unwrap_or_else(kitty_socket)?;
    let mut fixture = Fixture {
        socket: args.debug_socket,
        kitty: format!("unix:{}", socket.display()),
        session: None,
        window: None,
        previous,
    };
    let created: Value = serde_json::from_str(
        &fixture.debug(&format!("create_session:selfdev:{}", repo.display()))?,
    )?;
    fixture.session = Some(
        created["session_id"]
            .as_str()
            .context("missing session id")?
            .into(),
    );
    let title = format!(
        "NEXODE_EXPAND_E2E_{}_{}",
        std::process::id(),
        chrono::Utc::now().timestamp_micros()
    );
    fixture.window = Some(fixture.kitty(&[
        "launch",
        "--type",
        "os-window",
        "--os-window-title",
        &title,
        binary.to_str().context("binary path encoding")?,
        "self-dev",
        "--resume",
        fixture.session.as_deref().unwrap(),
    ])?);
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let clients: Value = serde_json::from_str(&fixture.debug("clients:map")?)?;
        if clients["clients"].as_array().is_some_and(|a| {
            a.iter()
                .any(|c| c["session_id"].as_str() == fixture.session.as_deref())
        }) {
            break;
        }
        ensure!(Instant::now() < deadline, "client did not attach");
        pause(0.15)?;
    }
    fixture.debug("client:expand-badge-fixture")?;
    let before: Value = serde_json::from_str(&fixture.debug("client:expand-badge-state")?)?;
    ensure!(
        before["diff_mode"] == "Inline" && before["input"] == "",
        "invalid initial expand fixture"
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    let id = loop {
        if let Some(id) = window_id(&windows()?, &title)? {
            break id;
        }
        ensure!(
            Instant::now() < deadline,
            "could not identify fixture compositor window"
        );
        pause(0.1)?;
    };
    command(
        "niri",
        &["msg", "action", "focus-window", "--id", &id.to_string()],
    )?;
    fixture.kitty(&[
        "focus-window",
        "--match",
        &format!("id:{}", fixture.window.as_deref().unwrap()),
    ])?;
    pause(0.5)?;
    ensure!(
        focused(&windows()?, id),
        "fixture window is not focused; refusing to type"
    );
    command(
        "wtype",
        &["-M", "alt", "-M", "shift", "e", "-m", "shift", "-m", "alt"],
    )?;
    pause(0.5)?;
    let after: Value = serde_json::from_str(&fixture.debug("client:expand-badge-state")?)?;
    ensure!(
        after["diff_mode"] == "FullInline" && after["input"] == "",
        "headed Alt+Shift+E did not expand cleanly"
    );
    println!("PASS: headed wtype Alt+Shift+E expands edit badge");
    Ok(0)
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn focus_requires_exact_unique_title_and_matching_focused_id() {
        let value = json!([{"id":1,"title":"fixture-other","is_focused":true},{"id":2,"title":"fixture","is_focused":false}]);
        assert_eq!(window_id(&value, "fixture").unwrap(), Some(2));
        assert!(!focused(&value, 2));
        assert!(focused(&value, 1));
        assert!(
            window_id(
                &json!([{"id":1,"title":"fixture"},{"id":2,"title":"fixture"}]),
                "fixture"
            )
            .is_err()
        );
    }
}
