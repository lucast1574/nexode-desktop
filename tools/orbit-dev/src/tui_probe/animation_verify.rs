use super::{
    inspect::moving_rows,
    isolated::{Session, pause},
    screen::Client,
};
use anyhow::{Result, ensure};
use clap::Parser;
use serde_json::{Value, json};
use std::{collections::BTreeSet, path::PathBuf, time::Instant};
const MENU: &[&str] = &["/help", "/model", "/clear", "/resume", "/config", "/login"];
fn schedule(s: &Session) -> Result<Value> {
    let ds: Value = serde_json::from_str(&s.command("draw-stats 1")?)?;
    let r = &ds["redraw_schedule"];
    let a = &ds["idle_animation"];
    Ok(
        json!({"donut_active":r["idle_animation_active"],"area":r["idle_animation_area"],"interval_ms":r["interval_ms"],"partial":a["partial_repaints"].as_u64().unwrap_or(0),"full":a["full_repaints"].as_u64().unwrap_or(0)}),
    )
}
fn expected_animation(after: &Value, moving: usize, partial_rate: f64) -> Result<()> {
    ensure!(
        after["donut_active"] == true,
        "animation is not active on the expected screen"
    );
    ensure!(
        !after["area"].is_null(),
        "animation is active but has no published rectangle"
    );
    ensure!(moving > 0, "animation has no moving screen rows");
    ensure!(
        partial_rate >= 5.0,
        "animation is not served by partial repaints ({partial_rate:.1}/s)"
    );
    Ok(())
}
#[derive(Default)]
struct MenuWatch {
    samples: usize,
    present: usize,
    previous: Option<bool>,
    disappearances: usize,
    transitions: Vec<Value>,
    counts: BTreeSet<usize>,
}
impl MenuWatch {
    fn add(&mut self, seconds: f64, count: usize) {
        self.samples += 1;
        self.counts.insert(count);
        let present = count > 0;
        if present {
            self.present += 1;
        }
        if self.previous != Some(present) && (self.previous.is_some() || present) {
            if !present {
                self.disappearances += 1;
            }
            if self.transitions.len() < 40 {
                self.transitions.push(
                    json!({"at_ms":seconds*1000.0,"kind":if present{"appear"}else{"disappear"}}),
                );
            }
        }
        self.previous = Some(present);
    }
    fn report(&self, seconds: f64) -> Value {
        json!({"samples":self.samples,"seconds":seconds,"menu_present_fraction":self.present as f64/self.samples.max(1) as f64,"disappearances":self.disappearances,"transitions":self.transitions,"distinct_row_counts":self.counts})
    }
}
pub(super) fn watch_menu(client: &Client, needles: &[&str], seconds: f64) -> Result<Value> {
    let start = Instant::now();
    let mut watch = MenuWatch::default();
    while start.elapsed().as_secs_f64() < seconds {
        let s = client.snapshot()?;
        ensure!(!s.eof, "client exited while observing menu");
        let count = s
            .rows
            .iter()
            .filter(|row| needles.iter().any(|n| row.contains(n)))
            .count();
        watch.add(start.elapsed().as_secs_f64(), count);
        pause(0.004)?;
    }
    Ok(watch.report(start.elapsed().as_secs_f64()))
}
#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "./target/selfdev/nexode")]
    binary: PathBuf,
    #[arg(long, default_value_t = 2.0)]
    watch_s: f64,
    #[arg(long)]
    json: bool,
}
pub(super) fn run(kind: &str, args: impl Iterator<Item = String>) -> Result<i32> {
    let a = Args::parse_from(std::iter::once(kind.to_owned()).chain(args));
    ensure!(
        a.watch_s.is_finite() && a.watch_s > 0.0 && a.watch_s <= 3600.0,
        "invalid observation window"
    );
    let binary = a.binary.canonicalize()?;
    let session = Session::with_environment(
        &binary,
        48,
        160,
        false,
        &[("NEXODE_IDLE_ANIMATION", "true")],
    )?;
    let live = kind == "check-donut-animates-live";
    session.client.send(b"\x1b")?;
    pause(if live { 1.5 } else { 2.0 })?;
    // A fully isolated HOME can open a welcome overlay that ignores Escape.
    // Clear locally to establish the intended empty-chat animation state.
    session.command("message:/clear")?;
    pause(2.5)?;
    session.client.send(b"x")?;
    pause(0.4)?;
    session.command("set_input:")?;
    pause(1.5)?;
    let before = schedule(&session)?;
    let state: Value = serde_json::from_str(&session.command("state")?)?;
    let start = Instant::now();
    let churn = moving_rows(&session.client, a.watch_s)?;
    let after = schedule(&session)?;
    let elapsed = start.elapsed().as_secs_f64();
    let rate = |key: &str| {
        after[key]
            .as_u64()
            .unwrap_or(0)
            .saturating_sub(before[key].as_u64().unwrap_or(0)) as f64
            / elapsed
    };
    let partial = rate("partial");
    let mut busiest: Vec<_> = churn.iter().map(|(k, v)| (*k, *v)).collect();
    busiest.sort_by_key(|(_, v)| std::cmp::Reverse(*v));
    busiest.truncate(6);
    let mut out = json!({"binary":binary,"display_messages":state["display_messages"],"donut_active":after["donut_active"],"animation_area":after["area"],"interval_ms":after["interval_ms"],"moving_rows":churn.len(),"busiest_rows":busiest,"partial_per_s":partial,"full_per_s":rate("full"),"cpu_cores":null});
    let mut failures = vec![];
    if let Err(e) = expected_animation(&after, churn.len(), partial) {
        failures.push(e.to_string());
    }
    if !live {
        session.command("set_input:/")?;
        let deadline = Instant::now() + std::time::Duration::from_secs(8);
        let needles = loop {
            let rows = session.client.snapshot()?.rows;
            let needles: Vec<_> = MENU
                .iter()
                .copied()
                .filter(|n| rows.iter().any(|r| r.contains(n)))
                .collect();
            if !needles.is_empty() || Instant::now() >= deadline {
                break needles;
            }
            pause(0.1)?;
        };
        if needles.is_empty() {
            failures.push("slash palette never opened".into());
        } else {
            let watch = watch_menu(&session.client, &needles, a.watch_s)?;
            if watch["disappearances"].as_u64().unwrap_or(0) > 0
                || watch["menu_present_fraction"].as_f64().unwrap_or(0.0) < 1.0
            {
                failures.push("slash palette disappeared while open".into());
            }
            out["palette"] = watch;
        }
        session.command("set_input:")?;
        pause(0.4)?;
        session.client.send(b"\x1b")?;
        pause(0.3)?;
        session.command("message:/resume")?;
        pause(2.0)?;
        let overlay = schedule(&session)?;
        let overlay_churn = moving_rows(&session.client, a.watch_s)?;
        if !overlay["area"].is_null() && overlay["interval_ms"] == 16 {
            failures.push("overlay screen is paced at animation cadence".into());
        }
        out["overlay"] = json!({"schedule":overlay,"moving_rows":overlay_churn.len()});
    }
    out["failures"] = json!(failures);
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(i32::from(!failures.is_empty()))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inactive_or_stationary_animation_cannot_pass() {
        let good = json!({"donut_active":true,"area":{"height":18}});
        expected_animation(&good, 3, 60.0).unwrap();
        assert!(expected_animation(&good, 0, 60.0).is_err());
        assert!(expected_animation(&good, 3, 0.0).is_err());
        assert!(expected_animation(&json!({"donut_active":true,"area":null}), 3, 60.0).is_err());
        assert!(expected_animation(&json!({"donut_active":false,"area":{}}), 3, 60.0).is_err());
    }
    #[test]
    fn menu_watch_counts_disappearances_and_resize_without_unbounded_history() {
        let mut w = MenuWatch::default();
        for (i, n) in [0, 3, 4, 0, 2].into_iter().enumerate() {
            w.add(i as f64, n);
        }
        let r = w.report(5.0);
        assert_eq!(r["disappearances"], 1);
        assert_eq!(r["menu_present_fraction"], 0.6);
        assert_eq!(r["distinct_row_counts"], json!([0, 2, 3, 4]));
        for i in 0..200 {
            w.add(i as f64, i % 2);
        }
        assert_eq!(w.transitions.len(), 40);
        assert!(w.disappearances > 40);
    }
}
