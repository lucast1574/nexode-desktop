//! Built-CLI browserless login acceptance; no OAuth exchange is completed.
use anyhow::{Context, Result, ensure};
use clap::Parser;
use serde_json::Value;
use std::{
    path::PathBuf,
    process::{Command, Output, Stdio},
    time::Duration,
};
fn has_qr(text: &str) -> bool {
    text.chars().any(|c| "█▀▄".contains(c))
}
fn auth_url(text: &str) -> Result<()> {
    ensure!(
        url::Url::parse(text.trim())?.host_str() == Some("auth.openai.com"),
        "unexpected authorization URL host"
    );
    Ok(())
}
fn validate(output: &Output, json_mode: bool) -> Result<()> {
    ensure!(output.status.success(), "login prompt failed");
    let stdout = std::str::from_utf8(&output.stdout)?;
    let stderr = std::str::from_utf8(&output.stderr)?;
    if json_mode {
        let prompt: Value = serde_json::from_str(stdout)?;
        ensure!(
            prompt["status"] == "pending" && prompt["provider"] == "openai",
            "unexpected JSON login state/provider"
        );
        auth_url(prompt["auth_url"].as_str().context("missing auth URL")?)?;
        ensure!(
            !has_qr(stdout) && !has_qr(stderr),
            "QR leaked into machine-readable login"
        );
    } else {
        ensure!(
            stdout.trim().lines().count() == 1,
            "human login stdout must contain only the URL"
        );
        auth_url(stdout)?;
        ensure!(
            stderr.contains("Scan this QR") && has_qr(stderr),
            "human login QR missing"
        );
        ensure!(
            stderr.contains("--callback-url"),
            "callback instructions missing"
        );
        ensure!(
            !stderr.contains("Waiting up to"),
            "headless login unexpectedly waits for callback"
        );
    }
    Ok(())
}
fn login_stdin(terminal: bool) -> Result<(Stdio, Option<std::fs::File>)> {
    #[cfg(unix)]
    if terminal {
        use std::os::fd::FromRawFd;
        let (mut master, mut slave) = (-1, -1);
        ensure!(
            unsafe {
                libc::openpty(
                    &mut master,
                    &mut slave,
                    std::ptr::null_mut(),
                    std::ptr::null(),
                    std::ptr::null(),
                )
            } == 0,
            "open login test PTY"
        );
        return Ok((
            Stdio::from(unsafe { std::fs::File::from_raw_fd(slave) }),
            Some(unsafe { std::fs::File::from_raw_fd(master) }),
        ));
    }
    ensure!(!terminal, "terminal login fixture requires Unix");
    Ok((Stdio::null(), None))
}
fn check(binary: &std::path::Path, flags: &[&str], terminal: bool, json_mode: bool) -> Result<()> {
    let scratch = std::env::var_os("NEXODE_SCRATCH_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let home = tempfile::Builder::new()
        .prefix("orbit-login-qr-")
        .tempdir_in(scratch)?;
    let mut command = Command::new(binary);
    command
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", home.path())
        .env("NEXODE_HOME", home.path().join("nexode"))
        .env("XDG_CONFIG_HOME", home.path().join("config"))
        .env("DO_NOT_TRACK", "1")
        .env("TERM", "xterm-256color")
        .args([
            "--no-update",
            "--no-selfdev",
            "login",
            "--provider",
            "openai",
        ])
        .args(flags);
    if json_mode {
        command.env("NEXODE_SHOW_LOGIN_QR", "1");
    }
    let (stdin, master_guard) = login_stdin(terminal)?;
    let output = crate::process::capture_with_stdin(&mut command, stdin, Duration::from_secs(20))?;
    drop(master_guard);
    validate(&output, json_mode)
}
#[derive(Parser)]
struct Args {
    binary: PathBuf,
}
pub fn run(args: impl Iterator<Item = String>) -> Result<()> {
    let a = Args::parse_from(std::iter::once("verify-login-qr".into()).chain(args));
    let binary = a.binary.canonicalize()?;
    let terminals = if cfg!(unix) {
        vec![false, true]
    } else {
        vec![false]
    };
    let mut total = 0;
    for flag in ["--no-browser", "--headless"] {
        for terminal in &terminals {
            check(&binary, &[flag], *terminal, false)?;
            total += 1;
        }
    }
    #[cfg(target_os = "linux")]
    for terminal in &terminals {
        check(&binary, &[], *terminal, false)?;
        total += 1;
    }
    for flags in [
        vec!["--no-browser", "--json"],
        vec!["--no-browser", "--print-auth-url", "--json"],
    ] {
        check(&binary, &flags, false, true)?;
        total += 1;
    }
    println!("PASS: {total} browserless login/QR cases; no browser or OAuth exchange completed");
    Ok(())
}
#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::process::ExitStatusExt;
    fn output(out: &str, err: &str) -> Output {
        Output {
            status: std::process::ExitStatus::from_raw(0),
            stdout: out.into(),
            stderr: err.into(),
        }
    }
    #[test]
    fn human_qr_and_json_contracts_reject_crossed_streams_and_wrong_hosts() {
        let human = output(
            "https://auth.openai.com/authorize\n",
            "Scan this QR █▀▄ --callback-url",
        );
        validate(&human, false).unwrap();
        assert!(
            validate(
                &output("https://wrong.example", "Scan this QR █ --callback-url"),
                false
            )
            .is_err()
        );
        assert!(
            validate(
                &output(
                    "https://auth.openai.com/a",
                    "Scan this QR █ --callback-url Waiting up to 60s"
                ),
                false
            )
            .is_err()
        );
        let prompt = r#"{"status":"pending","provider":"openai","auth_url":"https://auth.openai.com/authorize"}"#;
        validate(&output(prompt, ""), true).unwrap();
        assert!(validate(&output(prompt, "█"), true).is_err());
    }
}
