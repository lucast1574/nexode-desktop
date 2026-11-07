use super::{
    draws::cpu,
    isolated::{Session, pause},
};
use anyhow::{Context, Result, ensure};
use clap::Parser;
use serde_json::{Value, json};
use std::{path::PathBuf, time::Instant};
fn counter(v: &Value, key: &str) -> u64 {
    v[key].as_u64().unwrap_or(0)
}
fn delta(before: &Value, after: &Value, key: &str) -> u64 {
    counter(after, key).saturating_sub(counter(before, key))
}
fn preconditions(ds: &Value) -> Value {
    let s = &ds["redraw_schedule"];
    let a = &ds["idle_animation"];
    json!({"donut_active":s["idle_animation_active"],"interval_ms":s["interval_ms"],"tier":s["tier"],"animation_fps":s["animation_fps"],"decorative":s["decorative_animations"],"client_focused":s["client_focused"],"periodic_required":s["periodic_redraw_required"],"current_full_frame_reason":s["current_full_frame_reason"],"status_notice":s["status_notice"],"has_notification":s["has_notification"],"animation_area":s["idle_animation_area"],"partial":counter(a,"partial_repaints"),"full":counter(a,"full_repaints"),"blocked":a["fast_path_blocked"],"key_to_paint":s["key_to_paint"]})
}
fn measured(
    before: &Value,
    after: &Value,
    label: &str,
    elapsed: f64,
    cpu_delta: Option<f64>,
) -> Value {
    let a = &after["idle_animation"];
    let b = &before["idle_animation"];
    let s = &after["redraw_schedule"];
    let blocked: serde_json::Map<_, _> = a["fast_path_blocked"]
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(key, value)| {
            let n = value
                .as_u64()?
                .saturating_sub(counter(&b["fast_path_blocked"], key));
            (n > 0).then(|| (key.clone(), json!(n)))
        })
        .collect();
    json!({"state":label,"window_s":elapsed,"full_draws_per_s":delta(b,a,"full_repaints") as f64/elapsed,"sampled_draws_per_s":delta(before,after,"buffered_samples") as f64/elapsed,"partial_repaints_per_s":delta(b,a,"partial_repaints") as f64/elapsed,"cpu_cores":cpu_delta.map(|d|d/elapsed),"redraw_interval_ms":s["interval_ms"],"donut_active":s["idle_animation_active"],"periodic_redraw_required":s["periodic_redraw_required"],"full_frame_reason":s["current_full_frame_reason"],"fast_path_blocked":blocked,"status_notice":s["status_notice"],"has_notification":s["has_notification"],"client_focused":s["client_focused"]})
}
fn measure(session: &Session, label: &str, seconds: f64) -> Result<Value> {
    let before: Value = serde_json::from_str(&session.command("draw-stats 1")?)?;
    let cpu0 = cpu(session.client.pid);
    let start = Instant::now();
    pause(seconds)?;
    let after: Value = serde_json::from_str(&session.command("draw-stats 1")?)?;
    let cpu1 = cpu(session.client.pid);
    let elapsed = start.elapsed().as_secs_f64();
    let mut out = measured(
        &before,
        &after,
        label,
        elapsed,
        cpu0.zip(cpu1).map(|(a, b)| (b - a).max(0.0)),
    );
    out["input"] = json!(
        session
            .command("input")?
            .trim()
            .chars()
            .take(60)
            .collect::<String>()
    );
    out["full_frames_per_s"] = out["full_draws_per_s"].clone();
    out["partial_per_s"] = out["partial_repaints_per_s"].clone();
    out["interval_ms"] = out["redraw_interval_ms"].clone();
    out["blocked"] = out["fast_path_blocked"].clone();
    Ok(out)
}
#[derive(Parser)]
struct Args {
    #[arg(long)]
    binary: Option<PathBuf>,
    #[arg(long)]
    json: bool,
    #[arg(long)]
    window_s: Option<f64>,
    #[arg(long)]
    no_idle_animation: bool,
}
pub(super) fn run(kind: &str, args: impl Iterator<Item = String>) -> Result<i32> {
    let a = Args::parse_from(std::iter::once(kind.to_owned()).chain(args));
    let seconds = a
        .window_s
        .unwrap_or(if kind == "probe-idle-state" { 3.0 } else { 2.0 });
    ensure!(
        seconds.is_finite() && seconds > 0.0 && seconds <= 3600.0,
        "invalid sample interval"
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
    let session = Session::with_environment(
        &binary,
        48,
        160,
        false,
        &[(
            "NEXODE_IDLE_ANIMATION",
            if a.no_idle_animation { "false" } else { "true" },
        )],
    )?;
    let mut out = json!({"binary":binary,"idle_animation":!a.no_idle_animation,"states":[]});
    let strict = kind == "probe-idle-state";
    if strict {
        let stats: Value = serde_json::from_str(&session.command("draw-stats 1")?)?;
        out["preconditions"] = preconditions(&stats);
        if out["preconditions"]["donut_active"] != true {
            out["error"] =
                json!("donut inactive: this run cannot measure the requested fresh-spawn state");
            println!("{}", serde_json::to_string_pretty(&out)?);
            return Ok(3);
        }
    }
    let mut record = |label: &str| -> Result<()> {
        let state = measure(&session, label, seconds)?;
        if !a.json {
            println!("{state}");
        }
        out["states"].as_array_mut().unwrap().push(state);
        Ok(())
    };
    record("idle")?;
    session.client.send(b"/")?;
    pause(if strict { 1.5 } else { 1.2 })?;
    if strict {
        let input = session.command("input")?;
        ensure!(
            input.trim() == "/",
            "slash did not reach the composer: state precondition failed"
        );
    }
    record("palette-open")?;
    record(if strict {
        "palette-held"
    } else {
        "palette-settled"
    })?;
    session.command("set_input:hello")?;
    pause(1.5)?;
    record("plain-draft")?;
    if !strict {
        session.command("set_input:")?;
        pause(1.2)?;
        record("idle-again")?;
    }
    if strict {
        out["input_after_slash"] = json!("/");
    }
    if a.json {
        println!("{}", serde_json::to_string_pretty(&out)?);
    }
    Ok(0)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn monotonic_frame_counts_do_not_use_saturated_buffer_delta() {
        let before = json!({"buffered_samples":240,"idle_animation":{"full_repaints":500,"partial_repaints":100,"fast_path_blocked":{"palette":10,"old":4}}});
        let after = json!({"buffered_samples":240,"idle_animation":{"full_repaints":620,"partial_repaints":106,"fast_path_blocked":{"palette":13,"old":4}},"redraw_schedule":{"idle_animation_active":true,"interval_ms":16}});
        let v = measured(&before, &after, "palette", 2.0, Some(0.5));
        assert_eq!(v["full_draws_per_s"], 60.0);
        assert_eq!(v["sampled_draws_per_s"], 0.0);
        assert_eq!(v["partial_repaints_per_s"], 3.0);
        assert_eq!(v["fast_path_blocked"], json!({"palette":3}));
        assert_eq!(v["cpu_cores"], 0.25);
        assert_eq!(preconditions(&after)["donut_active"], true);
    }
}
