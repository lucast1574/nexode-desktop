use super::screen::Client;
use anyhow::{Context, Result, ensure};
use clap::Parser;
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};
const PROBE: &str = "jqx92";
struct Spec {
    name: &'static str,
    argv: Vec<String>,
    environment: Vec<(&'static str, &'static str)>,
    marker: Option<&'static str>,
}
fn specs() -> Vec<Spec> {
    [
        ("nexode", "nexode"),
        ("pi", "pi"),
        ("prime_agent", "prime-agent"),
        ("opencode", "opencode"),
        ("codex", "codex"),
        ("claude_code", "claude"),
        ("cursor_agent", "cursor-agent"),
        ("copilot_cli", "copilot"),
        ("antigravity_cli", "agy"),
    ]
    .into_iter()
    .map(|(name, program)| {
        let mut spec = Spec {
            name,
            argv: vec![program.into()],
            environment: vec![],
            marker: None,
        };
        if name == "nexode" {
            spec.argv
                .extend(["--no-update".into(), "--no-selfdev".into()]);
            spec.environment.push(("NEXODE_NO_TELEMETRY", "1"));
        }
        if name == "antigravity_cli" {
            spec.environment.push(("AGY_CLI_DISABLE_AUTO_UPDATE", "1"));
            spec.marker = Some("CLI ready for user input");
        }
        spec
    })
    .collect()
}
fn first(rows: &[String]) -> Option<String> {
    rows.iter().find_map(|line| {
        let normalized = line.split_whitespace().collect::<Vec<_>>().join(" ");
        (!normalized.contains(PROBE)
            && normalized.chars().filter(|c| c.is_alphanumeric()).count() >= 3
            && normalized.chars().count() >= 4)
            .then(|| normalized.chars().take(120).collect())
    })
}
fn summarize(values: impl Iterator<Item = Option<f64>>) -> Value {
    let values: Vec<_> = values.collect();
    let total = values.len();
    let mut ready: Vec<_> = values.into_iter().flatten().collect();
    if ready.is_empty() {
        return Value::Null;
    }
    ready.sort_by(f64::total_cmp);
    let n = ready.len();
    let median = if n % 2 == 1 {
        ready[n / 2]
    } else {
        (ready[n / 2 - 1] + ready[n / 2]) / 2.0
    };
    json!({"median_ms":median,"min_ms":ready[0],"max_ms":ready[n-1],"mean_ms":ready.iter().sum::<f64>()/n as f64,"runs_completed":n,"runs_total":total})
}
fn once(spec: &Spec, cwd: &Path, timeout: Duration) -> Result<Value> {
    let log = tempfile::NamedTempFile::new()?;
    let mut command = Command::new(&spec.argv[0]);
    command
        .args(&spec.argv[1..])
        .current_dir(cwd)
        .env("TERM", "xterm-256color")
        .env("COLORTERM", "truecolor")
        .envs(spec.environment.iter().copied());
    if spec.marker.is_some() {
        command.arg("--log-file").arg(log.path());
    }
    let start = Instant::now();
    let client = Client::spawn(&mut command, 24, 80)?;
    let mut visible = None;
    let mut excerpt = None;
    let mut ready = None;
    let mut source = "probe_echo";
    while start.elapsed() < timeout {
        ensure!(
            crate::ci_suites::unix::interrupted().is_none(),
            "startup benchmark interrupted"
        );
        let state = client.snapshot()?;
        if visible.is_none() {
            if let Some(line) = first(&state.rows) {
                visible = Some(start.elapsed().as_secs_f64() * 1000.0);
                excerpt = Some(line);
                client.send(PROBE.as_bytes())?;
            }
        } else if state.rows.iter().any(|s| s.contains(PROBE)) {
            ready = Some(start.elapsed().as_secs_f64() * 1000.0);
            break;
        }
        if visible.is_some()
            && let Some(marker) = spec.marker
            && fs::read_to_string(log.path()).is_ok_and(|s| s.contains(marker))
        {
            ready = Some(start.elapsed().as_secs_f64() * 1000.0);
            source = "log_marker";
            break;
        }
        if state.eof {
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    Ok(
        json!({"first_visible_ms":visible,"first_visible_excerpt":excerpt,"input_ready_ms":ready,"input_ready_source":source}),
    )
}
fn version(spec: &Spec) -> Result<String> {
    let out = crate::process::capture_interruptible(
        Command::new(&spec.argv[0]).arg(if spec.name == "nexode" {
            "version"
        } else {
            "--version"
        }),
        Duration::from_secs(20),
        || crate::ci_suites::unix::interrupted().is_some(),
    )?;
    let combined =
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    Ok(combined
        .lines()
        .find(|s| !s.trim().is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| format!("exit {}", out.status)))
}
fn detect_pi() -> Result<String> {
    if std::env::var_os("PATH")
        .is_some_and(|p| std::env::split_paths(&p).any(|dir| dir.join("pi").is_file()))
    {
        return Ok("pi".into());
    }
    let out = crate::process::capture(
        Command::new("npm").args(["prefix", "-g"]),
        Duration::from_secs(20),
    )?;
    ensure!(out.status.success(), "npm prefix lookup failed");
    let candidate = PathBuf::from(String::from_utf8(out.stdout)?.trim()).join("bin/pi");
    Ok(if candidate.is_file() {
        candidate.to_string_lossy().into_owned()
    } else {
        "pi".into()
    })
}
#[derive(Parser)]
struct Args {
    #[arg(long, default_value_t = 10)]
    runs: usize,
    #[arg(long, default_value_t = 10.0)]
    timeout: f64,
    #[arg(long)]
    cwd: Option<PathBuf>,
    #[arg(long,num_args=0..)]
    tools: Vec<String>,
    #[arg(long, default_value = "/var/tmp/startup_visible_ready_results.json")]
    json_out: PathBuf,
}
pub(super) fn run(args: impl Iterator<Item = String>) -> Result<i32> {
    let a = Args::parse_from(std::iter::once("bench-startup-visible-ready".into()).chain(args));
    ensure!(
        a.runs > 0
            && a.runs <= 1000
            && a.timeout.is_finite()
            && a.timeout > 0.0
            && a.timeout <= 3600.0,
        "positive runs <=1000 and timeout <=3600 required"
    );
    let known = specs();
    for name in &a.tools {
        ensure!(
            known.iter().any(|s| s.name == name),
            "unknown benchmark tool {name}"
        );
    }
    let cwd = a.cwd.unwrap_or(std::env::current_dir()?).canonicalize()?;
    let mut results = json!({"runs":a.runs,"timeout_s":a.timeout,"cwd":cwd,"tools":{},"method":"Native PTY + vt100 emulation; timing includes process spawn. No prompts submitted."});
    let mut failed = false;
    for mut spec in known
        .into_iter()
        .filter(|s| a.tools.is_empty() || a.tools.iter().any(|n| n == s.name))
    {
        if spec.name == "pi" {
            spec.argv[0] = detect_pi()?;
        }
        println!("=== {} ===", spec.name);
        let mut runs = vec![];
        for i in 0..a.runs {
            let result = once(&spec, &cwd, Duration::from_secs_f64(a.timeout))?;
            println!("run {}/{}: {result}", i + 1, a.runs);
            failed |= result["first_visible_ms"].is_null() || result["input_ready_ms"].is_null();
            runs.push(result);
        }
        results["tools"][spec.name] = json!({"version":version(&spec)?,"first_visible_summary":summarize(runs.iter().map(|r|r["first_visible_ms"].as_f64())),"input_ready_summary":summarize(runs.iter().map(|r|r["input_ready_ms"].as_f64())),"runs":runs});
        let parent = a
            .json_out
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        fs::create_dir_all(parent)?;
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        serde_json::to_writer_pretty(&mut file, &results)?;
        file.persist(&a.json_out)
            .map_err(|e| e.error)
            .context("save startup report")?;
    }
    println!("WROTE {}", a.json_out.display());
    Ok(i32::from(failed))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn summary_retains_missing_runs_and_visible_filter_excludes_probe() {
        assert!(summarize([None, None].into_iter()).is_null());
        let r = summarize([Some(4.0), None, Some(2.0)].into_iter());
        assert_eq!(r["median_ms"], 3.0);
        assert_eq!(r["runs_total"], 3);
        assert_eq!(r["runs_completed"], 2);
        assert_eq!(
            first(&[PROBE.into(), "  ready  now ".into()]),
            Some("ready now".into())
        );
    }
    #[test]
    fn actual_pty_measures_rendered_probe_not_terminal_echo() {
        let dir = tempfile::tempdir().unwrap();
        let spec=Spec{name:"fixture",argv:vec!["/bin/bash".into(),"-c".into(),r#"printf '\033[2J\033[Hready'; value=''; while [ ${#value} -lt 5 ]; do IFS= read -r -n1 c; value=$value$c; done; printf '\033[2;1H%s' "$value"; sleep 1"#.into()],environment:vec![],marker:None};
        let r = once(&spec, dir.path(), Duration::from_secs(2)).unwrap();
        assert!(r["first_visible_ms"].as_f64().unwrap() <= r["input_ready_ms"].as_f64().unwrap());
        let silent = Spec {
            argv: vec![
                "/bin/bash".into(),
                "-c".into(),
                r"printf '\033[2J\033[Hready'; sleep 1".into(),
            ],
            ..spec
        };
        let r = once(&silent, dir.path(), Duration::from_millis(100)).unwrap();
        assert!(r["first_visible_ms"].is_number());
        assert!(r["input_ready_ms"].is_null());
    }
}
