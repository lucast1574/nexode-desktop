//! Analyze persistent websocket reuse and the authoritative cache-read field.
use anyhow::{Context, Result, ensure};
use clap::Parser;
use regex::Regex;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
#[derive(Default, Debug)]
struct Report {
    conn: BTreeMap<String, u64>,
    reset: BTreeMap<String, u64>,
    unavailable: BTreeMap<String, u64>,
    idle: Vec<u64>,
    cache: BTreeMap<String, [i64; 3]>,
    percent: BTreeMap<String, u64>,
    n: u64,
    zero: u64,
    zero_tokens: i64,
}
fn increment(map: &mut BTreeMap<String, u64>, key: &str) {
    *map.entry(key.into()).or_default() += 1;
}
impl Report {
    fn json(&self) -> Value {
        json!({"conn":self.conn,"reset_reason":self.reset,"reuse_detail":self.unavailable,"idle_reconnect_secs":self.idle,"cache":self.cache,"oa_readpct":self.percent,"oa_readpct_n":self.n,"oa_zero":self.zero,"oa_zero_tokens":self.zero_tokens})
    }
    fn add(&mut self, text: &str) {
        let reason = Regex::new(r"reason=([a-z_]+)").unwrap();
        let idle = Regex::new(r"Persistent WS idle for (\d+)s; reconnecting").unwrap();
        let fields = Regex::new(r"(\w+)=([^\s]+)").unwrap();
        for line in text.lines() {
            if line.contains("persistent-reuse") {
                increment(&mut self.conn, "reuse");
            } else if line.contains("persistent-fresh") {
                increment(&mut self.conn, "fresh");
            }
            if let Some(c) = reason.captures(line) {
                if line.contains("persistent_state_reset") {
                    increment(&mut self.reset, &c[1]);
                }
                if line.contains("persistent_reuse_unavailable_detail") {
                    increment(&mut self.unavailable, &c[1]);
                }
            }
            if let Some(c) = idle.captures(line)
                && let Ok(n) = c[1].parse()
            {
                self.idle.push(n);
            }
            if line.contains("KV_CACHE_USAGE") {
                let d: BTreeMap<_, _> = fields
                    .captures_iter(line)
                    .map(|c| (c[1].to_owned(), c[2].to_owned()))
                    .collect();
                let get = |key: &str, default: &str| {
                    d.get(key).cloned().unwrap_or_else(|| default.into())
                };
                let provider = get("provider", "?");
                let (Ok(input), Ok(read)) = (
                    get("input", "0").parse::<i64>(),
                    get("cache_read", "0").parse::<i64>(),
                ) else {
                    continue;
                };
                let bucket = self.cache.entry(provider.clone()).or_default();
                bucket[0] += input;
                bucket[1] += read;
                bucket[2] += 1;
                if provider == "OpenAI" {
                    if input + read > 1024 && read == 0 {
                        self.zero += 1;
                        self.zero_tokens += input;
                    }
                    if let Ok(p) = get("read_pct", "None").parse::<f64>() {
                        self.n += 1;
                        increment(
                            &mut self.percent,
                            if p >= 90.0 {
                                ">=90%"
                            } else if p >= 70.0 {
                                "70-90%"
                            } else if p >= 50.0 {
                                "50-70%"
                            } else if p > 0.0 {
                                "1-50%"
                            } else {
                                "0%"
                            },
                        );
                    }
                }
            }
        }
    }
    fn print(&self) {
        let reuse = *self.conn.get("reuse").unwrap_or(&0);
        let fresh = *self.conn.get("fresh").unwrap_or(&0);
        let total = reuse + fresh;
        println!("\n== Connection mix ==");
        if total > 0 {
            println!(
                "  reuse : {reuse:>6} ({:.1}%)\n  fresh : {fresh:>6} ({:.1}%)",
                100.0 * reuse as f64 / total as f64,
                100.0 * fresh as f64 / total as f64
            );
        } else {
            println!("  (no ConnectionType events)");
        }
        println!("\n== Fresh-connection causes ==");
        for (name, counts) in [
            ("persistent_reuse_unavailable_detail", &self.unavailable),
            ("persistent_state_reset", &self.reset),
        ] {
            println!("  {name}:");
            let mut values: Vec<_> = counts.iter().collect();
            values.sort_by_key(|(_, n)| std::cmp::Reverse(**n));
            for (reason, n) in values {
                println!("    {reason:24} {n}");
            }
        }
        println!("\n== Idle-reconnect events (the avoidable churn) ==");
        if self.idle.is_empty() {
            println!("  count=0 (no idle reconnects logged)");
        } else {
            let mut idle = self.idle.clone();
            idle.sort();
            println!(
                "  count={}  min={}s  median={}s  max={}s",
                idle.len(),
                idle[0],
                idle[idle.len() / 2],
                idle.last().unwrap()
            );
            for thr in [90, 300, 600, 900] {
                println!(
                    "  threshold {thr:>4}s would have avoided {}/{} reconnects",
                    idle.iter().filter(|s| **s < thr).count(),
                    idle.len()
                );
            }
        }
        println!("\n== Realized cache hit rate (read / (read + new_input)) ==");
        for (provider, [input, read, n]) in &self.cache {
            let total = input + read;
            if total != 0 {
                println!(
                    "  {provider:8} hit={:5.1}%  read={read:>13} new_input={input:>13} n={n}",
                    100.0 * *read as f64 / total as f64
                );
            }
        }
        println!(
            "\n== OpenAI cold-prefill cost ==\n  zero-read prompts (>1024 tok): {}  (~{} full-price input tokens)",
            self.zero, self.zero_tokens
        );
        if self.n > 0 {
            println!("  read_pct distribution (harness field, n={}):", self.n);
            for k in [">=90%", "70-90%", "50-70%", "1-50%", "0%"] {
                let c = self.percent.get(k).copied().unwrap_or(0);
                println!(
                    "    {k:8}: {c:>5} ({:.1}%)",
                    100.0 * c as f64 / self.n as f64
                );
            }
        }
    }
}
#[derive(Parser)]
#[command(about = "Analyze websocket reuse and cache usage from local logs")]
struct Args {
    files: Vec<PathBuf>,
    #[arg(long)]
    json: bool,
}
fn files(home: &Path) -> Result<Vec<PathBuf>> {
    let dir = home.join(".nexode/logs");
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    for entry in fs::read_dir(dir)? {
        let p = entry?.path();
        if p.is_file()
            && p.file_name().is_some_and(|n| {
                let n = n.to_string_lossy();
                n.starts_with("nexode-") && n.ends_with(".log")
            })
        {
            files.push(p);
        }
    }
    files.sort();
    Ok(files)
}
pub fn run(args: impl Iterator<Item = String>) -> Result<()> {
    let args = Args::parse_from(std::iter::once("analyze-ws-cache".into()).chain(args));
    let paths = if args.files.is_empty() {
        files(&dirs::home_dir().context("home unavailable")?)?
    } else {
        args.files
    };
    ensure!(!paths.is_empty(), "no log files found");
    let mut report = Report::default();
    for p in &paths {
        match fs::read(p) {
            Ok(b) => report.add(&String::from_utf8_lossy(&b)),
            Err(e) => eprintln!("skipping {}: {e}", p.display()),
        }
    }
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report.json())?);
    } else {
        println!("Scanned {} log file(s)", paths.len());
        report.print();
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn authoritative_pct_and_malformed_values() {
        let mut r = Report::default();
        r.add("persistent-reuse\npersistent-fresh\npersistent_state_reset reason=idle\npersistent_reuse_unavailable_detail reason=no_response\nPersistent WS idle for 90s; reconnecting\nKV_CACHE_USAGE provider=OpenAI input=2000 cache_read=0 read_pct=0\nKV_CACHE_USAGE provider=OpenAI input=1 cache_read=999 read_pct=70\nKV_CACHE_USAGE provider=Other input=10 cache_read=5\nKV_CACHE_USAGE provider=OpenAI input=invalid cache_read=0 read_pct=99\n");
        assert_eq!(r.cache["OpenAI"], [2001, 999, 2]);
        assert_eq!(r.percent["70-90%"], 1);
        assert!(!r.percent.contains_key(">=90%"));
        assert_eq!(r.zero, 1);
        assert_eq!(r.zero_tokens, 2000);
        assert_eq!(r.idle, vec![90]);
        assert_eq!(r.conn["reuse"], 1);
        assert_eq!(r.reset["idle"], 1);
        assert_eq!(r.unavailable["no_response"], 1);
    }
}
