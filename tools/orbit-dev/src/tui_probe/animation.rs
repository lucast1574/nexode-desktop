//! Animation A/B measurements on an explicitly selected existing daemon.
use super::{draws::cpu, io, isolated::pause, profile::recent, screen::Client};
use anyhow::{Context, Result, ensure};
use clap::Parser;
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};
#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "./target/selfdev/nexode")]
    binary: PathBuf,
    #[arg(long)]
    session: Option<String>,
    #[arg(long, default_value_t = 5.0)]
    window_s: f64,
    #[arg(long, default_value_t = 2)]
    repeat: usize,
    #[arg(long, default_value_t = 48)]
    rows: u16,
    #[arg(long, default_value_t = 160)]
    cols: u16,
    #[arg(long,num_args=0..,default_values_t=[60u32,30,20,12])]
    fps: Vec<u32>,
    #[arg(long)]
    socket: Option<PathBuf>,
    #[arg(long)]
    debug_socket: Option<PathBuf>,
    #[arg(long)]
    working_dir: Option<PathBuf>,
}
fn median(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut v = values.to_vec();
    v.sort_by(f64::total_cmp);
    let n = v.len();
    Some(if n.is_multiple_of(2) {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    } else {
        v[n / 2]
    })
}
fn latency_summary(values: &[f64], expected: usize) -> Value {
    let mut v = values.to_vec();
    v.sort_by(f64::total_cmp);
    if v.is_empty() {
        return json!({"typing_samples":0,"typing_expected":expected});
    }
    json!({"typing_samples":v.len(),"typing_expected":expected,"typing_p50_ms":median(&v),"typing_p95_ms":v[(v.len()*95/100).min(v.len()-1)],"typing_max_ms":v[v.len()-1]})
}
struct Selection<'a> {
    binary: &'a Path,
    socket: &'a Path,
    work: &'a Path,
    session: &'a str,
    rows: u16,
    cols: u16,
}
fn once(
    selected: &Selection<'_>,
    enabled: bool,
    fps: Option<u32>,
    window: f64,
    settle: f64,
    phrase: &str,
) -> Result<Value> {
    let scratch = std::env::var_os("NEXODE_SCRATCH_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let root = tempfile::Builder::new()
        .prefix("orbit-animation-")
        .tempdir_in(scratch)?;
    let cmd = root.path().join("cmd");
    let resp = root.path().join("resp");
    let mut launch = Command::new(selected.binary);
    launch
        .current_dir(selected.work)
        .args(["--no-update", "--no-selfdev", "--socket"])
        .arg(selected.socket)
        .args(["--resume", selected.session])
        .env("NEXODE_DEBUG_CONTROL", "1")
        .env("NEXODE_DEBUG_CMD_PATH", &cmd)
        .env("NEXODE_DEBUG_RESPONSE_PATH", &resp)
        .env("NEXODE_THEME", "dark")
        .env("TERM", "xterm-256color")
        .env(
            "NEXODE_IDLE_ANIMATION",
            if enabled { "true" } else { "false" },
        );
    if let Some(fps) = fps {
        launch.env("NEXODE_ANIMATION_FPS", fps.to_string());
    }
    let client = Client::spawn(&mut launch, selected.rows, selected.cols)?;
    let command = |text: &str| io::client_command(&cmd, &resp, text, Duration::from_secs(15));
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        if io::client_command(&cmd, &resp, "input", Duration::from_secs(2)).is_ok() {
            break;
        }
        ensure!(
            Instant::now() < deadline && !client.snapshot()?.eof,
            "animation client never became ready"
        );
        pause(0.1)?;
    }
    pause(settle)?;
    let before: Value = serde_json::from_str(&command("draw-stats 1")?)?;
    let cpu0 = cpu(client.pid);
    let start = Instant::now();
    pause(window)?;
    let after: Value = serde_json::from_str(&command("draw-stats 1")?)?;
    let cpu1 = cpu(client.pid);
    let elapsed = start.elapsed().as_secs_f64();
    let a = after["idle_animation"]["partial_repaints"]
        .as_u64()
        .unwrap_or(0);
    let b = before["idle_animation"]["partial_repaints"]
        .as_u64()
        .unwrap_or(0);
    // First-output latency is retained for comparability. It is not key-to-paint:
    // an animation frame may be the first output after a key.
    let mut times = vec![];
    for ch in phrase.bytes() {
        let before = client.snapshot()?;
        let start = Instant::now();
        client.send(&[ch])?;
        while start.elapsed() < Duration::from_secs(2) {
            let now = client.snapshot()?;
            if now.frames > before.frames {
                times.push(start.elapsed().as_secs_f64() * 1000.0);
                break;
            }
            ensure!(!now.eof, "client exited while typing");
            pause(0.001)?;
        }
        pause(0.06)?;
    }
    command("set_input:")?;
    let sched = &before["redraw_schedule"];
    let active = sched["idle_animation_active"].as_bool();
    let valid =
        active == Some(enabled) && cpu0.is_some() && cpu1.is_some() && times.len() == phrase.len();
    let load = fs::read_to_string("/proc/loadavg")
        .ok()
        .and_then(|s| s.split_whitespace().next()?.parse::<f64>().ok());
    let mut out = latency_summary(&times, phrase.len());
    let data = json!({"animation":enabled,"fps":fps,"interval_ms":sched["interval_ms"],"donut_active":active,"partial_repaints_total":a,"animation_frames_per_s":a.saturating_sub(b) as f64/elapsed,"cpu_cores":cpu0.zip(cpu1).map(|(a,b)|(b-a)/elapsed),"load_average":load,"latency_reliable_load":load.map(|l|l<=4.0),"latency_metric":"first terminal output after key, not confirmed key-to-paint","valid":valid});
    out.as_object_mut()
        .unwrap()
        .extend(data.as_object().unwrap().clone());
    Ok(out)
}
pub(super) fn run(kind: &str, args: impl Iterator<Item = String>) -> Result<i32> {
    let a = Args::parse_from(std::iter::once(kind.to_owned()).chain(args));
    ensure!(
        a.window_s.is_finite()
            && a.window_s > 0.0
            && a.window_s <= 3600.0
            && a.repeat > 0
            && a.repeat <= 1000
            && a.rows > 0
            && a.cols > 0
            && a.rows <= 512
            && a.cols <= 1024
            && a.fps.iter().all(|f| *f > 0 && *f <= 1000),
        "invalid measurement bounds"
    );
    let binary = a.binary.canonicalize()?;
    let work = a.working_dir.unwrap_or(crate::root()?).canonicalize()?;
    let runtime = std::env::var_os("NEXODE_RUNTIME_DIR")
        .or_else(|| std::env::var_os("XDG_RUNTIME_DIR"))
        .map(PathBuf::from)
        .unwrap_or_else(|| format!("/run/user/{}", unsafe { libc::getuid() }).into());
    let socket = a
        .socket
        .or_else(|| std::env::var_os("NEXODE_SOCKET").map(PathBuf::from))
        .unwrap_or_else(|| runtime.join("nexode.sock"));
    let debug = a
        .debug_socket
        .unwrap_or_else(|| runtime.join("nexode-debug.sock"));
    let session = if let Some(session) = a.session {
        session
    } else {
        let previous = io::debug(&debug, "sessions", Duration::from_secs(30))?;
        let previous = recent(&serde_json::from_str(&previous)?, &work.to_string_lossy());
        match previous {
            Some(s) => s,
            None if kind == "measure-animation-cpu-cost" => io::create_session(&debug, &work)?,
            None => anyhow::bail!("could not resolve a session for frame-rate sweep"),
        }
    };
    ensure!(session.starts_with("session_"), "invalid session ID");
    let selected = Selection {
        binary: &binary,
        socket: &socket,
        work: &work,
        session: &session,
        rows: a.rows,
        cols: a.cols,
    };
    let mut measurements = vec![];
    if kind == "sweep-animation-fps" {
        for fps in a.fps.iter().copied().map(Some).chain(std::iter::once(None)) {
            let row = once(
                &selected,
                fps.is_some(),
                fps,
                a.window_s,
                9.0,
                "the quick brown fox jumps",
            )?;
            println!("{row}");
            measurements.push(row);
        }
    } else {
        for _ in 0..a.repeat {
            for enabled in [true, false] {
                let row = once(
                    &selected,
                    enabled,
                    None,
                    a.window_s,
                    10.0,
                    "the quick brown fox",
                )?;
                println!("{row}");
                measurements.push(row);
            }
        }
    }
    if measurements.iter().any(|v| v["valid"] != true) {
        eprintln!(
            "Invalid comparison: animation preconditions, CPU samples or typing samples are missing."
        );
        return Ok(3);
    }
    let off: Vec<_> = measurements
        .iter()
        .filter(|v| v["animation"] == false)
        .filter_map(|v| v["cpu_cores"].as_f64())
        .collect();
    let baseline = median(&off).context("missing animation-off baseline")?;
    println!("baseline (animation off): {baseline:.3} cores");
    if kind == "sweep-animation-fps" {
        for row in measurements.iter().filter(|r| r["animation"] == true) {
            println!(
                "{}fps costs {:.3} cores over baseline",
                row["fps"],
                row["cpu_cores"].as_f64().unwrap() - baseline
            );
        }
        Ok(0)
    } else {
        let on: Vec<_> = measurements
            .iter()
            .filter(|v| v["animation"] == true)
            .filter_map(|v| v["cpu_cores"].as_f64())
            .collect();
        let delta = median(&on).context("missing animation-on samples")? - baseline;
        println!("attributable CPU difference: {delta:.3} cores");
        Ok(i32::from(delta > 0.05))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn latency_summary_keeps_missing_samples_visible_and_medians_correct() {
        assert_eq!(median(&[7., 1., 5., 3.]), Some(4.));
        assert_eq!(median(&[]), None);
        let v = latency_summary(&[3., 1., 2.], 4);
        assert_eq!(v["typing_p50_ms"], 2.);
        assert_eq!(v["typing_p95_ms"], 3.);
        assert_eq!(v["typing_expected"], 4);
        assert_eq!(v["typing_samples"], 3);
        assert!(latency_summary(&[], 4)["typing_p50_ms"].is_null());
    }
}

#[cfg(all(test, target_os = "linux"))]
mod protocol_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    #[test]
    fn real_pty_and_command_channel_collect_on_and_off_measurements() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("client");
        fs::write(&script,r#"#!/bin/bash
printf '\033[2J\033[H1> '
count=0
while true; do
 if test -f "$NEXODE_DEBUG_CMD_PATH"; then
  request=$(cat "$NEXODE_DEBUG_CMD_PATH")
  unlink "$NEXODE_DEBUG_CMD_PATH"
  if [[ $request == draw-stats* ]]; then
   count=$((count+1))
   printf '{"redraw_schedule":{"idle_animation_active":%s,"interval_ms":16},"idle_animation":{"partial_repaints":%s}}' "$NEXODE_IDLE_ANIMATION" "$count" > "$NEXODE_DEBUG_RESPONSE_PATH"
  else
   printf 'ok' > "$NEXODE_DEBUG_RESPONSE_PATH"
  fi
 fi
 if IFS= read -r -t 0.01 -n1 key; then printf '\033[1;1H1> %s' "$key"; fi
done
"#).unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        let selected = Selection {
            binary: &script,
            socket: &dir.path().join("unused"),
            work: dir.path(),
            session: "session_fixture",
            rows: 3,
            cols: 40,
        };
        for enabled in [true, false] {
            let v = once(&selected, enabled, Some(20), 0.02, 0.0, "x").unwrap();
            assert_eq!(v["valid"], true, "{v}");
            assert_eq!(v["donut_active"], enabled);
            assert_eq!(v["typing_samples"], 1);
            assert!(v["cpu_cores"].is_number());
        }
    }
}
