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

#[derive(Parser)]
struct ReproArgs {
    #[arg(long)]
    binary: Option<PathBuf>,
    #[arg(long)]
    keep: bool,
    #[arg(short, long)]
    verbose: bool,
}
fn state(session: &Session) -> Result<Value> {
    Ok(serde_json::from_str(
        &session.command("expand-badge-state")?,
    )?)
}
fn reset(session: &Session) -> Result<()> {
    session.command("expand-badge-fixture")?;
    pause(0.2)?;
    ensure!(
        state(session)?["diff_mode"] == "Inline",
        "fixture did not reset to collapsed"
    );
    Ok(())
}
fn encodings(session: &Session, verbose: bool) -> Result<i32> {
    reset(session)?;
    session.client.send(b"e\x1b[101u")?;
    pause(0.6)?;
    ensure!(
        state(session)?["diff_mode"] == "Inline",
        "plain e expanded the diff: negative control failed"
    );
    let encodings: [(&str, &[u8]); 4] = [
        ("kitty-csi-u (e;SHIFT|ALT)", b"\x1b[101;4u\x1b[101;4:3u"),
        ("kitty-csi-u (E;SHIFT|ALT)", b"\x1b[69;4u\x1b[69;4:3u"),
        ("legacy esc+E", b"\x1bE"),
        ("legacy esc+e", b"\x1be"),
    ];
    let mut working = vec![];
    for (label, bytes) in encodings {
        reset(session)?;
        session.client.send(bytes)?;
        let deadline = Instant::now() + Duration::from_secs(2);
        let after = loop {
            pause(0.1)?;
            let value = state(session)?;
            if value["diff_mode"] == "FullInline" || Instant::now() >= deadline {
                break value;
            }
        };
        let expanded = after["diff_mode"] == "FullInline";
        println!(
            "{}",
            serde_json::json!({"encoding":label,"expanded":expanded})
        );
        if verbose {
            println!("state={after}");
        }
        if expanded {
            working.push(label);
        }
    }
    reset(session)?;
    session.command("keys:alt+shift+e")?;
    pause(0.3)?;
    let synthetic = state(session)?["diff_mode"] == "FullInline";
    let code = if !working.is_empty() {
        0
    } else if synthetic {
        2
    } else {
        3
    };
    println!(
        "{}",
        serde_json::json!({"working_encodings":working,"synthetic_control":synthetic,"exit_code":code})
    );
    Ok(code)
}
pub(super) fn repro(args: impl Iterator<Item = String>) -> Result<i32> {
    let args = ReproArgs::try_parse_from(
        std::iter::once("repro-expand-edit-shortcut".into()).chain(args),
    )?;
    let binary = args
        .binary
        .or_else(|| dirs::home_dir().map(|p| p.join(".nexode/builds/current/nexode")));
    let result = (|| {
        let binary = binary
            .ok_or_else(|| anyhow::anyhow!("binary path required"))?
            .canonicalize()?;
        let mut session =
            Session::returning(&binary, 40, 120, &[("NEXODE_CLIENT_SELFDEV_MODE", "1")])?;
        let result = encodings(&session, args.verbose);
        if args.keep || result.as_ref().map_or(true, |code| *code != 0) {
            eprintln!("Artifacts: {:?}", session.retain_artifacts());
        }
        result
    })();
    match result {
        Ok(code) => Ok(code),
        Err(error) => {
            eprintln!("expand shortcut setup/control failure: {error:#}");
            Ok(3)
        }
    }
}
