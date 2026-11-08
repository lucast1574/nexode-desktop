use super::isolated::{Session, pause};
use anyhow::{Result, ensure};
use clap::Parser;
use serde_json::Value;
use std::{path::PathBuf, process::Command, time::Duration};
#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "./target/selfdev/nexode")]
    binary: PathBuf,
    #[arg(long, default_value_t = 8.0)]
    seconds: f64,
    #[arg(long, default_value_t = 997)]
    freq: u32,
    #[arg(long)]
    palette: bool,
    #[arg(long, default_value = "perf")]
    perf: PathBuf,
}
pub(super) fn run(args: impl Iterator<Item = String>) -> Result<i32> {
    let a = Args::parse_from(std::iter::once("profile-idle-donut".into()).chain(args));
    ensure!(
        a.seconds.is_finite() && a.seconds > 0.0 && a.seconds <= 3600.0 && a.freq > 0,
        "invalid sampling duration/frequency"
    );
    let binary = a.binary.canonicalize()?;
    let session = Session::with_environment(
        &binary,
        48,
        160,
        false,
        &[("NEXODE_IDLE_ANIMATION", "true")],
    )?;
    let stats: Value = serde_json::from_str(&session.command("draw-stats 1")?)?;
    println!("redraw schedule: {}", stats["redraw_schedule"]);
    if stats["redraw_schedule"]["idle_animation_active"] != true {
        eprintln!(
            "WARNING: animation is inactive; this profile does not represent animation cost."
        );
    }
    if a.palette {
        session.client.send(b"/")?;
        pause(1.0)?;
    }
    let scratch = std::env::var_os("NEXODE_SCRATCH_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let root = tempfile::Builder::new()
        .prefix("orbit-idle-perf-")
        .tempdir_in(scratch)?
        .keep();
    let data = root.join("perf.data");
    println!("artifacts: {}", root.display());
    let cancelled = || crate::ci_suites::unix::interrupted().is_some();
    let record = crate::process::capture_interruptible(
        Command::new(&a.perf)
            .args([
                "record",
                "-F",
                &a.freq.to_string(),
                "-g",
                "--pid",
                &session.client.pid.to_string(),
                "-o",
            ])
            .arg(&data)
            .args(["--", "sleep", &a.seconds.to_string()]),
        Duration::from_secs_f64(a.seconds + 10.0),
        cancelled,
    )?;
    if !record.status.success() || !data.is_file() {
        eprintln!(
            "perf record failed: {}",
            String::from_utf8_lossy(&record.stderr)
                .chars()
                .take(2000)
                .collect::<String>()
        );
        return Ok(3);
    }
    for (title, children, limit) in [
        ("self time (flat)", "--no-children", "0.7"),
        ("inclusive time (children)", "--children", "2.0"),
    ] {
        let report = crate::process::capture_interruptible(
            Command::new(&a.perf)
                .args(["report", "-i"])
                .arg(&data)
                .args(["--stdio", children, "--percent-limit", limit, "-g", "none"]),
            Duration::from_secs(60),
            cancelled,
        )?;
        ensure!(report.status.success(), "perf report failed: {title}");
        let text = String::from_utf8_lossy(&report.stdout)
            .lines()
            .filter(|s| !s.trim().is_empty() && !s.starts_with('#'))
            .collect::<Vec<_>>()
            .join("\n");
        println!(
            "\n=== {title} ===\n{}",
            text.chars().take(4000).collect::<String>()
        );
    }
    Ok(0)
}
