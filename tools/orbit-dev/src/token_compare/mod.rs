//! Explicit live comparison against Claude CLI. Unit tests never contact providers.
use anyhow::{Context, Result, ensure};
use clap::Parser;
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};
#[derive(Default, Debug)]
struct Usage {
    input: u64,
    output: u64,
    read: u64,
    write: u64,
    cost: Option<f64>,
    duration: Option<u64>,
}
impl Usage {
    fn total(&self) -> Result<u64> {
        self.input
            .checked_add(self.output)
            .and_then(|n| n.checked_add(self.read))
            .and_then(|n| n.checked_add(self.write))
            .context("token count overflow")
    }
    fn value(&self) -> Result<Value> {
        Ok(
            json!({"input":self.input,"output":self.output,"cache_read":self.read,"cache_write":self.write,"total":self.total()?,"cost_usd":self.cost,"duration_ms":self.duration}),
        )
    }
}
fn claude(bytes: &[u8]) -> Result<Usage> {
    let v: Value = serde_json::from_slice(bytes)?;
    ensure!(v["is_error"] != true, "Claude reported an error");
    let u = &v["usage"];
    let count = |key: &str| {
        u[key]
            .as_u64()
            .with_context(|| format!("missing or invalid Claude {key}"))
    };
    Ok(Usage {
        input: count("input_tokens")?,
        output: count("output_tokens")?,
        read: u["cache_read_input_tokens"].as_u64().unwrap_or(0),
        write: u["cache_creation_input_tokens"].as_u64().unwrap_or(0),
        cost: v["total_cost_usd"].as_f64(),
        duration: v["duration_ms"].as_u64(),
    })
}
fn nexode(text: &str) -> Result<Usage> {
    let mut usage = None;
    for line in text.lines().filter(|l| l.contains("[trace] token_usage")) {
        let mut fields = std::collections::BTreeMap::new();
        for part in line.split_whitespace() {
            if let Some((k, v)) = part.split_once('=')
                && ["input", "output", "cache_read", "cache_write"].contains(&k)
            {
                fields.insert(k, v.parse::<u64>()?);
            }
        }
        // Preserve the inherited final trace record contract; absent evidence is a failure.
        usage = Some(Usage {
            input: *fields.get("input").context("missing trace input")?,
            output: *fields.get("output").context("missing trace output")?,
            read: fields.get("cache_read").copied().unwrap_or(0),
            write: fields.get("cache_write").copied().unwrap_or(0),
            ..Default::default()
        });
    }
    usage.context("no Nexode token-usage trace found")
}
fn comparison(c: &Usage, n: &Usage) -> Result<Value> {
    let c = c.value()?;
    let n = n.value()?;
    let mut diff = json!({});
    let mut pct = json!({});
    for key in ["input", "output", "cache_read", "cache_write", "total"] {
        let cv = c[key].as_u64().unwrap();
        let nv = n[key].as_u64().unwrap();
        let d = i128::from(nv) - i128::from(cv);
        diff[key] = serde_json::to_value(i64::try_from(d).context("token difference overflow")?)?;
        if ["input", "output", "total"].contains(&key) {
            pct[key] = if cv == 0 {
                if nv == 0 { json!(0.0) } else { Value::Null }
            } else {
                json!((nv as f64 / cv as f64 - 1.0) * 100.0)
            };
        }
    }
    Ok(json!({"claude":c,"nexode":n,"diff":diff,"pct_diff":pct}))
}
fn acceptable(results: &[Value], expected: usize) -> bool {
    if expected == 0 || results.len() != expected {
        return false;
    }
    let sum = |tool: &str| -> Option<u128> {
        results.iter().try_fold(0u128, |a, r| {
            a.checked_add(r["comparison"][tool]["total"].as_u64()? as u128)
        })
    };
    match (sum("claude"), sum("nexode")) {
        (Some(c), Some(n)) if c > 0 => n * 2 <= c * 3,
        _ => false,
    }
}
#[derive(Parser)]
struct Args {
    #[arg(short, long)]
    verbose: bool,
    #[arg(short = 'n', long, default_value_t = 1)]
    runs: usize,
    #[arg(long)]
    json: bool,
    #[arg(long)]
    nexode: Option<PathBuf>,
    #[arg(long, default_value = "claude")]
    claude: PathBuf,
    #[arg(long, default_value = "opus")]
    claude_model: String,
    #[arg(long, default_value = "claude-opus-4-5-20251101")]
    nexode_model: String,
}
fn run_pair(a: &Args, binary: &Path, prompt: &str, work: &Path) -> Result<Value> {
    let c = crate::process::capture(
        Command::new(&a.claude)
            .args([
                "-p",
                "--output-format",
                "json",
                "--dangerously-skip-permissions",
                "--model",
                &a.claude_model,
                prompt,
            ])
            .current_dir(work),
        Duration::from_secs(120),
    )?;
    ensure!(c.status.success(), "Claude CLI failed");
    let c = claude(&c.stdout)?;
    std::thread::sleep(Duration::from_secs(1));
    let home = tempfile::tempdir()?;
    let n = crate::process::capture(
        Command::new(binary)
            .args(["run", "--no-update", "--model", &a.nexode_model, prompt])
            .current_dir(work)
            .env("NEXODE_HOME", home.path())
            .env("NEXODE_TRACE", "1"),
        Duration::from_secs(120),
    )?;
    ensure!(n.status.success(), "Nexode CLI failed");
    let n = nexode(&String::from_utf8_lossy(&n.stderr))?;
    comparison(&c, &n)
}
pub fn run(root: &Path, args: impl Iterator<Item = String>) -> Result<i32> {
    let a = Args::parse_from(std::iter::once("compare-token-usage".into()).chain(args));
    ensure!(
        a.runs > 0 && a.runs <= 1000,
        "runs must be between 1 and 1000"
    );
    let binary = a.nexode.clone().unwrap_or_else(|| {
        let p = root.join("target/release/nexode");
        if p.is_file() {
            p
        } else {
            PathBuf::from("nexode")
        }
    });
    let binary = if binary.components().count() > 1 {
        binary.canonicalize()?
    } else {
        binary
    };
    let prompts = [
        "Reply with a single word: test",
        "What is 2 + 2? Reply with just the number.",
        "List three primary colors, one per line.",
    ];
    let work = tempfile::tempdir()?;
    let mut results = vec![];
    for prompt in prompts {
        for run in 1..=a.runs {
            eprintln!("[Run {run}/{}] {prompt}", a.runs);
            match run_pair(&a, &binary, prompt, work.path()) {
                Ok(comparison) => {
                    if a.verbose {
                        eprintln!("{}", serde_json::to_string_pretty(&comparison)?);
                    }
                    results.push(json!({"prompt":prompt,"run":run,"comparison":comparison}));
                }
                Err(e) => eprintln!("FAILED: {e}"),
            }
            std::thread::sleep(Duration::from_secs(2));
        }
    }
    let passed = acceptable(&results, prompts.len() * a.runs);
    if a.json {
        println!("{}", serde_json::to_string_pretty(&results)?);
    } else {
        println!(
            "Token Usage Comparison: Nexode vs Claude Code CLI\nSuccessful runs: {}/{}",
            results.len(),
            prompts.len() * a.runs
        );
        for r in &results {
            println!(
                "{} — Claude {} | Nexode {} | Diff {}",
                r["prompt"].as_str().unwrap(),
                r["comparison"]["claude"]["total"],
                r["comparison"]["nexode"]["total"],
                r["comparison"]["diff"]["total"]
            );
        }
        for tool in ["claude", "nexode"] {
            let total: u128 = results
                .iter()
                .map(|r| r["comparison"][tool]["total"].as_u64().unwrap() as u128)
                .sum();
            let io: u128 = results
                .iter()
                .map(|r| {
                    r["comparison"][tool]["input"].as_u64().unwrap() as u128
                        + r["comparison"][tool]["output"].as_u64().unwrap() as u128
                })
                .sum();
            println!("{tool}: total {total}, input+output excluding cache {io}");
        }
        println!(
            "{}: requires all runs, positive reference usage and at most 50% total overhead",
            if passed { "PASS" } else { "FAIL" }
        );
    }
    Ok(i32::from(!passed))
}
#[cfg(test)]
mod tests;
