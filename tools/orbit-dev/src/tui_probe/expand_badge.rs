use super::isolated::{Session, pause};
use anyhow::{Result, ensure};
use clap::Parser;
use serde_json::Value;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
#[derive(Parser)]
struct Args {
    #[arg(long)]
    binary: PathBuf,
}
pub(super) fn run(args: impl Iterator<Item = String>) -> Result<i32> {
    let args = Args::parse_from(std::iter::once("expand-badge-headless".into()).chain(args));
    let mut session = Session::returning(
        &args.binary.canonicalize()?,
        40,
        120,
        &[("NEXODE_CLIENT_SELFDEV_MODE", "1")],
    )?;
    let result = check(&session);
    if result.is_err() {
        eprintln!("Artifacts: {:?}", session.retain_artifacts());
    }
    result?;
    println!("PASS: headless PTY Alt+Shift+E expands edit badge without inserting text");
    Ok(0)
}
fn check(session: &Session) -> Result<()> {
    println!("fixture={}", session.command("expand-badge-fixture")?);
    pause(0.5)?;
    ensure!(
        session.command("history")?.contains("Edited demo.txt"),
        "fixture did not install edit message"
    );
    let before: Value = serde_json::from_str(&session.command("expand-badge-state")?)?;
    println!("before={before}");
    ensure!(
        before["diff_mode"] == "Inline" && before["input"] == "",
        "invalid initial state: {before}"
    );
    let screen = session.client.snapshot()?.rows.join("\n");
    ensure!(
        screen.contains("demo.txt") && screen.contains("expand"),
        "expand badge not rendered: {screen}"
    );
    session.client.send(b"\x1b[101;4u")?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        pause(0.05)?;
        let after: Value = serde_json::from_str(&session.command("expand-badge-state")?)?;
        if after["diff_mode"] == "FullInline" && after["input"] == "" {
            println!("after_csi_u={after}");
            return Ok(());
        }
        if Instant::now() >= deadline {
            session.client.send(b"\x1be")?;
            pause(0.5)?;
            anyhow::bail!(
                "Alt+Shift+E did not expand: after_csi_u={after}, after_legacy={}",
                session.command("expand-badge-state")?
            );
        }
    }
}
