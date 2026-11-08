//! Native, event-driven Waybar JSON status. Remote checks never provision hosts.
mod cloud;
#[cfg(unix)]
mod sockets;
use anyhow::{Context, Result};
use clap::Parser;
use serde_json::{Value, json};
use std::{io::Write, path::PathBuf, process::Command, time::Duration};
fn output(streaming: u64, total: u64, swarm: u64, cloud: &Value) -> Value {
    let shown = |key: &str| {
        cloud[key]
            .as_u64()
            .map(|n| n.to_string())
            .unwrap_or_else(|| "?".into())
    };
    let class = if total == 0 && swarm == 0 && cloud["alive"].as_u64() == Some(0) {
        "none"
    } else if streaming > 0 || cloud["streaming"].as_u64().unwrap_or(0) > 0 {
        "streaming"
    } else {
        "active"
    };
    json!({"text":format!("S {streaming}  Sessions {total}  Agents {swarm}  Cloud {}/{}",shown("streaming"),shown("alive")),"tooltip":format!("Local: {streaming} streaming, {total} root sessions, {swarm} live non-root swarm agents\nCloud: {} streaming / {} alive\n{} (refreshes every 30s)",shown("streaming"),shown("alive"),cloud["detail"].as_str().unwrap_or("cloud")),"class":class})
}
#[derive(Parser)]
struct Args {
    #[arg(long)]
    cloud_probe: bool,
    #[arg(long)]
    home: Option<PathBuf>,
    /// Do not query managed/ephemeral machines or use SSH.
    #[arg(long)]
    no_cloud: bool,
    /// Native orbit-dev executable installed on running remote hosts.
    #[arg(long, default_value = "orbit-dev")]
    remote_binary: String,
    #[arg(long)]
    once: bool,
}
pub fn run(args: impl Iterator<Item = String>) -> Result<()> {
    let a = Args::parse_from(std::iter::once("waybar-status".into()).chain(args));
    let home = dirs::home_dir().context("home unavailable")?;
    let presence = a
        .home
        .or_else(|| std::env::var_os("NEXODE_HOME").map(PathBuf::from))
        .unwrap_or_else(|| home.join(".nexode"));
    if a.cloud_probe {
        println!("{}", cloud::probe(&presence));
        return Ok(());
    }
    let query = move || {
        cloud::query(&home, &a.remote_binary, |args| {
            let result = crate::process::capture(
                Command::new(&args[0]).args(&args[1..]),
                Duration::from_secs(20),
            )?;
            anyhow::ensure!(result.status.success(), "cloud status query failed");
            Ok(String::from_utf8(result.stdout)?)
        })
    };
    #[cfg(not(unix))]
    {
        let _ = query;
        anyhow::bail!("Waybar subscriptions require Unix sockets");
    }
    #[cfg(unix)]
    {
        use std::sync::{Arc, Mutex, mpsc};
        use std::time::Instant;
        let (tx, rx) = mpsc::sync_channel::<()>(1);
        let counts = Arc::new(Mutex::new(if a.no_cloud {
            json!({"streaming":0,"alive":0,"detail":"cloud queries disabled"})
        } else {
            cloud::unknown("checking cloud")
        }));
        if !a.no_cloud {
            if a.once {
                *counts.lock().unwrap() = query();
            } else {
                let counts = counts.clone();
                let tx = tx.clone();
                std::thread::spawn(move || {
                    loop {
                        *counts.lock().unwrap() = query();
                        let _ = tx.try_send(());
                        std::thread::sleep(Duration::from_secs(30));
                    }
                });
            }
        }
        let mut subscriptions = std::collections::BTreeMap::new();
        let mut last = String::new();
        loop {
            let paths = sockets::discover();
            let (streaming, total, swarm) = sockets::collect(&paths);
            let text = output(streaming, total, swarm, &counts.lock().unwrap()).to_string();
            if text != last {
                println!("{text}");
                std::io::stdout().flush()?;
                last = text;
            }
            if a.once {
                return Ok(());
            }
            subscriptions
                .retain(|p, sub: &mut sockets::Subscriber| paths.contains(p) && sub.alive());
            for path in paths {
                if !subscriptions.contains_key(&path) {
                    subscriptions
                        .insert(path.clone(), sockets::Subscriber::start(path, tx.clone()));
                }
            }
            if rx.recv_timeout(Duration::from_secs(10)).is_ok() {
                let start = Instant::now();
                while start.elapsed() < Duration::from_millis(250) {
                    if rx.recv_timeout(Duration::from_millis(50)).is_err() {
                        break;
                    }
                }
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unknown_and_cloud_streaming_remain_visible() {
        let r = output(0, 0, 0, &cloud::unknown("checking"));
        assert!(r["text"].as_str().unwrap().contains("?/?"));
        assert_ne!(r["class"], "none");
        let r = output(0, 0, 0, &json!({"streaming":2,"alive":5,"detail":"cloud"}));
        assert_eq!(r["class"], "streaming");
        assert!(r["text"].as_str().unwrap().contains("2/5"));
        assert!(
            r["tooltip"]
                .as_str()
                .unwrap()
                .contains("Cloud: 2 streaming / 5 alive")
        );
    }
}
