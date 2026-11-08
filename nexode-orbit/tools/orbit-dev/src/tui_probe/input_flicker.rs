use super::{
    isolated::{Session, pause},
    screen::composer,
};
use anyhow::{Context, Result, ensure};
use clap::Parser;
use serde_json::{Value, json};
use std::{path::PathBuf, time::Instant};
fn analyze(typed: &str, timeline: &[(f64, Option<String>)]) -> Value {
    let mut first = None;
    let mut lost = None;
    for (at, value) in timeline {
        let visible = value.as_ref().is_some_and(|v| v.contains(typed));
        if visible && first.is_none() {
            first = Some(*at);
        } else if first.is_some() && !visible && lost.is_none() {
            lost = Some(*at);
        }
    }
    json!({"typed":typed,"timeline":timeline,"first_seen_ms":first,"flickered":lost.is_some(),"lost_after_ms":lost})
}
pub(super) fn observe(
    s: &Session,
    ch: u8,
    expected: &str,
    watch_s: f64,
    probe_input: bool,
) -> Result<Value> {
    let mark = s
        .client
        .snapshot()?
        .history
        .back()
        .map(|(n, _, _)| *n)
        .unwrap_or(0);
    let start = Instant::now();
    s.client.send(&[ch])?;
    let probe = Instant::now();
    let round_trip = if probe_input {
        s.command("input")
            .ok()
            .map(|_| probe.elapsed().as_secs_f64() * 1000.0)
    } else {
        None
    };
    let remaining = watch_s - start.elapsed().as_secs_f64();
    if remaining > 0.0 {
        pause(remaining)?;
    }
    let snapshot = s.client.snapshot()?;
    let timeline: Vec<_> = snapshot
        .history
        .iter()
        .filter(|(n, at, _)| *n > mark && *at >= start)
        .map(|(_, at, v)| (at.duration_since(start).as_secs_f64() * 1000.0, v.clone()))
        .collect();
    let mut out = analyze(expected, &timeline);
    out["app_input"] = if probe_input {
        s.command("input")
            .map(|v| json!(v.trim()))
            .unwrap_or_else(|e| json!(format!("<unavailable: {e}>")))
    } else {
        Value::Null
    };
    out["composer_on_screen"] = json!(composer(&snapshot.rows));
    out["loop_round_trip_ms"] = json!(round_trip);
    Ok(out)
}
#[derive(Parser)]
struct Args {
    #[arg(long)]
    binary: Option<PathBuf>,
    #[arg(long)]
    live: bool,
    #[arg(long, default_value_t = 3)]
    repeat: usize,
    #[arg(long)]
    json: bool,
    #[arg(short, long)]
    verbose: bool,
    #[arg(long)]
    keep: bool,
    #[arg(long)]
    dump_screen: bool,
    #[arg(long)]
    socket: Option<PathBuf>,
    #[arg(long)]
    debug_socket: Option<PathBuf>,
    #[arg(long)]
    working_dir: Option<PathBuf>,
}
pub(super) fn run(args: impl Iterator<Item = String>) -> Result<i32> {
    let a = Args::parse_from(std::iter::once("repro-input-flicker".into()).chain(args));
    ensure!(
        a.repeat > 0 && a.repeat <= 1000,
        "repeat must be between 1 and 1000"
    );
    let default = crate::root()?.join("target/selfdev/nexode");
    let binary = a
        .binary
        .or_else(|| default.exists().then_some(default))
        .or_else(|| {
            std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".nexode/builds/current/nexode"))
        })
        .context("provide --binary")?
        .canonicalize()?;
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
    let mut runs = vec![];
    let mut total = 0;
    let mut missing = 0;
    for index in 0..a.repeat {
        let mut s = if a.live {
            Session::live(&binary, &work, &socket, &debug)?
        } else {
            Session::returning(&binary, 48, 160, &[])?
        };
        let artifacts = if a.keep { s.retain_artifacts() } else { None };
        if let Some(path) = &artifacts {
            eprintln!("kept artifacts: {}", path.display());
        }
        let before = s.client.snapshot()?.rows;
        if a.dump_screen {
            s.client.send(b"/")?;
            pause(0.6)?;
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &json!({"before":before,"after":s.client.snapshot()?.rows})
                )?
            );
            return Ok(0);
        }
        ensure!(
            !composer(&before).is_some_and(|v| v.contains('/')),
            "detector sees slash before typing; negative control failed"
        );
        let mut typed = String::new();
        let mut observations = vec![];
        for ch in b"/model" {
            typed.push(char::from(*ch));
            let observation = observe(&s, *ch, &typed, 1.5, true)?;
            if !a.json || a.verbose {
                eprintln!("{observation}");
            }
            observations.push(observation);
        }
        let flicker = observations
            .iter()
            .filter(|v| v["flickered"] == true)
            .count();
        let never: Vec<_> = observations
            .iter()
            .filter(|v| v["first_seen_ms"].is_null())
            .map(|v| v["typed"].clone())
            .collect();
        total += flicker;
        missing += never.len();
        let mut run = json!({"run":index+1,"session_id":s.session_id,"keystrokes":observations,"flicker_count":flicker,"never_appeared":never,"artifacts":artifacts});
        match s
            .command("draw-stats 16")
            .and_then(|v| serde_json::from_str::<Value>(&v).map_err(Into::into))
        {
            Ok(v) => {
                run["draw_summary"] = v["summary"].clone();
                run["redraw_schedule"] = v["redraw_schedule"].clone();
            }
            Err(e) => run["draw_error"] = json!(e.to_string()),
        }
        runs.push(run);
    }
    let result = json!({"binary":binary,"runs":runs,"total_flicker":total,"never_appeared_count":missing,"flicker_reproduced":total>0,"mode":if a.live{"live-new-session"}else{"isolated-returning-user-new-server-per-run"}});
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(i32::from(total > 0 || missing > 0))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transient_erasure_and_never_seen_keys_cannot_look_clean() {
        let v = analyze(
            "/m",
            &[
                (1., Some("/".into())),
                (4., Some("/m".into())),
                (6., None),
                (8., Some("/m".into())),
            ],
        );
        assert_eq!(v["first_seen_ms"], 4.);
        assert_eq!(v["lost_after_ms"], 6.);
        assert_eq!(v["flickered"], true);
        assert!(analyze("/m", &[(1., Some("/".into()))])["first_seen_ms"].is_null());
        assert_eq!(
            analyze(
                "/m",
                &[(1., Some("/m".into())), (2., Some("/model".into()))]
            )["flickered"],
            false
        );
    }
}
