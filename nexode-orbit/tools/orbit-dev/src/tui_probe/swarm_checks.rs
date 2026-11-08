//! Real swarm coordination checks, scoped to disposable workspaces/sessions.
use super::{
    debug_fixture::{Fixture, OwnedSession},
    io,
};
use anyhow::{Context, Result, ensure};
use clap::Parser;
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
#[derive(Parser)]
struct Args {
    #[arg(long)]
    fresh: bool,
    #[arg(long, required_unless_present = "fresh", conflicts_with = "fresh")]
    socket: Option<PathBuf>,
    #[arg(long, default_value = "./target/release/nexode")]
    binary: PathBuf,
    #[arg(long)]
    json: Option<PathBuf>,
}
fn call(socket: &Path, cmd: &str, session: Option<&str>) -> Result<Value> {
    let v = io::debug_response(socket, cmd, session, Duration::from_secs(15))
        .with_context(|| format!("command {cmd}"))?;
    ensure!(v["ok"] != false && v["type"] != "error", "{cmd}: {v}");
    let text = v["output"].as_str().unwrap_or("");
    Ok(serde_json::from_str(text).unwrap_or(json!(text)))
}
fn rejected(socket: &Path, cmd: &str, session: Option<&str>, fragment: &str) -> Result<()> {
    let v = io::debug_response(socket, cmd, session, Duration::from_secs(15))
        .with_context(|| format!("command {cmd}"))?;
    ensure!(
        v["ok"] == false || v["type"] == "error",
        "unexpected success: {cmd}"
    );
    ensure!(
        v.to_string().to_lowercase().contains(fragment),
        "unexpected rejection: {v}"
    );
    Ok(())
}
fn fields(v: &Value, names: &[&str]) -> Result<()> {
    for name in names {
        ensure!(v.get(name).is_some(), "missing {name}: {v}");
    }
    Ok(())
}
fn rows(v: &Value) -> Result<&Vec<Value>> {
    v.as_array().context("expected JSON array")
}
fn coordination(socket: &Path) -> Result<Value> {
    let work = tempfile::Builder::new().prefix("orbit-swarm-").tempdir()?;
    let mut s1 = OwnedSession::new(socket, work.path())?;
    let mut s2 = OwnedSession::new(socket, work.path())?;
    let provenance = call(socket, &format!("swarm:id:{}", work.path().display()), None)?;
    fields(
        &provenance,
        &["path", "swarm_id", "git_root", "is_git_repo"],
    )?;
    ensure!(
        provenance["is_git_repo"] == false,
        "scratch workspace incorrectly git"
    );
    let detail1 = call(socket, &format!("swarm:session:{}", s1.id), None)?;
    let detail2 = call(socket, &format!("swarm:session:{}", s2.id), None)?;
    let swarm1 = detail1["swarm_id"].as_str().context("session swarm id")?;
    let swarm2 = detail2["swarm_id"].as_str().context("session swarm id")?;
    ensure!(
        swarm1 != swarm2,
        "independent headless roots shared a swarm"
    );
    let coords = call(socket, "swarm:coordinators", None)?;
    ensure!(
        !rows(&coords)?
            .iter()
            .any(|c| c["swarm_id"] == swarm1 || c["swarm_id"] == swarm2),
        "headless root auto-claimed coordinator"
    );
    let members = call(socket, "swarm:members", None)?;
    for sid in [&s1.id, &s2.id] {
        let member = rows(&members)?
            .iter()
            .find(|r| r["session_id"] == *sid)
            .context("member missing")?;
        fields(member, &["joined_secs_ago", "status_changed_secs_ago"])?;
    }
    let roles = call(socket, "swarm:roles", None)?;
    for sid in [&s1.id, &s2.id] {
        ensure!(
            rows(&roles)?
                .iter()
                .any(|r| r["session_id"] == *sid && r["is_coordinator"] == false),
            "headless role contract changed"
        );
    }
    let broadcast = call(
        socket,
        &format!("swarm:broadcast:{swarm1} native swarm probe"),
        Some(&s1.id),
    )?;
    fields(&broadcast, &["sent_to"])?;
    let dm = call(
        socket,
        &format!("swarm:notify:{} native direct probe", s2.id),
        Some(&s1.id),
    )?;
    ensure!(dm["sent_to"] == s2.id, "DM target mismatch");
    rejected(
        socket,
        "swarm:notify:nonexistent_session_native_probe hello",
        Some(&s1.id),
        "session",
    )?;
    rejected(
        socket,
        "swarm:session:nonexistent_session_native_probe",
        None,
        "session",
    )?;
    let key = format!("plan_proposal:{}", s1.id);
    let items = json!([{"id":"native_proposal_probe","content":"Implement test feature","status":"pending","priority":"normal"}]);
    call(
        socket,
        &format!("swarm:set_context:{} {key} {items}", s1.id),
        None,
    )?;
    call(socket, &format!("swarm:context:{swarm1}:{key}"), None)?;
    let proposals = call(socket, &format!("swarm:proposals:{swarm1}"), None)?;
    ensure!(!rows(&proposals)?.is_empty(), "proposal missing");
    let context = call(socket, "swarm:context", None)?;
    ensure!(!rows(&context)?.is_empty(), "context missing");
    for c in rows(&context)? {
        fields(c, &["created_secs_ago", "updated_secs_ago"])?;
    }
    rejected(
        socket,
        &format!("swarm:approve_plan:{} {}", s2.id, s1.id),
        None,
        "coordinator",
    )?;
    rejected(
        socket,
        &format!("swarm:reject_plan:{} {} Not authorized", s2.id, s1.id),
        None,
        "coordinator",
    )?;
    // Rejected actions must leave the proposal available.
    call(socket, &format!("swarm:context:{swarm1}:{key}"), None)?;
    s1.close()?;
    s2.close()?;
    Ok(
        json!({"headless_roots_isolated":true,"no_headless_auto_coordinator":true,"communication":true,"invalid_dm":true,"non_git_provenance":true,"proposal_context":true,"unauthorized_approval_and_rejection":true,"member_and_context_timestamps":true,"coordinator_plan_dag_coverage":"runtime comm_plan_tests; not inferred from headless responsiveness"}),
    )
}
fn diagnostics(socket: &Path) -> Result<Value> {
    let mut checked = vec![];
    for command in [
        "swarm:list",
        "swarm:members",
        "swarm:coordinators",
        "swarm:plans",
        "swarm:context",
        "swarm:touches",
        "swarm:conflicts",
        "swarm:interrupts",
        "swarm:proposals",
        "swarm:proposals:/tmp",
        "swarm:touches:swarm:/tmp",
        "events:recent",
        "events:recent:10",
        "events:since:0",
    ] {
        let v = call(socket, command, None)?;
        rows(&v)?;
        if command == "swarm:touches" {
            for row in rows(&v)? {
                fields(row, &["timestamp_unix"])?;
            }
        }
        if command == "swarm:conflicts" {
            for row in rows(&v)? {
                fields(row, &["accesses"])?;
            }
        }
        checked.push(command);
    }
    let count = call(socket, "events:count", None)?;
    fields(&count, &["count", "latest_id", "max_history"])?;
    let types = call(socket, "events:types", None)?;
    ensure!(!rows(&types["types"])?.is_empty(), "event types empty");
    let help = call(socket, "swarm:help", None)?
        .as_str()
        .context("help text")?
        .to_lowercase();
    for fragment in [
        "swarm:proposals",
        "swarm:touches:swarm:",
        "timestamp",
        "swarm:session",
        "events:recent",
        "events:since",
    ] {
        ensure!(help.contains(fragment), "help missing {fragment}");
    }
    Ok(
        json!({"commands":checked,"event_schema":true,"help":true,"touch_and_conflict_schema":"validated on every returned entry; empty lists do not prove recording"}),
    )
}
pub(super) fn run(args: impl Iterator<Item = String>) -> Result<i32> {
    let a = Args::try_parse_from(std::iter::once("test-swarm".into()).chain(args))?;
    let fixture = if a.fresh {
        Some(Fixture::new(&a.binary.canonicalize()?)?)
    } else {
        None
    };
    let socket = fixture
        .as_ref()
        .map(|f| &f.socket)
        .or(a.socket.as_ref())
        .context("socket")?;
    let mut results = vec![];
    for (name, f) in [
        ("coordination", coordination as fn(&Path) -> Result<Value>),
        ("debug diagnostics", diagnostics),
    ] {
        match f(socket) {
            Ok(v) => results.push(json!({"name":name,"passed":true,"evidence":v})),
            Err(e) => results.push(json!({"name":name,"passed":false,"error":format!("{e:#}")})),
        }
    }
    let passed = results.iter().all(|r| r["passed"] == true);
    let report = json!({"passed":passed,"results":results});
    let bytes = serde_json::to_vec_pretty(&report)?;
    if let Some(p) = a.json {
        std::fs::write(p, &bytes)?;
    }
    println!("{}", String::from_utf8(bytes)?);
    Ok(i32::from(!passed))
}
