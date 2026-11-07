use super::{
    isolated::{Session, pause},
    screen::Client,
};
use anyhow::{Result, ensure};
use clap::Parser;
use serde_json::Value;
use std::{collections::BTreeMap, path::PathBuf, time::Instant};
#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "./target/selfdev/nexode")]
    binary: PathBuf,
    #[arg(long, default_value_t = 48)]
    rows: u16,
    #[arg(long, default_value_t = 160)]
    cols: u16,
    #[arg(long, default_value_t = 2.0)]
    watch_s: f64,
    #[arg(long)]
    type_slash: bool,
    #[arg(long)]
    fresh_session: bool,
}
fn changed(previous: &[String], current: &[String], churn: &mut BTreeMap<usize, u64>) {
    for (i, (a, b)) in previous.iter().zip(current).enumerate() {
        if a != b {
            *churn.entry(i).or_default() += 1;
        }
    }
}
pub(super) fn moving_rows(client: &Client, seconds: f64) -> Result<BTreeMap<usize, u64>> {
    let mut prev = client.snapshot()?.rows;
    let mut churn = BTreeMap::new();
    let start = Instant::now();
    while start.elapsed().as_secs_f64() < seconds {
        let cur = client.snapshot()?;
        ensure!(!cur.eof, "client exited while observing screen");
        changed(&prev, &cur.rows, &mut churn);
        prev = cur.rows;
        pause(0.005)?;
    }
    Ok(churn)
}
fn selection(value: &Value, keys: &[&str]) -> Value {
    keys.iter()
        .map(|k| ((*k).to_owned(), value[*k].clone()))
        .collect::<serde_json::Map<_, _>>()
        .into()
}
pub(super) fn run(kind: &str, args: impl Iterator<Item = String>) -> Result<i32> {
    let a = Args::parse_from(std::iter::once(kind.to_owned()).chain(args));
    ensure!(
        a.watch_s.is_finite() && a.watch_s > 0.0 && a.watch_s <= 3600.0,
        "watch duration must be positive and at most 3600s"
    );
    let binary = a.binary.canonicalize()?;
    let session = Session::new(&binary, a.rows, a.cols, a.fresh_session)?;
    if a.type_slash {
        session.client.send(b"/")?;
        pause(1.0)?;
    }
    if kind == "why-no-animation-area" {
        session.command("enable")?;
        session.client.send(b"x")?;
        pause(1.0)?;
        session.command("set_input:")?;
        pause(1.0)?;
    }
    let stats: Value = serde_json::from_str(&session.command("draw-stats 1")?)?;
    let mut keys = vec![
        "idle_animation_active",
        "idle_animation_area",
        "interval_ms",
    ];
    if kind == "why-no-animation-area" {
        keys.extend([
            "animation_fps",
            "tier",
            "periodic_redraw_required",
            "current_full_frame_reason",
        ]);
    }
    println!(
        "== redraw schedule ==\n{}",
        serde_json::to_string_pretty(&selection(&stats["redraw_schedule"], &keys))?
    );
    match kind {
        "dump-fresh-spawn-screen" => {
            println!("\n== screen {}x{} ==", a.cols, a.rows);
            for (i, row) in session.client.snapshot()?.rows.iter().enumerate() {
                println!(
                    "{i:3} |{}",
                    row.trim_end().chars().take(150).collect::<String>()
                );
            }
            let churn = moving_rows(&session.client, a.watch_s)?;
            println!("\n== rows that changed over {}s ==", a.watch_s);
            if churn.is_empty() {
                println!("  NONE: the screen is completely static");
            } else {
                for (i, count) in churn {
                    println!("  row {i:3}: {count} changes");
                }
            }
        }
        "why-no-animation-area" => {
            println!(
                "\n== layout ==\n{}",
                session
                    .command("layout")?
                    .chars()
                    .take(3000)
                    .collect::<String>()
            );
            let screen = session.command("screen-json")?;
            println!("\n== screen (first 20 rows) ==");
            match serde_json::from_str::<Value>(&screen) {
                Ok(v) => {
                    for (i, row) in v["rows"]
                        .as_array()
                        .unwrap_or(&vec![])
                        .iter()
                        .take(20)
                        .enumerate()
                    {
                        let text = row.as_str().or_else(|| row["text"].as_str()).unwrap_or("");
                        println!("{i:3} |{}", text.chars().take(120).collect::<String>());
                    }
                }
                Err(_) => println!("{}", screen.chars().take(2000).collect::<String>()),
            }
        }
        "which-overlay-blocks-donut" => {
            let mut answered = 0;
            for probe in [
                "picker",
                "state",
                "overlays",
                "screen-text",
                "screen",
                "help",
            ] {
                match session.command(probe) {
                    Ok(answer) => {
                        answered += 1;
                        println!(
                            "\n== {probe} ==\n{}",
                            answer.chars().take(1500).collect::<String>()
                        );
                    }
                    Err(e) => eprintln!("{probe}: {e}"),
                }
            }
            ensure!(answered > 0, "no overlay diagnostic command answered");
        }
        _ => unreachable!(),
    }
    Ok(0)
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn changed_rows_distinguish_static_frames_and_unicode() {
        let before = vec!["header".into(), "é".into(), "static".into()];
        let after = vec!["header".into(), "中".into(), "static".into()];
        let mut changes = BTreeMap::new();
        changed(&before, &before, &mut changes);
        assert!(changes.is_empty());
        changed(&before, &after, &mut changes);
        changed(&after, &before, &mut changes);
        assert_eq!(changes, BTreeMap::from([(1, 2)]));
        assert_eq!(
            selection(&json!({"a":2,"extra":3}), &["a", "b"]),
            json!({"a":2,"b":null})
        );
    }
}
