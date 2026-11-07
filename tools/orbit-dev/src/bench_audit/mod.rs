//! Offline Harbor artifact audit. Rules mirror the inherited campaign checker.
use anyhow::{Context, Result, ensure};
use clap::Parser;
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
};
const OVERRIDES: [&str; 11] = [
    "override_timeout_sec",
    "max_timeout_sec",
    "override_setup_timeout_sec",
    "override_cpus",
    "override_memory_mb",
    "override_storage_mb",
    "override_gpus",
    "agent_timeout_multiplier",
    "verifier_timeout_multiplier",
    "agent_setup_timeout_multiplier",
    "environment_build_timeout_multiplier",
];
const TERMS: [&str; 5] = [
    "tbench.ai",
    "terminal-bench.org",
    "github.com/harbor-framework/terminal-bench",
    "github.com/laude-institute/terminal-bench",
    "terminal-bench leaderboard",
];
#[derive(Default)]
struct Report {
    failures: Vec<String>,
    warnings: Vec<String>,
    ready: Vec<String>,
    partial: Vec<String>,
}
fn load(path: &Path) -> Result<Value> {
    serde_json::from_slice(&fs::read(path)?)
        .with_context(|| format!("invalid JSON: {}", path.display()))
}
fn overrides(value: &Value, path: &str, out: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (key, v) in map {
                let p = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{path}.{key}")
                };
                if OVERRIDES.contains(&key.as_str()) && !v.is_null() {
                    out.push(format!("disallowed non-null config field {p}={v}"));
                }
                overrides(v, &p, out);
            }
        }
        Value::Array(items) => {
            for (i, v) in items.iter().enumerate() {
                overrides(v, &format!("{path}[{i}]"), out);
            }
        }
        _ => {}
    }
}
fn entries(path: &Path) -> Result<Vec<PathBuf>> {
    let mut paths = fs::read_dir(path)?
        .map(|e| e.map(|e| e.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    paths.sort();
    Ok(paths)
}
fn scan_logs(path: &Path, root: &Path, warnings: &mut Vec<String>) -> Result<()> {
    for p in entries(path)? {
        let metadata = fs::symlink_metadata(&p)?;
        if metadata.is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            scan_logs(&p, root, warnings)?;
        } else if metadata.is_file()
            && metadata.len() <= 2_000_000
            && [
                "events.ndjson",
                "stderr.txt",
                "exec_stderr.txt",
                "exec_stdout.txt",
                "instruction.txt",
                "download_error.txt",
            ]
            .contains(&p.file_name().unwrap().to_str().unwrap_or(""))
        {
            let text = String::from_utf8_lossy(&fs::read(&p)?).to_lowercase();
            let matches: Vec<_> = TERMS
                .iter()
                .filter(|term| text.contains(**term))
                .copied()
                .collect();
            if !matches.is_empty() {
                warnings.push(format!(
                    "possible forbidden benchmark-site/repo mention in agent log {}: {matches:?}",
                    p.strip_prefix(root)?.display()
                ));
            }
        }
    }
    Ok(())
}
fn audit(root: &Path, min: usize) -> Result<Report> {
    ensure!(min > 0, "minimum trials must be positive");
    let jobs = root.join("harbor-jobs");
    ensure!(
        jobs.is_dir(),
        "Missing harbor-jobs directory: {}",
        jobs.display()
    );
    let mut report = Report::default();
    let manifest = root.join("campaign.json");
    if manifest.exists() {
        let m = load(&manifest)?;
        if m["timeout_multiplier"].as_f64() != Some(1.0) {
            report.failures.push(format!(
                "campaign timeout_multiplier is {}, expected 1.0",
                m["timeout_multiplier"]
            ));
        }
        if m.get("attempts_per_task").is_some_and(|v| !v.is_null())
            && m["attempts_per_task"]
                .as_u64()
                .is_none_or(|v| v < min as u64)
        {
            report.failures.push(format!(
                "campaign attempts_per_task is {}, expected >= {min}",
                m["attempts_per_task"]
            ));
        }
    } else {
        report
            .warnings
            .push("campaign.json not found; validating job configs only".into());
    }
    for task in entries(&jobs)?
        .into_iter()
        .filter(|p| p.is_dir() && !p.is_symlink())
    {
        for run in entries(&task)?
            .into_iter()
            .filter(|p| p.is_dir() && !p.is_symlink())
        {
            let rel = run.strip_prefix(root)?.display().to_string();
            let config_path = run.join("config.json");
            if !config_path.is_file() {
                report.failures.push(format!("{rel}: missing config.json"));
                continue;
            }
            let config = load(&config_path)?;
            let before = report.failures.len();
            if config["timeout_multiplier"].as_f64() != Some(1.0) {
                report.failures.push(format!(
                    "{rel}: timeout_multiplier is {}, expected 1.0",
                    config["timeout_multiplier"]
                ));
            }
            let mut problems = vec![];
            overrides(&config, "", &mut problems);
            report
                .failures
                .extend(problems.into_iter().map(|p| format!("{rel}: {p}")));
            let trials: Vec<_> = entries(&run)?
                .into_iter()
                .filter(|p| {
                    p.is_dir()
                        && !p.is_symlink()
                        && p.file_name().unwrap().to_string_lossy().contains("__")
                        && p.join("result.json").is_file()
                })
                .collect();
            if trials.len() < min {
                report.partial.push(format!(
                    "{rel}: only {} trial result.json files, expected >= {min}",
                    trials.len()
                ));
                continue;
            }
            for trial in trials {
                let result = trial.join("result.json");
                if let Err(e) = load(&result) {
                    report
                        .failures
                        .push(format!("{}: {e}", result.strip_prefix(root)?.display()));
                    continue;
                }
                if !entries(&trial)?
                    .iter()
                    .any(|p| p.file_name().unwrap() != "result.json")
                {
                    report.failures.push(format!(
                        "{rel}: trial dir missing non-result artifacts: {}",
                        trial.strip_prefix(root)?.display()
                    ));
                }
            }
            if before == report.failures.len() {
                report.ready.push(rel);
            }
        }
    }
    scan_logs(&jobs, root, &mut report.warnings)?;
    Ok(report)
}
#[derive(Parser)]
struct Args {
    campaign_dir: PathBuf,
    #[arg(long, default_value_t = 5)]
    min_trials: usize,
}
pub fn run(args: impl Iterator<Item = String>) -> Result<i32> {
    let args = Args::parse_from(std::iter::once("audit-terminal-bench".into()).chain(args));
    let path = if let Ok(s) = args.campaign_dir.strip_prefix("~") {
        dirs::home_dir().context("home unavailable")?.join(s)
    } else {
        args.campaign_dir
    };
    let root = path.canonicalize()?;
    let r = audit(&root, args.min_trials)?;
    println!("campaign: {}", root.display());
    for (label, prefix, items, limit) in [
        ("submit-ready job runs", "OK", &r.ready, usize::MAX),
        (
            "partial/non-submittable job runs",
            "PARTIAL",
            &r.partial,
            usize::MAX,
        ),
        ("failures", "FAIL", &r.failures, usize::MAX),
        ("warnings", "WARN", &r.warnings, 50),
    ] {
        println!("{label}: {}", items.len());
        for item in items.iter().take(limit) {
            println!("  {prefix} {item}");
        }
    }
    Ok(i32::from(!r.failures.is_empty()))
}
#[cfg(test)]
mod tests;
