use super::{Gh, Github, objects};
use anyhow::{Context, Result, bail, ensure};
use clap::Parser;
use serde_json::Value;
use std::time::{Duration, Instant};
pub const MANDATORY: &[&str] = &[
    "Quality Guardrails",
    "Build & Test (ubuntu-latest)",
    "Build & Test (macos-latest)",
    "Build & Test (windows-latest)",
    "Windows ARM64 Release Configuration",
];
pub const PENDING: &[&str] = &["queued", "in_progress", "waiting", "pending", "requested"];
const ADVISORY: &[&str] = &[
    "Check formatting",
    "Enforce warning budget",
    "Enforce oversized-file ratchet",
    "Enforce oversized-test ratchet",
    "Enforce wildcard re-export ratchet",
    "Enforce no unused dependencies",
];
fn advisory(job: &str) -> &[&str] {
    match job {
        "Quality Guardrails" => ADVISORY,
        "Format" => &["Check formatting"],
        "Build & Test (ubuntu-latest)" | "Build & Test (macos-latest)" => {
            &["Enforce warning budget (Linux, macOS)"]
        }
        _ => &[],
    }
}
fn optional(job: &str) -> &[&str] {
    match job {
        "Quality Guardrails" => &["Post Run Swatinem/rust-cache@v2"],
        "Build & Test (ubuntu-latest)" => &[
            "Check PowerShell script syntax (Windows)",
            "Post Run Swatinem/rust-cache@v2",
        ],
        "Build & Test (macos-latest)" => &[
            "Install mold linker (Linux)",
            "Run automatic model discovery regressions (Linux only)",
            "Run embedding numeric-stability cohort (Linux only)",
            "Run TUI library tests (Linux only)",
            "Check PowerShell script syntax (Windows)",
            "Security preflight (Linux)",
            "Post Run Swatinem/rust-cache@v2",
        ],
        _ => &[],
    }
}
pub fn validate(repo: &str, sha: &str, timeout: f64, poll: f64) -> Result<()> {
    ensure!(
        regex::Regex::new(r"\A[A-Za-z0-9][A-Za-z0-9_.-]*/[A-Za-z0-9][A-Za-z0-9_.-]*\z")?
            .is_match(repo),
        "repository must be OWNER/REPO"
    );
    ensure!(
        sha.len() == 40 && sha.bytes().all(|b| b.is_ascii_hexdigit()),
        "expected full hexadecimal SHA"
    );
    for value in [timeout, poll] {
        ensure!(
            value.is_finite() && value >= 0.0 && Duration::try_from_secs_f64(value).is_ok(),
            "time budget must be finite, nonnegative and representable"
        );
    }
    Ok(())
}
fn identity(run: &Value, sha: &str) -> Result<()> {
    ensure!(
        run["head_sha"].as_str().unwrap_or("").to_ascii_lowercase() == sha
            && run["event"] == "push"
            && matches!(run["head_branch"].as_str(), Some("main" | "master"))
            && matches!(
                run["path"].as_str(),
                Some(
                    ".github/workflows/ci.yml"
                        | ".github/workflows/ci.yml@main"
                        | ".github/workflows/ci.yml@master"
                )
            ),
        "CI run identity does not match exact SHA/main push ci.yml"
    );
    for key in ["id", "run_number", "run_attempt"] {
        ensure!(
            run[key].as_u64().is_some_and(|n| n > 0),
            "invalid CI run {key}"
        );
    }
    Ok(())
}
fn status(item: &Value, label: &str) -> Result<Option<String>> {
    ensure!(
        item["conclusion"].is_null() || item["conclusion"] == "success",
        "{label} has non-success conclusion"
    );
    if item["status"] == "completed" {
        ensure!(
            item["conclusion"] == "success",
            "{label} completed without success"
        );
        return Ok(None);
    }
    if PENDING.contains(&item["status"].as_str().unwrap_or("")) && item["conclusion"].is_null() {
        return Ok(Some(format!("{label} is {}", item["status"])));
    }
    bail!("{label} has invalid status/conclusion")
}
fn run_status(run: &Value, preview: bool) -> Result<Option<String>> {
    if preview && run["status"] == "completed" && run["conclusion"] == "failure" {
        Ok(None)
    } else {
        status(run, "CI run")
    }
}
fn job_status(job: &Value, preview: bool) -> Result<Option<String>> {
    let name = job["name"].as_str().unwrap_or("unnamed job");
    let allowed = advisory(name);
    if !preview
        || allowed.is_empty()
        || job["status"] != "completed"
        || job["conclusion"] != "failure"
    {
        return status(job, name);
    }
    let steps = job["steps"]
        .as_array()
        .filter(|v| !v.is_empty())
        .context("missing advisory step evidence")?;
    let mut failed = vec![];
    for step in steps {
        ensure!(step.is_object(), "malformed step evidence");
        let title = step["name"].as_str().unwrap_or("");
        if step["status"] == "completed"
            && step["conclusion"] == "skipped"
            && optional(name).contains(&title)
        {
            continue;
        }
        if step["status"] == "completed"
            && step["conclusion"] == "failure"
            && allowed.contains(&title)
        {
            failed.push(title);
        } else {
            ensure!(status(step, title)?.is_none(), "incomplete step evidence");
        }
    }
    ensure!(
        !failed.is_empty(),
        "failed without allowlisted advisory evidence"
    );
    println!("ADVISORY (preview only): {name}: {}", failed.join(", "));
    Ok(None)
}
pub fn check_once(
    repo: &str,
    sha: &str,
    api: &mut impl Github,
    preview: bool,
) -> Result<Option<String>> {
    let base = format!("repos/{repo}/actions");
    let runs = objects(
        api,
        &format!("{base}/workflows/ci.yml/runs?head_sha={sha}&event=push&per_page=100"),
        "workflow_runs",
    )?;
    let candidates: Vec<_> = runs
        .iter()
        .filter(|run| {
            run["head_sha"].as_str().unwrap_or("").to_ascii_lowercase() == sha
                && run["event"] == "push"
                && matches!(run["head_branch"].as_str(), Some("main" | "master"))
        })
        .collect();
    for run in &candidates {
        identity(run, sha)?;
    }
    let Some(latest) = candidates.into_iter().max_by_key(|r| {
        (
            r["run_number"].as_u64(),
            r["id"].as_u64(),
            r["run_attempt"].as_u64(),
        )
    }) else {
        return Ok(Some("missing exact-SHA main push CI run".into()));
    };
    let endpoint = format!("{base}/runs/{}", latest["id"]);
    let run = api.api(&endpoint, false)?;
    identity(&run, sha)?;
    ensure!(
        run["id"] == latest["id"] && run["run_attempt"].as_u64() >= latest["run_attempt"].as_u64(),
        "CI detail does not match latest run/attempt"
    );
    let reason = run_status(&run, preview)?;
    let jobs = objects(
        api,
        &format!(
            "{endpoint}/attempts/{}/jobs?per_page=100",
            run["run_attempt"]
        ),
        "jobs",
    )?;
    let mut pending = vec![];
    if let Some(reason) = reason {
        pending.push(reason);
    }
    for name in MANDATORY {
        let matches: Vec<_> = jobs.iter().filter(|j| j["name"] == *name).collect();
        if matches.is_empty() {
            ensure!(
                run["status"] != "completed",
                "Completed CI run missing mandatory job: {name}"
            );
            pending.push(format!("missing mandatory job: {name}"));
            continue;
        }
        ensure!(matches.len() == 1, "Ambiguous mandatory job: {name}");
        let job = matches[0];
        for (key, expected) in [
            ("run_attempt", &run["run_attempt"]),
            ("run_id", &run["id"]),
            ("head_sha", &Value::String(sha.into())),
        ] {
            ensure!(
                job.get(key).is_none_or(|v| v == expected),
                "Wrong {key} for mandatory job: {name}"
            );
        }
        if let Some(reason) = job_status(job, preview)? {
            pending.push(reason);
        }
    }
    if preview {
        ensure!(
            run["conclusion"] != "failure" || jobs.iter().any(|j| j["conclusion"] == "failure"),
            "Failed CI run has no failed-job evidence"
        );
        for job in jobs
            .iter()
            .filter(|j| !MANDATORY.contains(&j["name"].as_str().unwrap_or("")))
        {
            if let Some(reason) = job_status(job, true)? {
                pending.push(reason);
            }
        }
    }
    if !pending.is_empty() {
        return Ok(Some(pending.join("; ")));
    }
    let current = api.api(&endpoint, false)?;
    identity(&current, sha)?;
    ensure!(current["id"] == run["id"], "CI run changed identity");
    let confirmation = run_status(&current, preview)?;
    if current["run_attempt"] != run["run_attempt"] {
        return Ok(Some("CI run attempt changed during verification".into()));
    }
    Ok(confirmation)
}
#[derive(Debug)]
pub struct WaitExpired(pub String);
impl std::fmt::Display for WaitExpired {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "WAIT EXPIRED: {}", self.0)
    }
}
impl std::error::Error for WaitExpired {}
pub fn authorize(
    repo: &str,
    sha: &str,
    timeout: f64,
    poll: f64,
    api: &mut impl Github,
    preview: bool,
) -> Result<()> {
    validate(repo, sha, timeout, poll)?;
    let sha = sha.to_ascii_lowercase();
    let start = Instant::now();
    let budget = Duration::try_from_secs_f64(timeout)?;
    loop {
        let Some(reason) = check_once(repo, &sha, api, preview)? else {
            println!("AUTHORIZED: exact-SHA main push CI for {sha}");
            return Ok(());
        };
        let remaining = budget.saturating_sub(start.elapsed());
        if remaining.is_zero() {
            return Err(WaitExpired(reason).into());
        }
        println!("WAITING: {reason}");
        std::thread::sleep(Duration::try_from_secs_f64(poll)?.min(remaining));
    }
}
#[derive(Parser)]
struct Args {
    #[arg(conflicts_with = "sha", required_unless_present = "sha")]
    tag_sha: Option<String>,
    #[arg(long, conflicts_with = "tag_sha")]
    sha: Option<String>,
    #[arg(long, alias = "repo")]
    repository: String,
    #[arg(long, default_value_t = 7200.0)]
    timeout: f64,
    #[arg(long, default_value_t = 30.0)]
    poll_interval: f64,
    #[arg(long)]
    preview: bool,
}
pub fn run(args: impl Iterator<Item = String>) -> Result<()> {
    let a = Args::parse_from(std::iter::once("check-release-ci".into()).chain(args));
    let sha = a.sha.or(a.tag_sha).context("missing SHA")?;
    if let Err(e) = validate(&a.repository, &sha, a.timeout, a.poll_interval) {
        eprintln!("{e}");
        std::process::exit(2);
    }
    if let Err(e) = authorize(
        &a.repository,
        &sha,
        a.timeout,
        a.poll_interval,
        &mut Gh,
        a.preview,
    ) {
        if e.is::<WaitExpired>() {
            eprintln!("{e}");
            std::process::exit(2);
        }
        return Err(e);
    }
    Ok(())
}
#[cfg(test)]
mod tests;
