//! Explicit live caching diagnostic. Offline tests supply a private fake socket.
use super::io;
use anyhow::{Context, Result, ensure};
use clap::Parser;
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const PROMPTS: [&str; 4] = [
    "List the files in /tmp and tell me how many there are.",
    "Now read the contents of /etc/hostname",
    "What's the current date and time? Use the bash tool to run 'date'.",
    "Summarize what you found in the previous steps.",
];
#[path = "usage_oauth.rs"]
mod oauth_comparison;
pub(super) fn oauth(args: impl Iterator<Item = String>) -> Result<i32> {
    oauth_comparison::run(args)
}
#[derive(Parser)]
struct Args {
    #[arg(long)]
    debug_socket: Option<PathBuf>,
    #[arg(long, default_value = "/tmp/cache-test")]
    working_dir: PathBuf,
}
struct Session {
    socket: PathBuf,
    id: String,
}
impl Session {
    fn command(&self, command: &str) -> Result<String> {
        io::debug_session(
            &self.socket,
            command,
            Some(&self.id),
            Duration::from_secs(180),
        )
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        if let Err(error) = io::debug(
            &self.socket,
            &format!("destroy_session:{}", self.id),
            Duration::from_secs(10),
        ) {
            eprintln!("session cleanup failed: {error:#}");
        }
    }
}
fn usage(v: &Value) -> Result<[u64; 4]> {
    let mut result = [0; 4];
    for (i, key) in [
        "input_tokens",
        "output_tokens",
        "cache_read_input_tokens",
        "cache_creation_input_tokens",
    ]
    .iter()
    .enumerate()
    {
        result[i] = if i >= 2 && v[key].is_null() {
            0
        } else {
            v[key]
                .as_u64()
                .with_context(|| format!("invalid usage field {key}"))?
        };
    }
    Ok(result)
}
fn metrics(t: [u64; 4]) -> Value {
    let input = t[0] as f64 + t[2] as f64 + t[3] as f64;
    json!({"input_tokens":t[0], "output_tokens":t[1], "cache_read_input_tokens":t[2], "cache_creation_input_tokens":t[3],
        "cache_hit_rate":if input > 0.0 { 100.0 * t[2] as f64 / input } else { 0.0 },
        "effective_tokens_cache_at_10_percent":t[0] as f64 + 0.1 * t[2] as f64 + t[1] as f64})
}
fn probe(socket: &Path, work: &Path) -> Result<Value> {
    let id = io::create_session(socket, work)?;
    let session = Session {
        socket: socket.into(),
        id,
    };
    let mut totals = [0u64; 4];
    let mut turns = vec![];
    for prompt in PROMPTS {
        ensure!(
            crate::ci_suites::unix::interrupted().is_none(),
            "caching probe interrupted"
        );
        let start = Instant::now();
        let response = session.command(&format!("message:{prompt}"))?;
        let elapsed = start.elapsed().as_secs_f64();
        let value: Value = serde_json::from_str(&session.command("usage")?)?;
        let counts = usage(&value)?;
        for i in 0..4 {
            totals[i] = totals[i]
                .checked_add(counts[i])
                .context("usage total overflow")?;
        }
        turns.push(
            json!({"prompt":prompt, "response":response, "time":elapsed, "usage":metrics(counts)}),
        );
    }
    let mut analysis = vec![];
    if totals[3] > totals[2] {
        analysis.push("More cache creation than reads; caching may not be working well");
    }
    if totals[0] as f64 / PROMPTS.len() as f64 > 1000.0 {
        analysis.push("High non-cached input suggests dynamic content or cache misses");
    }
    Ok(
        json!({"session_id":session.id, "turns":turns, "total":metrics(totals), "average_non_cached_input":totals[0] as f64 / PROMPTS.len() as f64, "analysis":analysis}),
    )
}
pub(super) fn run(args: impl Iterator<Item = String>) -> Result<i32> {
    let a = Args::parse_from(std::iter::once("test-caching-detailed".into()).chain(args));
    let socket = a.debug_socket.unwrap_or_else(|| {
        PathBuf::from(format!("/run/user/{}/nexode-debug.sock", unsafe {
            libc::getuid()
        }))
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&probe(&socket, &a.working_dir)?)?
    );
    Ok(0)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn null_cache_fields_and_zero_denominator_are_supported() {
        let counts =
            usage(&json!({"input_tokens":0,"output_tokens":0,"cache_read_input_tokens":null}))
                .unwrap();
        assert_eq!(metrics(counts)["cache_hit_rate"], 0.0);
        assert!(usage(&json!({"input_tokens":-1,"output_tokens":0})).is_err());
        assert_eq!(metrics([100, 10, 900, 0])["cache_hit_rate"], 90.0);
    }
}
