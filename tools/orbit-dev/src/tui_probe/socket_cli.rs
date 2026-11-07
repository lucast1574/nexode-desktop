//! Native shell socket helpers with bounded protocol reads.
use super::io;
use anyhow::{Context, Result, ensure};
use clap::Parser;
use std::{path::PathBuf, time::Duration};
#[derive(Parser)]
struct Args {
    #[arg(long)]
    socket: PathBuf,
    #[arg(long, default_value_t = 5.0)]
    timeout: f64,
    command: Option<String>,
    #[arg(long)]
    session: Option<String>,
}
fn connect(path: &std::path::Path, timeout: Duration) -> Result<()> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(async {
            tokio::time::timeout(timeout, tokio::net::UnixStream::connect(path))
                .await
                .context("socket connect timed out")??;
            Ok(())
        })
}
pub(super) fn run(command: &str, args: impl Iterator<Item = String>) -> Result<i32> {
    let a = Args::try_parse_from(std::iter::once(command.to_owned()).chain(args))?;
    ensure!(
        a.timeout.is_finite() && a.timeout > 0.0 && a.timeout <= 3600.0,
        "invalid timeout"
    );
    let timeout = Duration::from_secs_f64(a.timeout);
    if command == "socket-connect" {
        ensure!(
            a.command.is_none() && a.session.is_none(),
            "connect does not take a debug command"
        );
        connect(&a.socket, timeout)?;
    } else {
        let response = io::debug_response(
            &a.socket,
            a.command.as_deref().context("debug command required")?,
            a.session.as_deref(),
            timeout,
        )?;
        println!("{response}");
        ensure!(
            response["ok"] != false && response["type"] != "error",
            "debug command rejected"
        );
    }
    Ok(0)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn connect_accepts_listener_and_rejects_missing_socket() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("socket");
        assert!(connect(&p, Duration::from_secs(1)).is_err());
        let _s = std::os::unix::net::UnixListener::bind(&p).unwrap();
        connect(&p, Duration::from_secs(1)).unwrap();
    }
}
