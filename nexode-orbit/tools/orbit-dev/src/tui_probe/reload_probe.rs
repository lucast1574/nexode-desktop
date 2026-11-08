//! Reload readiness diagnostics. Actual signal/timeout contracts live in runtime tests.
use super::{
    debug_fixture::{Fixture, OwnedSession},
    io, selfdev_status,
};
use anyhow::{Context, Result, ensure};
use clap::Parser;
use serde_json::{Value, json};
use std::{
    fs,
    path::PathBuf,
    time::{Duration, Instant, SystemTime},
};
#[derive(Parser)]
struct Args {
    #[arg(long)]
    fresh: bool,
    #[arg(long, default_value = "./target/release/nexode")]
    binary: PathBuf,
    #[arg(long, required_unless_present = "fresh", conflicts_with = "fresh")]
    socket: Option<PathBuf>,
    #[arg(long, default_value = ".")]
    repo: PathBuf,
    #[arg(long)]
    nexode_home: Option<PathBuf>,
    #[arg(short, long)]
    verbose: bool,
    #[arg(short, long)]
    test: Option<String>,
}
fn observations(home: &std::path::Path) -> Result<Value> {
    let manifest = home.join("build-manifest.json");
    let canary = if manifest.exists() {
        let v: Value = serde_json::from_slice(&fs::read(manifest)?)?;
        json!({"status":"observed","canary":v["canary"],"canary_binary_exists":home.join("builds/canary/nexode").exists()})
    } else {
        json!({"status":"skipped","reason":"no build manifest"})
    };
    let info = home.join("reload-info");
    let marker = if info.exists() {
        let bytes = fs::read(&info)?;
        let age = SystemTime::now()
            .duration_since(fs::metadata(&info)?.modified()?)
            .unwrap_or_default()
            .as_secs();
        json!({"status":if age>600{"warning"}else{"observed"},"age_seconds":age,"has_reload_prefix":bytes.starts_with(b"reload:")})
    } else {
        json!({"status":"skipped","reason":"no reload-info marker"})
    };
    Ok(json!({"canary":canary,"reload_info":marker,"context":selfdev_status::context(home)?}))
}
pub(super) fn run(args: impl Iterator<Item = String>) -> Result<i32> {
    let a = Args::try_parse_from(std::iter::once("test-reload".into()).chain(args))?;
    let repo = a.repo.canonicalize()?;
    let fixture = if a.fresh {
        Some(Fixture::new(&a.binary.canonicalize()?)?)
    } else {
        None
    };
    let socket = fixture
        .as_ref()
        .map(|f| &f.socket)
        .or(a.socket.as_ref())
        .context("socket required")?;
    let home = fixture
        .as_ref()
        .map(Fixture::home)
        .or(a.nexode_home)
        .or_else(|| std::env::var_os("NEXODE_HOME").map(PathBuf::from))
        .or_else(|| dirs::home_dir().map(|p| p.join(".nexode")))
        .context("home required")?;
    let mut session = OwnedSession::new(socket, &repo)?;
    let mut results = vec![];
    let names = [
        "debug socket and sessions",
        "swarm members",
        "session state",
        "selfdev status",
        "selfdev socket-info",
        "reload files and canary observations",
        "idle query responsiveness",
        "rapid requests without deadlock",
        "session lifecycle",
        "multiple sessions responsive",
        "help reference",
        "swarm session detail",
    ];
    let mut failed = false;
    for name in names {
        if a.test.as_ref().is_some_and(|s| !name.contains(s)) {
            continue;
        }
        let result = (|| -> Result<Value> {
            match name {
                "debug socket and sessions" => {
                    let v: Value = serde_json::from_str(&io::debug(
                        socket,
                        "sessions",
                        Duration::from_secs(10),
                    )?)?;
                    ensure!(v.is_array(), "sessions must be an array");
                    Ok(json!({"connected_sessions":v.as_array().unwrap().len()}))
                }
                "swarm members" => {
                    let v: Value = serde_json::from_str(&io::debug(
                        socket,
                        "swarm:members",
                        Duration::from_secs(10),
                    )?)?;
                    let rows = v.as_array().context("members must be array")?;
                    ensure!(
                        rows.iter()
                            .all(|v| v["session_id"].is_string() && v["status"].is_string()),
                        "invalid member"
                    );
                    Ok(json!({"members":rows.len()}))
                }
                "session state" => {
                    let v: Value = serde_json::from_str(&session.command("state")?)?;
                    ensure!(v["session_id"] == session.id, "wrong session state");
                    Ok(json!({}))
                }
                "selfdev status" => selfdev_status::probe(socket, &repo, "status"),
                "selfdev socket-info" => selfdev_status::probe(socket, &repo, "socket-info"),
                "reload files and canary observations" => observations(&home),
                "idle query responsiveness" => {
                    let start = Instant::now();
                    for _ in 0..5 {
                        io::debug(socket, "swarm:members", Duration::from_secs(2))?;
                    }
                    ensure!(
                        start.elapsed() < Duration::from_secs(2),
                        "idle queries exceeded 2s"
                    );
                    Ok(
                        json!({"five_queries_ms":start.elapsed().as_secs_f64()*1000.0,"shutdown_inference":false}),
                    )
                }
                "rapid requests without deadlock" => {
                    let mut times = vec![];
                    for _ in 0..20 {
                        let start = Instant::now();
                        io::debug(socket, "sessions", Duration::from_secs(3))?;
                        ensure!(
                            start.elapsed() < Duration::from_secs(3),
                            "request exceeded 3s"
                        );
                        times.push(start.elapsed().as_secs_f64() * 1000.0);
                    }
                    Ok(json!({"samples_ms":times}))
                }
                "session lifecycle" => {
                    let mut s = OwnedSession::new(socket, &repo)?;
                    let state: Value = serde_json::from_str(&s.command("state")?)?;
                    ensure!(state["session_id"] == s.id, "created session missing");
                    s.close()?;
                    Ok(json!({"created_and_destroyed":true}))
                }
                "multiple sessions responsive" => {
                    let mut sessions = (0..3)
                        .map(|_| OwnedSession::new(socket, &repo))
                        .collect::<Result<Vec<_>>>()?;
                    let start = Instant::now();
                    io::debug(socket, "swarm:members", Duration::from_secs(2))?;
                    ensure!(
                        start.elapsed() < Duration::from_millis(1500),
                        "queries slowed under load"
                    );
                    for s in &mut sessions {
                        s.close()?;
                    }
                    Ok(json!({"sessions":3}))
                }
                "help reference" => {
                    let help = io::debug(socket, "help", Duration::from_secs(10))?;
                    ensure!(
                        help.len() > 100 && (help.contains("message:") || help.contains("tool:")),
                        "missing help reference"
                    );
                    Ok(json!({"characters":help.len()}))
                }
                "swarm session detail" => {
                    let v = io::debug(
                        socket,
                        &format!("swarm:session:{}", session.id),
                        Duration::from_secs(10),
                    )?;
                    ensure!(
                        v.contains(&session.id) || v.to_lowercase().contains("session"),
                        "invalid session detail"
                    );
                    Ok(json!({}))
                }
                _ => unreachable!(),
            }
        })();
        match result{Ok(detail)=>results.push(json!({"test":name,"status":detail["status"].as_str().unwrap_or("passed"),"detail":detail})),Err(e)=>{failed=true;results.push(json!({"test":name,"status":"failed","error":e.to_string()}));}}
        if a.verbose {
            eprintln!("{}", results.last().unwrap());
        }
    }
    ensure!(!results.is_empty(), "no tests match filter");
    session.close()?;
    println!(
        "{}",
        serde_json::to_string_pretty(
            &json!({"results":results,"failed":failed,"signal_timeout_and_context_contracts":"covered by native nexode-app-core reload_tests, reload_marker_scope_tests and selfdev reload_context tests; responsiveness alone does not prove shutdown"})
        )?
    );
    Ok(i32::from(failed))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn observes_reload_files_without_replacing_or_consuming_them() {
        let root = tempfile::tempdir().unwrap();
        let info = root.path().join("reload-info");
        fs::write(&info, "reload:fixture").unwrap();
        fs::File::options()
            .write(true)
            .open(&info)
            .unwrap()
            .set_modified(SystemTime::UNIX_EPOCH)
            .unwrap();
        fs::write(
            root.path().join("build-manifest.json"),
            r#"{"canary":"abc"}"#,
        )
        .unwrap();
        let report = observations(root.path()).unwrap();
        assert_eq!(report["reload_info"]["status"], "warning");
        assert_eq!(report["canary"]["canary_binary_exists"], false);
        assert_eq!(report["context"]["status"], "skipped");
        assert_eq!(fs::read_to_string(info).unwrap(), "reload:fixture");
        fs::write(root.path().join("build-manifest.json"), "malformed").unwrap();
        assert!(observations(root.path()).is_err());
    }
}
