use super::model::{Event, Record, Records, add};
use anyhow::Result;
use regex::Regex;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
pub(super) fn files(dir: &Path, extension: &str) -> Vec<PathBuf> {
    let mut files = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|e| e == extension))
        .collect::<Vec<_>>();
    files.sort();
    files
}
pub(super) fn parse_logs(paths: &[PathBuf]) -> Result<(Records, Vec<Event>)> {
    let patterns=[
        ("persisted",r"reload recovery store: persisted intent reload_id=(reload_[^ ]+) session=([^ ]+) role=([^ ]+)"),
        ("claimed",r"reload recovery store: claimed intent reload_id=(reload_[^ ]+) session=([^ ]+) role=([^ ]+)"),
        ("attached",r"reload recovery store: attached pending intent reload_id=(reload_[^ ]+) session=([^ ]+) role=([^ ]+)"),
        ("delivered",r"reload recovery store: delivered intent reload_id=(reload_[^ ]+) session=([^ ]+) role=([^ ]+) accepted_by=([^ ]+)"),
        ("non_pending",r"reload recovery store: skipping non-pending intent session=([^ ]+) reload_id=(reload_[^ ]+) status=([^ ]+)"),
        ("history_queued",r"History payload requested reload recovery continuation: session=([^ ]+) was_interrupted=([^ ]+)"),
        ("tui_flow",r"reload recovery flow=([^ ]+) session_id=([^ ]+) outcome=([^ ]+) detail=(.*)"),
        ("history_snapshot",r"history_reload_recovery_snapshot: (?:using|attaching) server-owned recovery intent for session=([^ ]+)"),
        ("send_history",r"\[TIMING\] send_history write: session=([^, ]+)"),
        ("member_running",r"new_status=running old_status=[^ ]+ phase=member_status_updated .*session_id=([^ ]+)"),
        ("hidden_send",r"Sending hidden continuation reminder \((\d+) chars\)"),
    ].into_iter().map(|(kind,re)|Ok((kind,Regex::new(re)?))).collect::<Result<Vec<_>>>()?;
    let mut records = Records::new();
    let mut latest: BTreeMap<String, String> = BTreeMap::new();
    let mut hidden = vec![];
    for path in paths {
        let Ok(bytes) = fs::read(path) else { continue };
        let text = String::from_utf8_lossy(&bytes);
        for (index, line) in text.lines().enumerate() {
            for (kind, pattern) in &patterns {
                let Some(m) = pattern.captures(line) else {
                    continue;
                };
                let mut e = Event {
                    kind: (*kind).into(),
                    line: index + 1,
                    text: line.into(),
                    file: path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into(),
                    ..Default::default()
                };
                match *kind {
                    "persisted" | "claimed" | "attached" | "delivered" => {
                        e.reload = Some(m[1].into());
                        e.session = Some(m[2].into());
                        e.role = Some(m[3].into());
                        if *kind == "delivered" {
                            e.detail = Some(format!("accepted_by={}", &m[4]));
                        }
                        latest.insert(m[2].into(), m[1].into());
                    }
                    "non_pending" => {
                        e.reload = Some(m[2].into());
                        e.session = Some(m[1].into());
                        e.detail = Some(m[3].into());
                        latest.insert(m[1].into(), m[2].into());
                    }
                    "hidden_send" => {
                        e.detail = Some(format!("chars={}", &m[1]));
                        hidden.push(e);
                        break;
                    }
                    other => {
                        let session = if other == "tui_flow" { &m[2] } else { &m[1] };
                        e.session = Some(session.into());
                        e.reload = latest.get(session).cloned();
                        e.detail = match other {
                            "history_queued" => Some(format!("was_interrupted={}", &m[2])),
                            "tui_flow" => {
                                Some(format!("flow={} outcome={} detail={}", &m[1], &m[3], &m[4]))
                            }
                            _ => None,
                        };
                    }
                }
                add(&mut records, e);
                break;
            }
        }
    }
    Ok((records, hidden))
}
fn string(v: &Value, key: &str) -> Option<String> {
    v[key].as_str().filter(|s| !s.is_empty()).map(str::to_owned)
}
pub(super) fn traces(records: &mut Records, dir: &Path) {
    for p in files(dir, "jsonl") {
        let Ok(bytes) = fs::read(&p) else { continue };
        for (index, line) in String::from_utf8_lossy(&bytes).lines().enumerate() {
            let Ok(v) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            let kind = match v["phase"].as_str() {
                Some("intent_attached_to_history") => "attached",
                Some("intent_delivered") => "delivered",
                Some("intent_delivery_mismatch") => "delivery_mismatch",
                _ => continue,
            };
            let detail = ["accepted_by", "status", "phase"]
                .into_iter()
                .filter_map(|key| string(&v, key).map(|s| format!("{key}={s}")))
                .collect::<Vec<_>>()
                .join(" ");
            add(
                records,
                Event {
                    kind: kind.into(),
                    line: index + 1,
                    text: line.into(),
                    file: p.file_name().unwrap_or_default().to_string_lossy().into(),
                    reload: string(&v, "reload_id")
                        .or_else(|| p.file_stem().map(|s| s.to_string_lossy().into())),
                    session: string(&v, "session_id"),
                    role: string(&v, "role"),
                    detail: Some(detail),
                },
            );
        }
    }
}
pub(super) fn durable(records: &mut Records, dir: &Path) {
    for p in files(dir, "json") {
        let Ok(v) = fs::read(&p)
            .and_then(|b| serde_json::from_slice::<Value>(&b).map_err(std::io::Error::other))
        else {
            continue;
        };
        let (Some(reload), Some(session)) = (string(&v, "reload_id"), string(&v, "session_id"))
        else {
            continue;
        };
        let r = records
            .entry((reload.clone(), session.clone()))
            .or_insert_with(|| Record {
                reload,
                session,
                ..Default::default()
            });
        if r.role.is_none() {
            r.role = string(&v, "role");
        }
        r.status = string(&v, "status");
        r.delivered_at = v["delivered_at"].clone();
    }
}
pub(super) fn transcript(r: &mut Record, dir: &Path) {
    // Log identifiers must never redirect this read outside the selected session directory.
    if r.session.is_empty()
        || r.session.contains(['/', '\\'])
        || matches!(r.session.as_str(), "." | "..")
    {
        return;
    }
    if let Ok(bytes) = fs::read(dir.join(format!("{}.json", r.session))) {
        let text = String::from_utf8_lossy(&bytes);
        r.markers = json!(
            [
                "Reload succeeded",
                "interrupted by a server reload",
                "Continue immediately from where you left off",
                "Reload complete"
            ]
            .into_iter()
            .map(|s| (s, text.matches(s).count()))
            .collect::<BTreeMap<_, _>>()
        );
    }
}
