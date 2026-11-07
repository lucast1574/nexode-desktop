//! Real per-client transcript duplication detector with a forced positive control.
use super::{debug_fixture::Fixture, io, isolated::pause, screen::Client};
use anyhow::{Context, Result, ensure};
use clap::Parser;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "./target/release/nexode")]
    binary: PathBuf,
    #[arg(long, default_value_t = 3)]
    turns: usize,
    #[arg(long)]
    keep: bool,
    #[arg(long)]
    no_self_test: bool,
    #[arg(short, long)]
    verbose: bool,
}
struct View {
    client: Client,
    cmd: PathBuf,
    resp: PathBuf,
}
impl View {
    fn launch(f: &Fixture, binary: &Path, id: &str, dir: &Path, n: usize) -> Result<Self> {
        let cmd = dir.join(format!("cmd{n}"));
        let resp = dir.join(format!("resp{n}"));
        let mut c = f.client_command(binary);
        c.args(["--resume", id])
            .env("NEXODE_DEBUG_CONTROL", "1")
            .env("NEXODE_DEBUG_CMD_PATH", &cmd)
            .env("NEXODE_DEBUG_RESPONSE_PATH", &resp)
            .env("NEXODE_THEME", "dark")
            .env("TERM", "xterm-256color");
        let view = Self {
            client: Client::spawn(&mut c, 40, 140)?,
            cmd,
            resp,
        };
        view.ready()?;
        Ok(view)
    }
    fn command(&self, c: &str) -> Result<String> {
        io::client_command(&self.cmd, &self.resp, c, Duration::from_secs(8))
    }
    fn ready(&self) -> Result<Value> {
        let start = Instant::now();
        loop {
            if let Ok(s) = self.command("history")
                && let Ok(v) = serde_json::from_str::<Value>(&s)
                && v.is_array()
            {
                return Ok(v);
            }
            ensure!(
                start.elapsed() < Duration::from_secs(60) && !self.client.snapshot()?.eof,
                "client history unavailable"
            );
            pause(0.1)?;
        }
    }
}
fn seed(home: &Path, cwd: &Path, turns: usize) -> Result<(String, Vec<String>)> {
    let mut bytes = [0u8; 8];
    getrandom::fill(&mut bytes).map_err(|e| anyhow::anyhow!("random id: {e}"))?;
    let suffix = bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
    let id = format!(
        "session_repro_{}_{}",
        chrono::Utc::now().timestamp_millis(),
        suffix
    );
    let now = chrono::Utc::now().to_rfc3339();
    let mut markers = vec!["MARK_USER_OPEN".to_owned()];
    let mut messages = vec![];
    let mut message = |role: &str, content: Value| {
        messages.push(json!({"id":format!("message_{}_{}",suffix,messages.len()),"role":role,"content":content,"timestamp":now}));
    };
    message(
        "user",
        json!([{"type":"text","text":"MARK_USER_OPEN please run the repro tasks"}]),
    );
    for i in 0..turns {
        let a = format!("MARK_ASSIST_{i:02}");
        let ti = format!("MARK_TOOLIN_{i:02}");
        let to = format!("MARK_TOOLOUT_{i:02}");
        let tool_id = format!("toolu_{suffix}_{i}");
        message(
            "assistant",
            json!([{"type":"text","text":format!("{a} I'll inspect step {i}.")},{"type":"tool_use","id":tool_id,"name":"bash","input":{"command":format!("echo {ti}"),"intent":format!("repro step {i}")}}]),
        );
        message(
            "user",
            json!([{"type":"tool_result","tool_use_id":tool_id,"content":format!("{to} step {i} ok")} ]),
        );
        markers.extend([a, ti, to]);
    }
    message(
        "assistant",
        json!([{"type":"text","text":"MARK_ASSIST_FINAL all steps complete"}]),
    );
    markers.push("MARK_ASSIST_FINAL".into());
    let session = json!({"id":id,"parent_id":null,"title":"repro-314","created_at":now,"updated_at":now,"messages":messages,"provider_key":"openai","model":"gpt-5.5","reasoning_effort":"low","is_canary":false,"testing_build":null,"working_dir":cwd,"short_name":"repro","status":"Active","last_pid":null,"last_active_at":now,"is_debug":true,"saved":false,"env_snapshots":[]});
    fs::create_dir_all(home.join("sessions"))?;
    fs::write(
        home.join("sessions").join(format!("{id}.json")),
        serde_json::to_vec_pretty(&session)?,
    )?;
    Ok((id, markers))
}
fn counts(history: &Value, markers: &[String]) -> Result<BTreeMap<String, usize>> {
    let messages = history.as_array().context("history is not an array")?;
    let mut counts = markers
        .iter()
        .map(|m| (m.clone(), 0))
        .collect::<BTreeMap<_, _>>();
    for msg in messages {
        let text = msg.to_string();
        for (m, n) in &mut counts {
            if text.contains(m.as_str()) {
                *n += 1;
            }
        }
    }
    Ok(counts)
}
fn check(stage: &str, name: &str, view: &View, markers: &[String]) -> Result<Value> {
    let history = view.ready()?;
    let counts = counts(&history, markers)?;
    ensure!(
        counts.get("MARK_USER_OPEN").copied().unwrap_or(0) > 0
            && counts.get("MARK_ASSIST_FINAL").copied().unwrap_or(0) > 0,
        "{stage}/{name}: seed transcript not rendered"
    );
    ensure!(
        counts
            .iter()
            .filter(|(m, _)| !m.starts_with("MARK_TOOLIN_"))
            .all(|(_, n)| *n > 0),
        "{stage}/{name}: visible marker missing"
    );
    let duplicates = counts
        .iter()
        .filter(|(_, n)| **n > 1)
        .map(|(m, n)| (m.clone(), *n))
        .collect::<BTreeMap<_, _>>();
    Ok(
        json!({"stage":stage,"client":name,"display_count":history.as_array().unwrap().len(),"marker_counts":counts,"duplicates":duplicates}),
    )
}
pub(super) fn run(args: impl Iterator<Item = String>) -> Result<i32> {
    match inner(args) {
        Ok(c) => Ok(c),
        Err(e) => {
            eprintln!("duplicate probe invalid: {e:#}");
            Ok(3)
        }
    }
}
fn inner(args: impl Iterator<Item = String>) -> Result<i32> {
    let a = Args::try_parse_from(std::iter::once("repro-live-duplicate".into()).chain(args))?;
    ensure!((1..=100).contains(&a.turns), "turns must be 1..100");
    let binary = a.binary.canonicalize()?;
    let f = Fixture::new(&binary)?;
    let root = tempfile::tempdir()?;
    let (id, markers) = seed(&f.home(), &std::env::current_dir()?, a.turns)?;
    let a_view = View::launch(&f, &binary, &id, root.path(), 0)?;
    pause(0.5)?;
    let deadline = Instant::now() + Duration::from_secs(30);
    let truth_response = loop {
        let response = io::debug_response(&f.socket, "history", Some(&id), Duration::from_secs(5))?;
        if response["ok"] == true {
            break response;
        }
        ensure!(
            Instant::now() < deadline,
            "server did not finish resuming seeded session"
        );
        pause(0.1)?;
    };
    let truth: Value = serde_json::from_str(
        truth_response["output"]
            .as_str()
            .context("missing history")?,
    )?;
    let truth_counts = counts(&truth, &markers)?;
    ensure!(
        truth_counts.values().all(|n| *n == 1),
        "server transcript has missing or duplicated seed markers"
    );
    let mut results = vec![check("fresh-resume", "A", &a_view, &markers)?];
    let b_view = View::launch(&f, &binary, &id, root.path(), 1)?;
    pause(1.0)?;
    for (name, view) in [("A", &a_view), ("B", &b_view)] {
        results.push(check("second-attach", name, view, &markers)?);
    }
    ensure!(
        b_view.command("reload")?.contains("reload triggered"),
        "reload not triggered"
    );
    pause(2.0)?;
    for (name, view) in [("A", &a_view), ("B", &b_view)] {
        results.push(check("post-reload", name, view, &markers)?);
    }
    drop(a_view);
    drop(b_view);
    pause(1.0)?;
    let c_view = View::launch(&f, &binary, &id, root.path(), 2)?;
    pause(0.5)?;
    results.push(check("detach-reattach", "C", &c_view, &markers)?);
    let control = if a.no_self_test {
        json!({"status":"skipped"})
    } else {
        let marker = "MARK_SELFTEST_FORCED".to_owned();
        for _ in 0..2 {
            c_view.command(&format!("inject:assistant:{marker} injected duplicate"))?;
            pause(0.3)?;
        }
        let count = counts(&c_view.ready()?, std::slice::from_ref(&marker))?[&marker];
        ensure!(
            count == 2,
            "positive control failed to detect injected duplicate"
        );
        json!({"status":"passed","copies":count})
    };
    let reproduced = results
        .iter()
        .any(|v| !v["duplicates"].as_object().unwrap().is_empty());
    let mut out = json!({"stages":results,"server_marker_counts":truth_counts,"positive_control":control,"duplication_reproduced":reproduced});
    io::debug(
        &f.socket,
        &format!("destroy_session:{id}"),
        Duration::from_secs(10),
    )?;
    drop(c_view);
    if a.keep {
        out["artifacts"] = json!(root.keep());
        out["server_artifacts"] = json!(f.retain_artifacts());
    }
    if a.verbose {
        eprintln!("Verified six per-client snapshots and positive-control status");
    }
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(if reproduced { 2 } else { 0 })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn counts_messages_not_repeated_metadata_and_rejects_bad_shape() {
        let m = vec!["MARK_TEST".into()];
        assert_eq!(
            counts(&json!([{"content":"MARK_TEST","title":"MARK_TEST"}]), &m).unwrap()["MARK_TEST"],
            1
        );
        assert_eq!(
            counts(
                &json!([{"content":"MARK_TEST"},{"content":"MARK_TEST"}]),
                &m
            )
            .unwrap()["MARK_TEST"],
            2
        );
        assert!(counts(&json!({}), &m).is_err());
    }
}
