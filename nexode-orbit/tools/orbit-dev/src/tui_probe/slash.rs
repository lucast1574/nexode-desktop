use super::{
    animation_verify::watch_menu,
    inspect::moving_rows,
    isolated::{Session, pause},
};
use anyhow::{Context, Result, ensure};
use clap::Parser;
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
#[derive(Parser)]
struct Args {
    #[arg(long)]
    binary: Option<PathBuf>,
    #[arg(long)]
    json: bool,
    #[arg(short, long)]
    verbose: bool,
    #[arg(long, default_value_t = 3.0)]
    watch_s: f64,
    #[arg(long)]
    no_idle_animation: bool,
}
pub(super) fn run(args: impl Iterator<Item = String>) -> Result<i32> {
    let a = Args::parse_from(std::iter::once("repro-slash-flicker".into()).chain(args));
    ensure!(
        a.watch_s.is_finite() && a.watch_s > 0.0 && a.watch_s <= 3600.0,
        "invalid watch duration"
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
    let s = Session::returning(
        &binary,
        48,
        160,
        &[(
            "NEXODE_IDLE_ANIMATION",
            if a.no_idle_animation { "false" } else { "true" },
        )],
    )?;
    s.client.send(b"/")?;
    let deadline = Instant::now() + Duration::from_secs(8);
    let needles = loop {
        let rows = s.client.snapshot()?.rows;
        let n: Vec<_> = ["/help", "/model", "/clear", "/resume", "/config"]
            .into_iter()
            .filter(|n| rows.iter().any(|r| r.contains(n)))
            .collect();
        if !n.is_empty() {
            break n;
        }
        if Instant::now() >= deadline {
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &json!({"binary":binary,"error":"slash popup never appeared","screen":rows})
                )?
            );
            return Ok(3);
        }
        pause(0.05)?;
    };
    let menu = watch_menu(&s.client, &needles, a.watch_s)?;
    let churn = moving_rows(&s.client, 1.5)?;
    let mut busiest: Vec<_> = churn.iter().map(|(k, v)| (*k, *v)).collect();
    busiest.sort_by_key(|(_, v)| std::cmp::Reverse(*v));
    busiest.truncate(8);
    let stats = match s
        .command("draw-stats 16")
        .and_then(|v| serde_json::from_str::<Value>(&v).map_err(Into::into))
    {
        Ok(v) => v,
        Err(e) => json!({"error":e.to_string()}),
    };
    let flicker = menu["disappearances"].as_u64().unwrap_or(0) > 0
        || menu["menu_present_fraction"].as_f64().unwrap_or(0.0) < 1.0;
    let result = json!({"binary":binary,"idle_animation":!a.no_idle_animation,"menu_needles":needles,"menu_watch":menu,"static_rows_watch":{"changed_rows":churn.len(),"busiest_rows":busiest,"total_row_changes":churn.values().sum::<u64>()},"draw_stats":stats,"flicker_reproduced":flicker,"fixture":"isolated returning user with empty history"});
    if a.json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        println!(
            "menu present: {}\ndisappearances: {}\nstatic row churn: {}\nflicker reproduced: {flicker}",
            result["menu_watch"]["menu_present_fraction"],
            result["menu_watch"]["disappearances"],
            result["static_rows_watch"]
        );
        if a.verbose {
            println!(
                "transitions: {}\ndraw stats: {}",
                result["menu_watch"]["transitions"], result["draw_stats"]
            );
        }
    }
    Ok(i32::from(flicker))
}
