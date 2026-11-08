//! Read-only correlation of reload logs, durable intents and transcript markers.
mod model;
mod parse;
use anyhow::{Context, Result, ensure};
use clap::Parser;
use serde_json::json;
use std::{path::PathBuf, time::UNIX_EPOCH};
#[derive(Parser)]
struct Args {
    #[arg(long)]
    home: Option<PathBuf>,
    #[arg(long)]
    reload_id: Option<String>,
    #[arg(long)]
    session: Option<String>,
    #[arg(long, default_value_t = 3)]
    max_log_files: usize,
    #[arg(long)]
    show_lines: bool,
    #[arg(long)]
    json: bool,
}
pub fn run(args: impl Iterator<Item = String>) -> Result<()> {
    let a = Args::try_parse_from(std::iter::once("reload-recovery-audit".into()).chain(args))?;
    ensure!(a.max_log_files <= 10000, "too many log files requested");
    let home = a
        .home
        .or_else(|| std::env::var_os("NEXODE_HOME").map(PathBuf::from))
        .or_else(|| dirs::home_dir().map(|p| p.join(".nexode")))
        .context("home directory not found")?;
    let mut logs = parse::files(&home.join("logs"), "log")
        .into_iter()
        .filter(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("nexode-"))
        })
        .collect::<Vec<_>>();
    logs.sort_by_key(|p| {
        std::cmp::Reverse(
            p.metadata()
                .and_then(|m| m.modified())
                .unwrap_or(UNIX_EPOCH),
        )
    });
    logs.truncate(a.max_log_files);
    logs.reverse();
    let (mut records, hidden) = parse::parse_logs(&logs)?;
    parse::traces(&mut records, &home.join("reload-traces"));
    parse::durable(&mut records, &home.join("reload-recovery"));
    let reload = a.reload_id.map(|s| {
        if s.starts_with("reload_") {
            s
        } else {
            format!("reload_{s}")
        }
    });
    let mut selected = records
        .into_values()
        .filter(|r| {
            reload.as_ref().is_none_or(|id| r.reload == *id)
                && a.session.as_ref().is_none_or(|id| r.session == *id)
        })
        .collect::<Vec<_>>();
    for r in &mut selected {
        parse::transcript(r, &home.join("sessions"));
    }
    if a.json {
        println!(
            "{}",
            serde_json::to_string_pretty(
                &json!({"home":home,"records":selected.iter().map(|r|r.json(a.show_lines)).collect::<Vec<_>>(),"global_hidden_sends":hidden.iter().map(model::Event::json).collect::<Vec<_>>()})
            )?
        );
    } else if selected.is_empty() {
        println!("No reload recovery records found for the selected filters.");
    } else {
        println!(
            "reload_id\tsession\trole\tstore\tpersist\told_claim\tattach\taccepted\tclient_queue\thist_write\tran_after\tmarkers\tverdict"
        );
        for r in &selected {
            println!(
                "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                r.reload,
                r.session,
                r.role.as_deref().unwrap_or("?"),
                r.status.as_deref().unwrap_or("?"),
                !r.events("persisted").is_empty(),
                !r.events("claimed").is_empty(),
                r.events("attached").len(),
                r.accepted(),
                r.client_queued(),
                r.events("send_history").len(),
                r.events("member_running").len(),
                r.markers
                    .as_object()
                    .map(|m| m.values().filter_map(|v| v.as_u64()).sum::<u64>())
                    .unwrap_or(0),
                r.verdict()
            );
        }
        println!(
            "\nGlobal hidden continuation send log lines scanned: {}",
            hidden.len()
        );
        println!(
            "Suspect delivered-without-acceptance records: {}",
            selected
                .iter()
                .filter(|r| r.verdict().starts_with("suspect"))
                .count()
        );
        if a.show_lines {
            for r in &selected {
                println!("\n[{} {}]", r.reload, r.session);
                for e in r.json(true)["events"].as_array().unwrap() {
                    println!(
                        "{}:{} {}: {}",
                        e["file"].as_str().unwrap(),
                        e["line"],
                        e["kind"].as_str().unwrap(),
                        e["text"].as_str().unwrap()
                    );
                }
            }
        }
    }
    Ok(())
}
