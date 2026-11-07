//! Timed CI-style suites with process-group cleanup on timeout or interruption.
use anyhow::{Result, ensure};
use clap::Parser;
use serde_json::json;
use std::{
    io::Write,
    path::Path,
    process::Command,
    time::{Duration, Instant},
};
#[cfg(unix)]
mod unix;
const SUITES: [(&str, u64, &[&str]); 3] = [
    ("lib-bins", 1800, &["test", "--locked", "--lib", "--bins"]),
    (
        "provider-matrix",
        900,
        &["test", "--locked", "--test", "provider_matrix"],
    ),
    ("e2e", 1800, &["test", "--locked", "--test", "e2e"]),
];
#[derive(Parser)]
struct Args {
    #[arg(value_parser=["all","lib-bins","provider-matrix","e2e"])]
    suite: Vec<String>,
    #[arg(long, default_value_t = 1.0)]
    timeout_scale: f64,
    #[arg(long)]
    parallel: bool,
}
fn progress(message: &str, current: usize, total: usize, eta: Option<u64>) -> Result<()> {
    let mut p = json!({"kind":"indeterminate","message":message,"current":current,"total":total,"unit":"suite"});
    if let Some(eta) = eta {
        p["eta_seconds"] = json!(eta);
    }
    println!("NEXODE_PROGRESS {p}");
    std::io::stdout().flush()?;
    Ok(())
}
pub fn run(root: &Path, args: impl Iterator<Item = String>) -> Result<i32> {
    let args = Args::parse_from(std::iter::once("test-ci-suites".into()).chain(args));
    ensure!(
        args.timeout_scale.is_finite() && args.timeout_scale > 0.0 && args.timeout_scale <= 1000.0,
        "timeout scale must be positive, finite and at most 1000"
    );
    let selected: Vec<_> = if args.suite.is_empty() || args.suite.iter().any(|s| s == "all") {
        SUITES.iter().collect()
    } else {
        args.suite
            .iter()
            .map(|n| SUITES.iter().find(|s| s.0 == n).unwrap())
            .collect()
    };
    #[cfg(unix)]
    let _signals = unix::Signals::install()?;
    let total = Instant::now();
    let mut completed = 0;
    for (index, (name, seconds, cargo_args)) in selected.iter().enumerate() {
        let timeout = ((*seconds as f64 * args.timeout_scale) as u64).max(1);
        progress(
            &format!("Running {name} ({}/{})", index + 1, selected.len()),
            index + 1,
            selected.len(),
            None,
        )?;
        progress(&format!("Running {name}"), 0, 1, Some(timeout))?;
        let mut cmd = Command::new("cargo");
        cmd.current_dir(root).args(*cargo_args);
        if !args.parallel {
            cmd.args(["--", "--test-threads=1"]);
        }
        println!(
            "\n=== {name} ===\n$ cargo {}",
            cmd.get_args()
                .map(|a| a.to_string_lossy())
                .collect::<Vec<_>>()
                .join(" ")
        );
        std::io::stdout().flush()?;
        let started = Instant::now();
        #[cfg(unix)]
        let code = unix::execute(&mut cmd, Duration::from_secs(timeout))?;
        #[cfg(not(unix))]
        let code = {
            let _ = cmd;
            anyhow::bail!("test-ci-suites requires Unix process groups");
        };
        completed += 1;
        println!(
            "=== {name} exit={code} elapsed={:.1}s timeout={timeout}s ===",
            started.elapsed().as_secs_f64()
        );
        if code != 0 {
            println!(
                "\n=== test suite summary ===\nsuites={completed}/{} elapsed={:.1}s\nFAILED {name}: exit={code}",
                selected.len(),
                total.elapsed().as_secs_f64()
            );
            return Ok(code);
        }
    }
    println!(
        "\n=== test suite summary ===\nsuites={completed} elapsed={:.1}s\nAll selected CI-style test suites passed.",
        total.elapsed().as_secs_f64()
    );
    Ok(0)
}
