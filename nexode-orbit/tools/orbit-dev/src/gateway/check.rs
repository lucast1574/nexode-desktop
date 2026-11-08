use super::*;
use clap::Parser;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};
#[derive(Parser)]
struct Args {
    #[arg(long)]
    host: String,
    #[arg(long, default_value_t = 7643)]
    port: u16,
    #[arg(long)]
    code: Option<String>,
    #[arg(long)]
    working_dir: String,
    #[arg(long, default_value = "nexode-remote-check")]
    device_id: String,
    #[arg(long, default_value = "nexode remote check")]
    device_name: String,
    #[arg(
        long,
        default_value = "Run a shell command that prints the OS name and current directory, then report the result."
    )]
    prompt: String,
    #[arg(long, default_value_t = 150.0)]
    timeout: f64,
    #[arg(long)]
    token_dir: Option<PathBuf>,
}
fn token_path(dir: &Path, host: &str, port: u16) -> PathBuf {
    let safe = !host.is_empty()
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-_".contains(&b));
    let name = if safe {
        host.to_owned()
    } else {
        format!("{:x}", Sha256::digest(host.as_bytes()))
    };
    dir.join(format!("{name}_{port}.json"))
}
fn load_or_pair(a: &Args, dir: &Path) -> Result<String> {
    let path = token_path(dir, &a.host, a.port);
    let value = if path.exists() && a.code.is_none() {
        ensure!(
            fs::symlink_metadata(&path)?.is_file() && !path.is_symlink(),
            "token cache must be regular file"
        );
        serde_json::from_slice(&fs::read(&path)?)?
    } else {
        let code = a
            .code
            .as_ref()
            .context("No cached token; run nexode pair remotely and pass --code")?;
        let result = http(
            &a.host,
            a.port,
            "/pair",
            Some(&json!({"code":code,"device_id":a.device_id,"device_name":a.device_name})),
        )?;
        ensure!(
            result["token"].as_str().is_some_and(|s| !s.is_empty()),
            "pairing response has no token"
        );
        crate::oauth::private_write(&path, &result)?;
        result
    };
    Ok(value["token"]
        .as_str()
        .filter(|s| !s.is_empty())
        .context("cached token missing")?
        .to_owned())
}
fn run_check(a: &Args, dir: &Path) -> Result<()> {
    ensure!(
        a.timeout.is_finite() && a.timeout > 0.0 && a.timeout <= 86400.0,
        "timeout must be positive and at most one day"
    );
    let info = http(&a.host, a.port, "/health", None)?;
    ensure!(info["status"] == "ok", "gateway health not OK");
    ensure!(
        info["gateway"] == true,
        "gateway disabled; enable [gateway] remotely"
    );
    println!("[PASS] reachable /health and gateway enabled");
    let token = load_or_pair(a, dir)?;
    println!("[PASS] paired (token available)");
    let start = Instant::now();
    let mut gateway = Gateway::connect(&a.host, a.port, &token, Duration::from_secs(30))?;
    println!(
        "[PASS] websocket upgrade: {:.0}ms",
        start.elapsed().as_secs_f64() * 1000.0
    );
    let id = gateway.send(json!({"type":"subscribe","working_dir":a.working_dir}))?;
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut subscribed = false;
    while let Some(v) = gateway.next(deadline)? {
        if v["type"] == "error" {
            anyhow::bail!("gateway rejected subscription");
        }
        if v["type"] == "done" && v["id"] == id {
            subscribed = true;
            break;
        }
    }
    ensure!(subscribed, "subscribe not acknowledged");
    println!("[PASS] subscribe acknowledged");
    let id = gateway.send(json!({"type":"get_history"}))?;
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut history = None;
    while let Some(v) = gateway.next(deadline)? {
        if v["type"] == "history" {
            history = Some(v);
            break;
        }
        if v["type"] == "error" || (v["type"] == "done" && v["id"] == id) {
            break;
        }
    }
    let history = history.context("get_history did not return history")?;
    println!(
        "[PASS] history: {} msgs, {} models",
        history["messages"].as_array().map(Vec::len).unwrap_or(0),
        history["available_models"]
            .as_array()
            .map(Vec::len)
            .unwrap_or(0)
    );
    let start = Instant::now();
    let id = gateway.send(json!({"type":"message","content":a.prompt}))?;
    let deadline = Instant::now() + Duration::from_secs_f64(a.timeout);
    let mut text = String::new();
    let mut tools = vec![];
    let mut done = false;
    let mut errors = 0;
    while let Some(v) = gateway.next(deadline)? {
        match v["type"].as_str().unwrap_or("") {
            "text_delta" => {
                text.push_str(v["text"].as_str().unwrap_or(""));
                ensure!(
                    text.len() <= 4 * 1024 * 1024,
                    "remote reply exceeds diagnostic limit"
                );
            }
            "tool_done" => {
                if let Some(name) = v["name"].as_str() {
                    tools.push(name.to_owned());
                }
            }
            "error" => {
                errors += 1;
                eprintln!("[FAIL] remote error event (payload withheld)");
            }
            "done" if v["id"] == id => {
                done = true;
                break;
            }
            _ => {}
        }
    }
    ensure!(done, "agent turn did not complete before timeout");
    ensure!(errors == 0, "agent emitted {errors} errors");
    ensure!(!text.trim().is_empty(), "assistant produced no text");
    println!(
        "[PASS] agent turn completed in {:.1}s",
        start.elapsed().as_secs_f64()
    );
    if !tools.is_empty() {
        println!(
            "Remote tools: {}",
            tools.join(", ").replace(&token, "[redacted]")
        );
    }
    let reply = text.replace(&token, "[redacted]");
    println!(
        "\n--- remote reply ---\n{}\n\nALL CHECKS PASSED",
        reply.trim().chars().take(600).collect::<String>()
    );
    Ok(())
}
pub(super) fn run(args: impl Iterator<Item = String>) -> Result<()> {
    let a = Args::parse_from(std::iter::once("remote-check".into()).chain(args));
    let dir = a.token_dir.clone().unwrap_or(
        dirs::home_dir()
            .context("home unavailable")?
            .join(".nexode/remote-tokens"),
    );
    run_check(&a, &dir)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cache_paths_never_escape_directory() {
        let d = Path::new("/cache");
        assert_eq!(
            token_path(d, "host.example", 7643),
            d.join("host.example_7643.json")
        );
        for h in ["../../escape", "::1", "x/y"] {
            let p = token_path(d, h, 123);
            assert_eq!(p.parent(), Some(d));
            assert!(!p.file_name().unwrap().to_str().unwrap().contains('/'));
        }
    }
}
