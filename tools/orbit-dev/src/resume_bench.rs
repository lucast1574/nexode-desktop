//! Resume benchmarks: real PTY picker and existing offline Rust test matrices.
use anyhow::{Context, Result, ensure};
use clap::Parser;
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};
#[derive(Parser)]
struct PickerArgs {
    binary: PathBuf,
    #[arg(long, default_value_t = 3)]
    runs: usize,
    #[arg(long, default_value_t = 30.0)]
    timeout: f64,
    #[arg(long)]
    cancel_loading: bool,
}
fn median(mut values: Vec<f64>) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(f64::total_cmp);
    let m = values.len() / 2;
    Some(if values.len().is_multiple_of(2) {
        (values[m - 1] + values[m]) / 2.0
    } else {
        values[m]
    })
}
#[cfg(unix)]
fn measure(binary: &Path, timeout: Duration, cancel: bool) -> Result<Value> {
    use std::os::unix::process::ExitStatusExt;
    use std::time::Instant;
    let mut command = Command::new(binary);
    command
        .args(["--resume", "--no-update"])
        .env("TERM", "xterm-256color")
        .env("COLORTERM", "truecolor");
    let start = Instant::now();
    let mut terminal = crate::pty::Terminal::spawn(&mut command, 40, 120)?;
    let ansi = regex::bytes::RegexBuilder::new(
        r"\x1b\[[0-?]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)",
    )
    .unicode(false)
    .build()?;
    let populated_pattern = regex::bytes::Regex::new(r"\b[1-9][0-9]*\s+sessions\b")?;
    let mut output = Vec::new();
    let (mut first, mut populated, mut cancelled) = (None, None, None);
    while start.elapsed() < timeout {
        let chunk = terminal.read(Duration::from_millis(20))?;
        if !chunk.is_empty() {
            for (query, reply) in [
                (
                    b"\x1b]11;?\x07".as_slice(),
                    b"\x1b]11;rgb:0000/0000/0000\x07".as_slice(),
                ),
                (
                    b"\x1b]11;?\x1b\\".as_slice(),
                    b"\x1b]11;rgb:0000/0000/0000\x1b\\".as_slice(),
                ),
            ] {
                if chunk.windows(query.len()).any(|w| w == query) {
                    terminal.write(reply)?;
                }
            }
            output.extend(chunk);
            ensure!(
                output.len() <= 16 * 1024 * 1024,
                "picker output exceeded 16 MiB"
            );
            let text = ansi.replace_all(&output, b"".as_slice());
            let elapsed = start.elapsed().as_secs_f64() * 1000.0;
            if first.is_none()
                && text
                    .windows(b"(s/S filter)".len())
                    .any(|w| w == b"(s/S filter)")
            {
                first = Some(elapsed);
                if cancel {
                    terminal.write(b"q")?;
                    cancelled = Some(Instant::now());
                }
            }
            if populated.is_none() && populated_pattern.is_match(&text) {
                populated = Some(elapsed);
                if !cancel {
                    terminal.write(b"q")?;
                    cancelled = Some(Instant::now());
                }
            }
        }
        if cancelled.is_some() && output.windows(8).any(|w| w == b"\x1b[?1049l") {
            break;
        }
        if terminal.eof {
            break;
        }
        if terminal.status()?.is_some() {
            // Exit can race the last PTY write. Drain it before checking restoration.
            output.extend(terminal.read(Duration::from_millis(20))?);
            break;
        }
    }
    let deadline = Instant::now() + Duration::from_millis(500);
    let status = loop {
        let s = terminal.status()?;
        if s.is_some() || Instant::now() >= deadline {
            break s;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let exit = status.map(|s| s.code().unwrap_or_else(|| -s.signal().unwrap_or(1)));
    let restored = output.windows(8).any(|w| w == b"\x1b[?1049l");
    Ok(
        json!({"first_frame_ms":first,"populated_list_ms":populated,"cancel_exit_ms":cancelled.map(|c|(c.elapsed().as_secs_f64()*100000.0).round()/100.0),"exit_code":exit,"terminal_restored":restored}),
    )
}
#[cfg(not(unix))]
fn measure(_binary: &Path, _timeout: Duration, _cancel: bool) -> Result<Value> {
    anyhow::bail!("PTY resume benchmark currently requires Unix")
}
fn picker(binary: &Path, runs: usize, timeout: f64, cancel: bool) -> Result<String> {
    ensure!(
        runs > 0 && timeout.is_finite() && timeout > 0.0,
        "runs and timeout must be positive"
    );
    let mut samples = Vec::new();
    let mut text = String::new();
    for _ in 0..runs {
        let sample = measure(binary, Duration::from_secs_f64(timeout), cancel)?;
        text += &format!("{sample}\n");
        samples.push(sample);
    }
    let medians: serde_json::Map<String, Value> =
        ["first_frame_ms", "populated_list_ms", "cancel_exit_ms"]
            .into_iter()
            .filter_map(|k| {
                median(samples.iter().filter_map(|s| s[k].as_f64()).collect())
                    .map(|m| (k.to_owned(), json!(m)))
            })
            .collect();
    text += &format!("{}\n", json!({"median":medians,"runs":runs}));
    print!("{text}");
    std::io::stdout().flush()?;
    ensure!(
        samples.iter().all(|s| s["first_frame_ms"].is_number()
            && s["terminal_restored"] == true
            && (cancel || s["populated_list_ms"].is_number())),
        "resume picker measurements failed"
    );
    Ok(text)
}
pub fn run_picker(args: impl Iterator<Item = String>) -> Result<()> {
    let args = PickerArgs::parse_from(std::iter::once("bench-resume-picker".into()).chain(args));
    picker(
        &args.binary.canonicalize()?,
        args.runs,
        args.timeout,
        args.cancel_loading,
    )?;
    Ok(())
}
fn rows(text: &str) -> Vec<Value> {
    let mut rows = Vec::new();
    for line in text.lines() {
        let candidate = if let Some((_, s)) = line.split_once("resume_bench ") {
            s.trim_start()
        } else if let Some(start) = line.find("{\"resume_bench\":") {
            &line[start..]
        } else if line.contains("\"resume_bench\":1") {
            let Some(start) = line.find('{') else {
                continue;
            };
            &line[start..]
        } else {
            continue;
        };
        let mut stream = serde_json::Deserializer::from_str(candidate).into_iter::<Value>();
        if let Some(Ok(value)) = stream.next()
            && value.is_object()
            && value.get("scenario").is_some()
        {
            rows.push(value);
        }
    }
    rows
}
fn validate(rows: &[Value], requested: usize) -> Result<()> {
    for row in rows {
        if let Some(metrics) = row.get("metrics") {
            let counts: Vec<_> = metrics
                .as_object()
                .context("invalid benchmark metrics")?
                .values()
                .filter_map(|v| v.get("samples"))
                .collect();
            ensure!(
                !counts.is_empty() && counts.iter().all(|n| n.as_u64() == Some(requested as u64)),
                "{}: expected {requested} samples, got {counts:?}",
                row["scenario"]
            );
        } else if let Some(attempts) = row.get("attempts") {
            ensure!(
                attempts.as_u64() == Some(requested as u64),
                "{}: expected {requested} attempts, got {attempts}",
                row["scenario"]
            );
        }
    }
    Ok(())
}
#[derive(Parser)]
struct SuiteArgs {
    #[arg(long, default_value_t = 20)]
    samples: usize,
    #[arg(long, default_value = "selfdev")]
    profile: String,
    #[arg(long)]
    output: Option<PathBuf>,
    #[arg(long)]
    skip_check: bool,
    #[arg(long)]
    binary: Option<PathBuf>,
    #[arg(long, default_value_t = 5)]
    pty_runs: usize,
}
fn execute(repo: &Path, args: &[String], samples: usize, log: &Path) -> Result<String> {
    println!("+ justrust {}", args.join(" "));
    std::io::stdout().flush()?;
    let mut command = Command::new("justrust");
    command
        .args(args)
        .current_dir(repo)
        .env("NEXODE_RESUME_BENCH_SAMPLES", samples.to_string());
    let out = crate::process::capture(&mut command, Duration::from_secs(7200))?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    fs::write(log, &text)?;
    print!("{text}");
    ensure!(
        out.status.success(),
        "command exited {}, full output retained in {}",
        out.status,
        log.display()
    );
    Ok(text)
}
pub fn run_suite(repo: &Path, args: impl Iterator<Item = String>) -> Result<()> {
    let args = SuiteArgs::parse_from(std::iter::once("bench-resume-suite".into()).chain(args));
    ensure!(
        (5..=200).contains(&args.samples) && args.pty_runs > 0,
        "samples must be 5..200 and pty-runs positive"
    );
    let git = |args: &[&str]| -> Result<String> {
        let o = Command::new("git").args(args).current_dir(repo).output()?;
        ensure!(o.status.success(), "git command failed");
        Ok(String::from_utf8(o.stdout)?.trim().to_owned())
    };
    let revision = git(&["rev-parse", "HEAD"])?;
    let dirty = !git(&["status", "--porcelain"])?.is_empty();
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    let out = args.output.unwrap_or_else(|| {
        repo.join("target/resume-latency")
            .join(format!("{stamp}-{}", &revision[..9]))
    });
    fs::create_dir_all(&out)?;
    let out = out.canonicalize()?;
    let platform = match std::env::consts::OS {
        "macos" => "darwin",
        "windows" => "win32",
        s => s,
    };
    let environment: serde_json::Map<String, Value> =
        ["JUSTRUST_REMOTE", "NEXODE_PERF_TIER", "NEXODE_CONFIG"]
            .into_iter()
            .map(|n| (n.into(), json!(std::env::var(n).ok())))
            .collect();
    let metadata = json!({"schema_version":1,"timestamp_utc":stamp,"revision":revision,"working_tree_dirty":dirty,"profile":args.profile,"samples_per_scenario":args.samples,"platform":platform,"measurement_scope":"in-process/offline fixtures; no cold OS cache or emulator paint","environment":environment});
    fs::write(
        out.join("metadata.json"),
        format!("{}\n", serde_json::to_string_pretty(&metadata)?),
    )?;
    let result_path = out.join("results.json");
    fs::write(
        &result_path,
        serde_json::to_string_pretty(&json!({"metadata":metadata,"rows":[],"status":"running"}))?,
    )?;
    let mut measurements = Vec::new();
    let result = (|| -> Result<()> {
        if !args.skip_check {
            execute(
                repo,
                &[
                    "check",
                    "--locked",
                    "-p",
                    "nexode-tui",
                    "--tests",
                    "--profile",
                    &args.profile,
                ]
                .map(str::to_owned),
                args.samples,
                &out.join("check.log"),
            )?;
        }
        for module in ["benchmark_matrix::", "benchmark_io::"] {
            let name = module.trim_end_matches(':');
            let options = vec![
                "test".into(),
                "--locked".into(),
                "-p".into(),
                "nexode-tui".into(),
                "--lib".into(),
                "--profile".into(),
                args.profile.clone(),
                module.into(),
                "--config".into(),
                format!("env.NEXODE_RESUME_BENCH_SAMPLES=\"{}\"", args.samples),
                "--".into(),
                "--ignored".into(),
                "--nocapture".into(),
                "--test-threads=1".into(),
            ];
            let text = execute(
                repo,
                &options,
                args.samples,
                &out.join(format!("{name}.log")),
            )?;
            let found = rows(&text);
            ensure!(!found.is_empty(), "{name} produced no benchmark rows");
            measurements.extend(found.clone());
            validate(&found, args.samples)?;
        }
        if let Some(binary) = args.binary {
            let binary = binary.canonicalize()?;
            for cancel in [false, true] {
                let text = picker(&binary, args.pty_runs, 30.0, cancel)?;
                fs::write(
                    out.join(if cancel {
                        "pty_cancel.log"
                    } else {
                        "pty_startup.log"
                    }),
                    text,
                )?;
            }
        }
        Ok(())
    })();
    let payload = match &result {
        Ok(()) => json!({"metadata":metadata,"rows":measurements,"status":"completed"}),
        Err(e) => {
            json!({"metadata":metadata,"rows":measurements,"status":"failed","error":format!("{e:#}")})
        }
    };
    fs::write(
        &result_path,
        format!("{}\n", serde_json::to_string_pretty(&payload)?),
    )?;
    result?;
    println!("Resume benchmark results: {}", result_path.display());
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_both_schemas_prefixes_and_null_percentiles() {
        let r = rows(
            "test matrix ... resume_bench {\"scenario\":\"cold\",\"metrics\":{}}\n{\"cache\":\"warm\",\"resume_bench\":1,\"scenario\":\"disk\"}\nresume_bench {\"scenario\":\"failed\",\"p50_ms\":null,\"failures\":2}\nresume_bench broken\nresume_bench {\"warning\":1}",
        );
        assert_eq!(r.len(), 3);
        assert_eq!(r[0]["scenario"], "cold");
        assert_eq!(r[1]["scenario"], "disk");
        assert!(r[2]["p50_ms"].is_null());
        assert_eq!(r[2]["failures"], 2);
    }
    #[test]
    fn rejects_lost_sample_configuration() {
        assert!(
            validate(
                &[json!({"scenario":"cold","metrics":{"frame":{"samples":20}}})],
                5
            )
            .is_err()
        );
        validate(
            &[json!({"scenario":"cold","metrics":{"frame":{"samples":5}}})],
            5,
        )
        .unwrap();
        validate(&[json!({"scenario":"disk","attempts":5,"failures":5})], 5).unwrap();
        assert!(validate(&[json!({"scenario":"empty","metrics":{}})], 5).is_err());
    }
}
