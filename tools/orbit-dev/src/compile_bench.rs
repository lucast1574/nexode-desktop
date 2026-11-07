//! Build-loop timing with source restoration on errors and signals.
use anyhow::{Context, Result, ensure};
use clap::Parser;
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant, SystemTime},
};
#[derive(Parser, Clone)]
struct Args {
    #[arg(value_parser=["check","build","release-nexode","selfdev-nexode"])]
    target: String,
    #[arg(long)]
    cold: bool,
    #[arg(long, conflicts_with = "edit")]
    touch: Option<PathBuf>,
    #[arg(long)]
    edit: Option<PathBuf>,
    #[arg(long, default_value_t = 1)]
    runs: usize,
    #[arg(long)]
    json: bool,
    #[arg(long)]
    repo: Option<PathBuf>,
    #[arg(last = true)]
    extra: Vec<String>,
}
struct Restore {
    path: PathBuf,
    bytes: Vec<u8>,
}
impl Drop for Restore {
    fn drop(&mut self) {
        if let Err(e) = fs::write(&self.path, &self.bytes) {
            eprintln!("failed to restore {}: {e}", self.path.display());
        }
    }
}
fn command(a: &Args) -> Vec<String> {
    let args: Vec<&str> = match a.target.as_str() {
        "check" => vec!["cargo", "check", "--quiet"],
        "build" => vec!["cargo", "build", "--quiet"],
        "release-nexode" => vec![
            "scripts/dev_cargo.sh",
            "build",
            "--release",
            "-p",
            "nexode",
            "--bin",
            "nexode",
            "--quiet",
        ],
        _ => vec![
            "scripts/dev_cargo.sh",
            "build",
            "--profile",
            "selfdev",
            "-p",
            "nexode",
            "--bin",
            "nexode",
            "--quiet",
        ],
    };
    args.into_iter()
        .map(str::to_owned)
        .chain(a.extra.clone())
        .collect()
}
fn interrupted() -> bool {
    #[cfg(unix)]
    {
        crate::ci_suites::unix::interrupted().is_some()
    }
    #[cfg(not(unix))]
    {
        false
    }
}
fn execute(root: &Path, argv: &[String]) -> Result<(i32, String)> {
    let out = crate::process::capture_interruptible(
        Command::new(&argv[0]).args(&argv[1..]).current_dir(root),
        Duration::from_secs(10800),
        interrupted,
    )?;
    let log = String::from_utf8_lossy(&out.stderr).into_owned();
    eprint!("{}{}", String::from_utf8_lossy(&out.stdout), log);
    Ok((out.status.code().unwrap_or(1), log))
}
fn measure(root: &Path, a: &Args) -> Result<(i32, Value)> {
    ensure!(a.runs > 0, "runs must be positive");
    let backup = a
        .edit
        .as_ref()
        .map(|p| {
            let path = root.join(p);
            Ok::<_, anyhow::Error>(Restore {
                bytes: fs::read(&path)?,
                path,
            })
        })
        .transpose()?;
    if let Some(p) = &a.touch {
        ensure!(root.join(p).exists(), "touch path must exist");
    }
    if a.cold {
        let (code, log) = execute(root, &["cargo".into(), "clean".into()])?;
        if code != 0 {
            return Ok((code, json!({"error":log,"exit_code":code})));
        }
    }
    let argv = command(a);
    let mut times = vec![];
    for i in 0..a.runs {
        ensure!(!interrupted(), "benchmark interrupted");
        if let Some(p) = &a.touch {
            fs::File::open(root.join(p))?.set_modified(SystemTime::now())?;
        }
        if let Some(b) = &backup {
            let mut bytes = b.bytes.clone();
            if i % 2 == 0 {
                bytes.push(b'\n');
            }
            fs::write(&b.path, bytes)?;
        }
        let start = Instant::now();
        let (code, log) = execute(root, &argv)?;
        if code != 0 {
            return Ok((code, json!({"error":log,"exit_code":code})));
        }
        times.push((start.elapsed().as_secs_f64() * 1000.0).round() / 1000.0);
    }
    let mut sorted = times.clone();
    sorted.sort_by(f64::total_cmp);
    let n = sorted.len();
    let median = if n % 2 == 0 {
        (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0
    } else {
        sorted[n / 2]
    };
    Ok((
        0,
        json!({"target":a.target,"cold":a.cold,"touch":a.touch,"edit":a.edit,"runs":a.runs,"command":argv.join(" "),"times_seconds":times,"min_seconds":sorted[0],"max_seconds":sorted[n-1],"avg_seconds":times.iter().sum::<f64>()/n as f64,"median_seconds":median}),
    ))
}
pub fn run(root: &Path, args: impl Iterator<Item = String>) -> Result<i32> {
    let a = Args::parse_from(std::iter::once("bench-compile".into()).chain(args));
    #[cfg(unix)]
    let _signals = crate::ci_suites::unix::Signals::install()?;
    let (code, report) = measure(a.repo.as_deref().unwrap_or(root), &a)?;
    if a.json {
        println!("{report}");
    } else {
        eprintln!("bench_compile: {}", serde_json::to_string_pretty(&report)?);
    }
    Ok(code)
}
pub fn checkpoints(root: &Path, args: impl Iterator<Item = String>) -> Result<()> {
    #[derive(Parser)]
    struct Checkpoints {
        #[arg(long, default_value = "src/server.rs")]
        touch: PathBuf,
        #[arg(long, default_value_t = 3)]
        runs: usize,
        #[arg(long)]
        skip_cold: bool,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        repo: Option<PathBuf>,
    }
    let a =
        Checkpoints::parse_from(std::iter::once("bench-selfdev-checkpoints".into()).chain(args));
    #[cfg(unix)]
    let _signals = crate::ci_suites::unix::Signals::install()?;
    ensure!(a.runs > 0, "runs must be positive");
    let root = a.repo.as_deref().unwrap_or(root);
    ensure!(root.join(&a.touch).exists(), "touch path must exist");
    let mut reports = json!({});
    let mut failed = vec![];
    for (name, target, cold) in [
        ("cold_check", "check", true),
        ("cold_selfdev_build", "selfdev-nexode", true),
        ("warm_check_edit", "check", false),
        ("warm_selfdev_edit", "selfdev-nexode", false),
    ] {
        if cold && a.skip_cold {
            reports[name] = json!({"checkpoint":name,"ok":null,"skipped":true});
            continue;
        }
        let args = Args {
            target: target.into(),
            cold,
            touch: (!cold).then(|| a.touch.clone()),
            edit: None,
            runs: if cold { 1 } else { a.runs },
            json: true,
            repo: None,
            extra: vec![],
        };
        let (code, mut report) =
            measure(root, &args).with_context(|| format!("checkpoint {name}"))?;
        report["checkpoint"] = json!(name);
        report["ok"] = json!(code == 0);
        if code != 0 {
            failed.push(name);
        }
        reports[name] = report;
    }
    let report = json!({"touch_path":a.touch,"warm_runs":a.runs,"skip_cold":a.skip_cold,"checkpoints":reports,"failed_checkpoints":failed});
    if a.json {
        println!("{report}");
    } else {
        println!(
            "selfdev compile checkpoints\n{}",
            serde_json::to_string_pretty(&report)?
        );
    }
    Ok(())
}
