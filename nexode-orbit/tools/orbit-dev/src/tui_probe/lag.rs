use super::{
    draws::{cpu, report},
    input_flicker::observe,
    io,
    isolated::{Session, pause},
    profile::recent,
};
use anyhow::{Result, ensure};
use clap::Parser;
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
fn median(v: &mut [f64]) -> Option<f64> {
    v.sort_by(f64::total_cmp);
    let n = v.len();
    (n > 0).then(|| {
        if n.is_multiple_of(2) {
            (v[n / 2 - 1] + v[n / 2]) / 2.0
        } else {
            v[n / 2]
        }
    })
}
fn startup_result(samples: Vec<Value>) -> (Value, i32) {
    let mut during: Vec<_> = samples
        .iter()
        .filter(|s| s["startup_active"] == true)
        .filter_map(|s| s["paint_ms"].as_f64())
        .collect();
    let mut after: Vec<_> = samples
        .iter()
        .filter(|s| s["startup_active"] == false)
        .filter_map(|s| s["paint_ms"].as_f64())
        .collect();
    let d = median(&mut during);
    let a = median(&mut after);
    let code = match (d, a) {
        (Some(d), Some(a)) => i32::from(d > a * 1.5),
        _ => 3,
    };
    (
        json!({"samples":samples,"during_startup":{"count":during.len(),"p50_ms":d},"after_startup":{"count":after.len(),"p50_ms":a},"conclusion":match code{1=>"latency drop observed across startup boundary",0=>"no 1.5x latency drop observed",_=>"inconclusive: missing samples on one side of startup boundary"}}),
        code,
    )
}
fn measure(s: &Session, label: &str, seconds: f64) -> Result<Value> {
    let before: Value = serde_json::from_str(&s.command("draw-stats 240")?)?;
    let cpu0 = cpu(s.client.pid);
    let start = Instant::now();
    pause(seconds)?;
    let after: Value = serde_json::from_str(&s.command("draw-stats 240")?)?;
    let cpu1 = cpu(s.client.pid);
    let mut out = report(
        &before,
        &after,
        start.elapsed().as_secs_f64(),
        cpu0.zip(cpu1).map(|(a, b)| b - a),
    )?;
    out["state"] = json!(label);
    out["draws"] = out["real_draws"].clone();
    out["draws_per_s"] = out["real_draws_per_s"].clone();
    out["changed_cells_p50"] = out["changed_cells"]["p50"].clone();
    out["total_cells"] = out["changed_cells"]["total_cells"].clone();
    out["interval_ms"] = after["redraw_schedule"]["interval_ms"].clone();
    out["donut_active"] = after["redraw_schedule"]["idle_animation_active"].clone();
    out["animation_area"] = after["redraw_schedule"]["idle_animation_area"].clone();
    Ok(out)
}
#[derive(Parser)]
struct Args {
    #[arg(long)]
    binary: Option<PathBuf>,
    #[arg(long)]
    session: Option<String>,
    #[arg(long, default_value_t = 4.0)]
    window_s: f64,
    #[arg(long, default_value_t = 48)]
    rows: u16,
    #[arg(long, default_value_t = 160)]
    cols: u16,
    #[arg(long)]
    json: bool,
    #[arg(long, default_value_t = 25.0)]
    duration: f64,
    #[arg(long, default_value_t = 0.9)]
    interval: f64,
    #[arg(long)]
    socket: Option<PathBuf>,
    #[arg(long)]
    debug_socket: Option<PathBuf>,
    #[arg(long)]
    working_dir: Option<PathBuf>,
}
pub(super) fn run(kind: &str, args: impl Iterator<Item = String>) -> Result<i32> {
    let a = Args::parse_from(std::iter::once(kind.to_owned()).chain(args));
    for seconds in [a.window_s, a.duration, a.interval] {
        ensure!(
            seconds.is_finite() && seconds > 0.0 && seconds <= 3600.0,
            "durations must be positive and at most 3600 seconds"
        );
    }
    if kind == "repro-startup-lag" {
        ensure!(a.binary.is_some(), "--binary required for startup lag");
    }
    let binary = a
        .binary
        .unwrap_or_else(|| "./target/selfdev/nexode".into())
        .canonicalize()?;
    let work = a
        .working_dir
        .unwrap_or(if kind == "repro-startup-lag" {
            std::env::current_dir()?
        } else {
            crate::root()?
        })
        .canonicalize()?;
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
    if kind == "repro-startup-lag" {
        let s = Session::live(&binary, &work, &socket, &debug)?;
        let mut samples = vec![];
        let mut typed = String::new();
        let mut i = 0;
        while s.launched.elapsed().as_secs_f64() < a.duration {
            let ch = b'a' + i % 26;
            i = (i + 1) % 26;
            typed.push(char::from(ch));
            let state = match s
                .command("draw-stats 4")
                .and_then(|v| serde_json::from_str::<Value>(&v).map_err(Into::into))
            {
                Ok(v) => v["redraw_schedule"].clone(),
                Err(e) => json!({"error":e.to_string()}),
            };
            let observed = observe(&s, ch, &typed, a.interval, false)?;
            let row = json!({"since_spawn_ms":s.launched.elapsed().as_secs_f64()*1000.0,"paint_ms":observed["first_seen_ms"],"startup_active":state["remote_startup_phase_active"],"interval_ms":state["interval_ms"],"reason":state["current_full_frame_reason"],"snapshot_error":state["error"]});
            if !a.json {
                eprintln!("{row}");
            }
            samples.push(row);
            if typed.len() > 20 {
                s.command("set_input:")?;
                typed.clear();
            }
        }
        let (result, code) = startup_result(samples);
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(code);
    }
    let id = if let Some(id) = a.session {
        id
    } else {
        let previous = io::debug(&debug, "sessions", Duration::from_secs(30))
            .ok()
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .and_then(|v| recent(&v, &work.to_string_lossy()));
        match previous {
            Some(id) => id,
            None => io::create_session(&debug, &work)?,
        }
    };
    ensure!(id.starts_with("session_"), "invalid session ID");
    let s = Session::resume(&binary, &work, &socket, &id, a.rows, a.cols)?;
    let mut states = vec![measure(&s, "just-spawned", a.window_s)?];
    pause(2.0)?;
    states.push(measure(&s, "settled", a.window_s)?);
    let mut lat = vec![];
    for ch in b"hello world" {
        let before = s.client.snapshot()?.frames;
        let start = Instant::now();
        s.client.send(&[*ch])?;
        while start.elapsed() < Duration::from_secs(2) {
            let state = s.client.snapshot()?;
            ensure!(!state.eof, "client exited while typing");
            if state.frames > before {
                lat.push(start.elapsed().as_secs_f64() * 1000.0);
                break;
            }
            pause(0.001)?;
        }
        pause(0.12)?;
    }
    s.command("set_input:")?;
    lat.sort_by(f64::total_cmp);
    let typing = if lat.is_empty() {
        json!({"error":"no repaints observed"})
    } else {
        json!({"p50_ms":lat[lat.len()/2],"p95_ms":lat[(lat.len()*95/100).min(lat.len()-1)],"max_ms":lat.last(),"samples":lat.len(),"expected":11,"metric":"first terminal output, not confirmed key-to-paint"})
    };
    let worst = states
        .iter()
        .filter_map(|v| v["render_duty_cycle"].as_f64())
        .max_by(f64::total_cmp)
        .unwrap_or(0.0);
    let incomplete = lat.len() != 11 || states.iter().any(|s| s["sample_window_saturated"] == true);
    let payload = json!({"binary":binary,"session":id,"states":states,"typing":typing,"worst_render_duty_cycle":worst,"incomplete":incomplete});
    println!("{}", serde_json::to_string_pretty(&payload)?);
    Ok(if incomplete {
        3
    } else {
        i32::from(worst > 0.30)
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn startup_comparison_requires_both_observed_phases() {
        assert_eq!(
            startup_result(vec![json!({"paint_ms":10,"startup_active":null})]).1,
            3
        );
        assert_eq!(
            startup_result(vec![
                json!({"paint_ms":20,"startup_active":true}),
                json!({"paint_ms":10,"startup_active":false})
            ])
            .1,
            1
        );
        assert_eq!(
            startup_result(vec![
                json!({"paint_ms":12,"startup_active":true}),
                json!({"paint_ms":10,"startup_active":false})
            ])
            .1,
            0
        );
    }
}
