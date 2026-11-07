//! Rust orchestration for the external Performance Take-Home dataset.
use super::{debug_fixture::OwnedSession, io};
use anyhow::{Context, Result, ensure};
use clap::Parser;
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};
const BASELINE: u64 = 147734;
#[derive(Parser)]
struct Args {
    #[arg(default_value="both",value_parser=["single","swarm","both"])]
    mode: String,
    #[arg(long, conflicts_with = "swarm_only")]
    single_only: bool,
    #[arg(long, conflicts_with = "single_only")]
    swarm_only: bool,
    #[arg(long)]
    socket: Option<PathBuf>,
    #[arg(long)]
    source: Option<PathBuf>,
    #[arg(long)]
    output: Option<PathBuf>,
    #[arg(long)]
    timeout: Option<f64>,
    #[arg(long, default_value_t = 30.)]
    check_interval: f64,
    /// Shell-quoted argv, executed directly without a shell. This evaluates an external dataset.
    #[arg(long, default_value = "python3 tests/submission_tests.py -v")]
    evaluator_command: String,
    #[arg(long, default_value_t = 120.)]
    evaluator_timeout: f64,
}
fn call(socket: &Path, cmd: &str, session: Option<&str>) -> Result<Value> {
    let v = io::debug_response(socket, cmd, session, Duration::from_secs(30))?;
    ensure!(v["ok"] != false && v["type"] != "error", "{cmd}: {v}");
    Ok(serde_json::from_str(
        v["output"].as_str().context("debug output")?,
    )?)
}
fn copy_tree(source: &Path, dest: &Path) -> Result<()> {
    std::fs::create_dir_all(dest)?;
    for e in std::fs::read_dir(source)? {
        let e = e?;
        if e.file_name() == ".git" {
            continue;
        }
        let ty = e.file_type()?;
        ensure!(
            !ty.is_symlink(),
            "dataset symlinks are not copied outside the isolated workspace: {}",
            e.path().display()
        );
        if ty.is_dir() {
            copy_tree(&e.path(), &dest.join(e.file_name()))?
        } else if ty.is_file() {
            std::fs::copy(e.path(), dest.join(e.file_name()))?;
        }
    }
    Ok(())
}
fn evaluate(workspace: &Path, argv: &[String], timeout: f64) -> Result<Value> {
    let mut cmd = Command::new(&argv[0]);
    cmd.args(&argv[1..]).current_dir(workspace);
    let output = crate::process::capture(&mut cmd, Duration::from_secs_f64(timeout))?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    ensure!(output.status.success(), "external evaluator failed: {text}");
    let re = regex::Regex::new(r"(?m)CYCLES:\s*(\d+)").unwrap();
    let cycles = re
        .captures(&text)
        .context("evaluator emitted no CYCLES count")?[1]
        .parse::<u64>()?;
    ensure!(cycles > 0, "zero cycle count is invalid");
    Ok(json!({"cycles":cycles,"output":text}))
}
fn prompt(workspace: &Path, evaluator: &str, swarm: bool) -> String {
    format!(
        "Optimize build_kernel() in perf_takehome.py to minimize cycle count on the VLIW SIMD simulator. Historical baseline: {BASELINE} cycles. Work ONLY in {}. Read problem.py to determine actual vector width, core count, engine slot limits and scratch limits; do not assume a fixed VLEN. Focus on vectorization, VLIW packing, loop unrolling, hash stages, and memory access. Do not modify tests/. After each change verify correctness and cycles with: {evaluator}. {}",
        workspace.display(),
        if swarm {
            "Break the optimization into parallel subtasks, coordinate agents, and integrate verified improvements."
        } else {
            "Work efficiently on the highest-impact optimizations."
        }
    )
}
fn run_trial(
    a: &Args,
    socket: &Path,
    source: &Path,
    out: &Path,
    argv: &[String],
    mode: &str,
    seconds: f64,
) -> Result<Value> {
    let workspace = out.join(mode);
    ensure!(
        !workspace.exists(),
        "trial workspace exists; use a new output directory"
    );
    copy_tree(source, &workspace)?;
    // A repo is sufficient for swarm provenance; no fabricated commit identity.
    let git = crate::process::capture(
        Command::new("git").arg("init").current_dir(&workspace),
        Duration::from_secs(10),
    )?;
    ensure!(git.status.success(), "git init failed");
    let initial = evaluate(&workspace, argv, a.evaluator_timeout)?;
    let mut best = initial["cycles"].as_u64().unwrap();
    let mut samples = vec![initial];
    let start = Instant::now();
    let mut session = OwnedSession::new(socket, &workspace)?;
    let swarm = call(socket, &format!("swarm:session:{}", session.id), None)?["swarm_id"].clone();
    ensure!(swarm.is_string(), "owned swarm identity missing");
    let result = (|| {
        let command = if mode == "swarm" {
            "swarm_message_async"
        } else {
            "message_async"
        };
        let job = call(
            socket,
            &format!(
                "{command}:{}",
                prompt(&workspace, &a.evaluator_command, mode == "swarm")
            ),
            Some(&session.id),
        )?;
        let job_id = job["job_id"].as_str().context("job id missing")?;
        let mut status = String::from("running");
        let mut job_error = Value::Null;
        let mut evaluation_errors = vec![];
        while start.elapsed().as_secs_f64() < seconds {
            let state = call(socket, &format!("job_status:{job_id}"), Some(&session.id))?;
            status = state["status"]
                .as_str()
                .context("job status missing")?
                .into();
            if status == "completed" || status == "failed" {
                job_error = state["error"].clone();
                break;
            }
            match evaluate(
                &workspace,
                argv,
                a.evaluator_timeout
                    .min((seconds - start.elapsed().as_secs_f64()).max(0.01)),
            ) {
                Ok(v) => {
                    best = best.min(v["cycles"].as_u64().unwrap());
                    samples.push(v)
                }
                Err(e) => evaluation_errors.push(format!("{e:#}")),
            }
            std::thread::sleep(Duration::from_secs_f64(
                a.check_interval
                    .min((seconds - start.elapsed().as_secs_f64()).max(0.)),
            ));
        }
        let timed_out = status != "completed" && status != "failed";
        // Stop owned inference before evaluating the final artifact, avoiding writes during scoring.
        call(socket, "cancel", Some(&session.id))?;
        session.close()?;
        let members = call(socket, "swarm:members", None)?;
        for m in members.as_array().context("swarm members array")? {
            if m["swarm_id"] == swarm
                && let Some(id) = m["session_id"].as_str()
            {
                call(socket, "cancel", Some(id))?;
                call(socket, &format!("destroy_session:{id}"), None)?;
            }
        }
        let final_result = evaluate(&workspace, argv, a.evaluator_timeout)?;
        let final_cycles = final_result["cycles"].as_u64().unwrap();
        best = best.min(final_cycles);
        samples.push(final_result);
        let thresholds = [
            ("Baseline", 147734),
            ("Updated starter", 18532),
            ("Opus 4 many hours", 2164),
            ("Opus 4.5 casual", 1790),
            ("Opus 4.5 2hr harness", 1579),
            ("Sonnet 4.5 many hours", 1548),
            ("Opus 4.5 11.5hr harness", 1487),
            ("Opus 4.5 improved harness", 1363),
        ];
        Ok(
            json!({"approach":mode,"cycles":best,"final_cycles":final_cycles,"time_seconds":start.elapsed().as_secs_f64(),"workspace":workspace,"speedup":BASELINE as f64/best as f64,"job_status":status,"job_error":job_error,"timed_out":timed_out,"passed":status=="completed","evaluations":samples,"evaluation_errors":evaluation_errors,"historical_thresholds":thresholds.iter().map(|(name,n)|json!({"name":name,"cycles":n,"passed":best<*n})).collect::<Vec<_>>()}),
        )
    })();
    // Also clean up spawned agents when a debug/evaluator error aborts the run.
    if result.is_err() {
        let _ = call(socket, "cancel", Some(&session.id));
    }
    let cleanup = session.close();
    if let Ok(members) = call(socket, "swarm:members", None)
        && let Some(rows) = members.as_array()
    {
        for m in rows {
            if m["swarm_id"] == swarm
                && let Some(id) = m["session_id"].as_str()
            {
                let _ = call(socket, "cancel", Some(id));
                let _ = call(socket, &format!("destroy_session:{id}"), None);
            }
        }
    }

    result.and_then(|v| {
        cleanup?;
        Ok(v)
    })
}
pub(super) fn run(args: impl Iterator<Item = String>) -> Result<i32> {
    let a = Args::try_parse_from(std::iter::once("benchmark-swarm".into()).chain(args))?;
    let timeout = a
        .timeout
        .or_else(|| {
            std::env::var("BENCHMARK_TIMEOUT")
                .ok()
                .and_then(|s| s.parse().ok())
        })
        .unwrap_or(10.);
    for n in [timeout, a.check_interval, a.evaluator_timeout] {
        ensure!(
            n.is_finite() && n > 0.,
            "timings must be positive and finite"
        );
    }
    let argv = shlex::split(&a.evaluator_command).context("invalid evaluator quoting")?;
    ensure!(!argv.is_empty(), "empty evaluator command");
    let source = a
        .source
        .clone()
        .or_else(|| std::env::var_os("TAKEHOME_SOURCE").map(PathBuf::from))
        .unwrap_or(
            dirs::home_dir()
                .context("home directory")?
                .join("original_performance_takehome"),
        )
        .canonicalize()?;
    let socket = a.socket.clone().unwrap_or_else(|| {
        PathBuf::from(format!("/run/user/{}/nexode-debug.sock", unsafe {
            libc::getuid()
        }))
    });
    let output = if let Some(p) = &a.output {
        std::fs::create_dir_all(p)?;
        p.canonicalize()?
    } else {
        tempfile::Builder::new()
            .prefix("takehome-benchmark-")
            .tempdir()?
            .keep()
    };
    ensure!(
        !output.starts_with(&source),
        "output must not be inside source dataset"
    );
    let modes = if a.single_only || a.mode == "single" {
        vec!["single"]
    } else if a.swarm_only || a.mode == "swarm" {
        vec!["swarm"]
    } else {
        vec!["single", "swarm"]
    };
    let mut results = json!({});
    for mode in modes {
        results[mode] = match run_trial(&a, &socket, &source, &output, &argv, mode, timeout * 60.) {
            Ok(v) => v,
            Err(e) => {
                json!({"approach":mode,"passed":false,"error":format!("{e:#}"),"cycles":null})
            }
        };
    }
    let passed = results
        .as_object()
        .unwrap()
        .values()
        .all(|r| r["passed"] == true);
    std::fs::write(
        output.join("results.json"),
        serde_json::to_vec_pretty(&results)?,
    )?;
    println!("{}", serde_json::to_string_pretty(&results)?);
    println!(
        "{:<12} {:>12} {:>12} {:>12}",
        "Approach", "Best cycles", "Seconds", "Speedup"
    );
    for (name, row) in results.as_object().unwrap() {
        println!(
            "{name:<12} {:>12} {:>12} {:>12}",
            row["cycles"], row["time_seconds"], row["speedup"]
        );
    }
    if let Some((winner, row)) = results
        .as_object()
        .unwrap()
        .iter()
        .filter(|(_, r)| r["passed"] == true && r["cycles"].as_u64().is_some())
        .min_by_key(|(_, r)| r["cycles"].as_u64().unwrap())
    {
        println!(
            "Best completed approach: {winner} ({} cycles)",
            row["cycles"]
        );
    }
    println!("Results: {}", output.display());
    Ok(i32::from(!passed))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn evaluator_requires_success_positive_cycles_and_copies_without_mutating_source() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        std::fs::create_dir(&source).unwrap();
        std::fs::write(source.join("data"), "original").unwrap();
        let dest = root.path().join("copy");
        copy_tree(&source, &dest).unwrap();
        std::fs::write(dest.join("data"), "changed").unwrap();
        assert_eq!(
            std::fs::read_to_string(source.join("data")).unwrap(),
            "original"
        );
        let argv = vec!["sh".into(), "-c".into(), "echo CYCLES: 123".into()];
        assert_eq!(evaluate(&dest, &argv, 2.).unwrap()["cycles"], 123);
        for cmd in ["echo CYCLES: 0", "echo missing", "echo CYCLES: 12; exit 1"] {
            assert!(evaluate(&dest, &["sh".into(), "-c".into(), cmd.into()], 2.).is_err());
        }
    }
}
