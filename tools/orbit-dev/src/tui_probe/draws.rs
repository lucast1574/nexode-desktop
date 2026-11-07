use super::isolated::{Session, pause};
use anyhow::{Context, Result, ensure};
use clap::Parser;
use serde_json::{Value, json};
use std::{fs, path::PathBuf, time::Instant};
fn cpu_stat(text: &str, ticks: f64) -> Result<f64> {
    ensure!(ticks > 0.0, "invalid system clock tick rate");
    let fields: Vec<_> = text
        .rsplit_once(") ")
        .context("invalid process stat")?
        .1
        .split_whitespace()
        .collect();
    let user = fields
        .get(11)
        .context("missing user ticks")?
        .parse::<u64>()?;
    let system = fields
        .get(12)
        .context("missing system ticks")?
        .parse::<u64>()?;
    Ok((user as f64 + system as f64) / ticks)
}
fn cpu(pid: u32) -> Option<f64> {
    let ticks = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    cpu_stat(
        &fs::read_to_string(format!("/proc/{pid}/stat")).ok()?,
        ticks as f64,
    )
    .ok()
}
fn samples(value: &Value) -> &[Value] {
    value["samples"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
}
fn newest(value: &Value) -> Option<f64> {
    samples(value)
        .last()
        .and_then(|v| v["timestamp_ms"].as_f64())
}
fn fresh(value: &Value, last: Option<f64>) -> Result<Vec<&Value>> {
    samples(value)
        .iter()
        .filter_map(|v| {
            let time = v["timestamp_ms"].as_f64();
            match time {
                Some(t) if last.is_none_or(|l| t > l) => Some(Ok(v)),
                Some(_) => None,
                None => Some(Err(anyhow::anyhow!("draw sample missing timestamp"))),
            }
        })
        .collect()
}
fn count(v: &Value, key: &str) -> u64 {
    v[key].as_u64().unwrap_or(0)
}
fn report(before: &Value, after: &Value, elapsed: f64, cpu_delta: Option<f64>) -> Result<Value> {
    ensure!(
        elapsed.is_finite() && elapsed > 0.0,
        "invalid sample interval"
    );
    let recent = fresh(after, newest(before))?;
    let schedule = &before["redraw_schedule"];
    let a = &after["idle_animation"];
    let b = &before["idle_animation"];
    let mut out = json!({"donut_active":schedule["idle_animation_active"],"animation_area":schedule["idle_animation_area"],"interval_ms":schedule["interval_ms"],"blocked_before":b["fast_path_blocked"],"blocked_after":a["fast_path_blocked"],"real_draws":recent.len(),"real_draws_per_s":recent.len() as f64/elapsed,"elapsed_s":elapsed,"sample_window_saturated":recent.len()>=240,"partial_delta":count(a,"partial_repaints").saturating_sub(count(b,"partial_repaints")),"full_delta":count(a,"full_repaints").saturating_sub(count(b,"full_repaints"))});
    if let Some(delta) = cpu_delta {
        ensure!(delta >= 0.0, "process CPU counter regressed");
        out["cpu_cores"] = json!(delta / elapsed);
    }
    if !recent.is_empty() {
        let mut renders = recent
            .iter()
            .map(|v| {
                v["render_ms"]
                    .as_f64()
                    .filter(|n| n.is_finite() && *n >= 0.0)
                    .context("invalid render duration")
            })
            .collect::<Result<Vec<_>>>()?;
        renders.sort_by(f64::total_cmp);
        let n = renders.len();
        let sum = renders.iter().sum::<f64>();
        out["render_ms"] = json!({"p50":renders[n/2],"p95":renders[(n*95/100).min(n-1)],"max":renders[n-1],"sum":sum});
        out["render_duty_cycle"] = json!(sum / (elapsed * 1000.0));
        let mut cells: Vec<_> = recent
            .iter()
            .filter_map(|v| v["changed_cells"].as_u64())
            .collect();
        if !cells.is_empty() {
            cells.sort_unstable();
            out["changed_cells"] = json!({"p50":cells[cells.len()/2],"max":cells[cells.len()-1],"total_cells":recent.last().unwrap()["total_cells"]});
        }
    }
    Ok(out)
}
#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "./target/selfdev/nexode")]
    binary: PathBuf,
    #[arg(long, default_value_t = 3.0)]
    window_s: f64,
    #[arg(long)]
    type_slash: bool,
    #[arg(long)]
    no_idle_animation: bool,
    #[arg(long)]
    json: bool,
    #[arg(long, default_value_t = 14)]
    samples: usize,
    #[arg(long, default_value_t = 1.0)]
    interval_s: f64,
}
pub(super) fn run(kind: &str, args: impl Iterator<Item = String>) -> Result<i32> {
    let a = Args::parse_from(std::iter::once(kind.to_owned()).chain(args));
    ensure!(
        a.window_s.is_finite()
            && a.window_s > 0.0
            && a.window_s <= 3600.0
            && a.interval_s.is_finite()
            && a.interval_s > 0.0
            && a.interval_s <= 3600.0
            && a.samples > 0
            && a.samples <= 10000,
        "invalid sample count or duration"
    );
    let binary = a.binary.canonicalize()?;
    let env = if a.no_idle_animation {
        vec![("NEXODE_IDLE_ANIMATION", "false")]
    } else {
        vec![]
    };
    let session = Session::with_environment(&binary, 48, 160, false, &env)?;
    let stats =
        || -> Result<Value> { Ok(serde_json::from_str(&session.command("draw-stats 240")?)?) };
    if kind == "pin-no-area-state" {
        let start = Instant::now();
        let mut prev_time = None;
        let mut prev_at = None;
        println!("     t  donut                   area   ms  msgs  proc notif draws/s");
        for _ in 0..a.samples {
            let ds = stats()?;
            let state: Value = serde_json::from_str(&session.command("state")?)?;
            let now = Instant::now();
            let rate = if let Some(previous) = prev_at {
                let delta = now.duration_since(previous).as_secs_f64();
                Some(fresh(&ds, prev_time)?.len() as f64 / delta)
            } else {
                None
            };
            let sched = &ds["redraw_schedule"];
            println!(
                "{:6.1} {:>6} {:>22} {:>4} {:>5} {:>5} {:>5} {}",
                start.elapsed().as_secs_f64(),
                sched["idle_animation_active"],
                sched["idle_animation_area"],
                sched["interval_ms"],
                state["display_messages"],
                state["processing"],
                sched["has_notification"],
                rate.map(|v| format!("{v:.1}")).unwrap_or_default()
            );
            if newest(&ds).is_some() {
                prev_time = newest(&ds);
            }
            prev_at = Some(now);
            pause(a.interval_s)?;
        }
        return Ok(0);
    }
    let mut composer = None;
    if a.type_slash {
        session.client.send(b"/")?;
        pause(1.5)?;
        composer = Some(session.command("input")?.trim().to_owned());
    }
    let before = stats()?;
    let cpu0 = cpu(session.client.pid);
    let start = Instant::now();
    pause(a.window_s)?;
    let after = stats()?;
    let elapsed = start.elapsed().as_secs_f64();
    let cpu1 = cpu(session.client.pid);
    let mut out = report(&before, &after, elapsed, cpu0.zip(cpu1).map(|(a, b)| b - a))?;
    out["binary"] = json!(binary);
    if let Some(composer) = composer {
        out["composer"] = json!(composer);
    }
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(0)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn draw_samples_not_animation_counters_determine_real_draw_rate() {
        let before = json!({"samples":[{"timestamp_ms":100}],"idle_animation":{"partial_repaints":0,"full_repaints":0}});
        let after = json!({"samples":[{"timestamp_ms":100,"render_ms":999},{"timestamp_ms":101,"render_ms":3,"changed_cells":0,"total_cells":100},{"timestamp_ms":103,"render_ms":7,"changed_cells":2,"total_cells":100}],"idle_animation":{"partial_repaints":0,"full_repaints":0}});
        let out = report(&before, &after, 2.0, Some(0.5)).unwrap();
        assert_eq!(out["real_draws"], 2);
        assert_eq!(out["real_draws_per_s"], 1.0);
        assert_eq!(out["partial_delta"], 0);
        assert_eq!(out["render_ms"]["sum"], 10.0);
        assert_eq!(out["cpu_cores"], 0.25);
        assert!(report(&before, &json!({"samples":[{}]}), 1.0, None).is_err());
    }
    #[test]
    fn process_name_parentheses_do_not_shift_cpu_fields() {
        let mut f = vec!["0"; 20];
        f[0] = "S";
        f[11] = "123";
        f[12] = "77";
        assert_eq!(
            cpu_stat(&format!("12 (name with ) parens) {}", f.join(" ")), 100.0).unwrap(),
            2.0
        );
        assert!(cpu_stat("bad", 100.0).is_err());
    }
}
