//! Opt-in status/socket/context probe; does not trigger a build or reload.
use super::io;
use anyhow::{Context, Result, ensure};
use clap::Parser;
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Parser)]
struct Args {
    #[arg(long)]
    socket: PathBuf,
    #[arg(long, default_value = ".")]
    repo: PathBuf,
    #[arg(long)]
    nexode_home: PathBuf,
}
struct Session<'a> {
    socket: &'a Path,
    id: String,
    closed: bool,
}
impl Session<'_> {
    fn close(&mut self) -> Result<()> {
        io::debug(
            self.socket,
            &format!("destroy_session:{}", self.id),
            Duration::from_secs(10),
        )?;
        self.closed = true;
        Ok(())
    }
}
impl Drop for Session<'_> {
    fn drop(&mut self) {
        if !self.closed
            && let Err(error) = self.close()
        {
            eprintln!("selfdev probe session cleanup failed: {error:#}");
        }
    }
}
fn probe(socket: &Path, repo: &Path, action: &str) -> Result<Value> {
    let output = io::debug(
        socket,
        &format!("create_session:selfdev:{}", repo.display()),
        Duration::from_secs(30),
    )?;
    let created: Value = serde_json::from_str(&output)?;
    let mut session = Session {
        socket,
        id: created["session_id"]
            .as_str()
            .context("missing session id")?
            .into(),
        closed: false,
    };
    let response = io::debug_response(
        socket,
        &format!("tool:selfdev {}", json!({"action":action})),
        Some(&session.id),
        Duration::from_secs(30),
    )?;
    session.close()?;
    if response["ok"] == false || response["type"] == "error" {
        let error = response["error"].as_str().unwrap_or("").to_lowercase();
        ensure!(
            error.contains("not available")
                && (action == "socket-info" || error.contains("selfdev")),
            "selfdev {action} failed"
        );
        return Ok(json!({"action":action,"status":"skipped","reason":"selfdev is not available"}));
    }
    ensure!(response["ok"] == true, "invalid selfdev response");
    let output = response["output"].as_str().unwrap_or("");
    Ok(
        json!({"action":action,"status":"passed","contains_socket_info": action != "socket-info" || output.to_lowercase().contains("socket")}),
    )
}
fn context(home: &Path) -> Result<Value> {
    let mut candidates = if home.exists() {
        fs::read_dir(home)?
            .collect::<std::io::Result<Vec<_>>>()?
            .into_iter()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|s| s.to_str())
                    .is_some_and(|s| s.starts_with("reload-context-") && s.ends_with(".json"))
            })
            .collect::<Vec<_>>()
    } else {
        vec![]
    };
    candidates.sort();
    if home.join("reload-context.json").exists() {
        candidates.push(home.join("reload-context.json"));
    }
    let Some(path) = candidates.first() else {
        return Ok(
            json!({"action":"reload-context","status":"skipped","reason":"no existing reload context"}),
        );
    };
    let value: Value = serde_json::from_slice(&fs::read(path)?)?;
    let object = value
        .as_object()
        .context("reload context must be an object")?;
    let missing: Vec<_> = [
        "task_context",
        "version_before",
        "version_after",
        "session_id",
        "timestamp",
    ]
    .into_iter()
    .filter(|key| !object.contains_key(*key))
    .collect();
    Ok(json!({"action":"reload-context","status":"passed","path":path,"missing_fields":missing}))
}
pub fn run(args: impl Iterator<Item = String>) -> Result<i32> {
    let args = Args::try_parse_from(std::iter::once("test-selfdev-reload".into()).chain(args))?;
    let repo = args.repo.canonicalize()?;
    let mut failed = false;
    for result in [
        probe(&args.socket, &repo, "status"),
        probe(&args.socket, &repo, "socket-info"),
        context(&args.nexode_home),
    ] {
        match result {
            Ok(report) => println!("{report}"),
            Err(error) => {
                failed = true;
                eprintln!("selfdev probe failed: {error:#}");
            }
        }
    }
    Ok(i32::from(failed))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn context_distinguishes_missing_invalid_and_partial_records() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(context(dir.path()).unwrap()["status"], "skipped");
        fs::write(dir.path().join("reload-context.json"), "invalid").unwrap();
        assert!(context(dir.path()).is_err());
        fs::write(dir.path().join("reload-context.json"), "{}").unwrap();
        assert_eq!(
            context(dir.path()).unwrap()["missing_fields"]
                .as_array()
                .unwrap()
                .len(),
            5
        );
    }
    #[test]
    fn unavailable_tool_is_skipped_and_created_session_destroyed() {
        use std::{
            io::{BufRead, BufReader, Write},
            os::unix::net::UnixListener,
        };
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("s");
        let listener = UnixListener::bind(&socket).unwrap();
        let server = std::thread::spawn(move || {
            for expected in [
                "create_session:selfdev:/fixture",
                "tool:selfdev {\"action\":\"status\"}",
                "destroy_session:session_fixture",
            ] {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut line = String::new();
                BufReader::new(stream.try_clone().unwrap())
                    .read_line(&mut line)
                    .unwrap();
                let request: Value = serde_json::from_str(&line).unwrap();
                assert_eq!(request["command"], expected);
                let response = if expected.starts_with("create") {
                    json!({"ok":true,"output":"{\"session_id\":\"session_fixture\"}"})
                } else if expected.starts_with("tool") {
                    assert_eq!(request["session_id"], "session_fixture");
                    json!({"ok":false,"error":"selfdev is not available"})
                } else {
                    json!({"ok":true,"output":""})
                };
                writeln!(stream, "{response}").unwrap();
            }
        });
        assert_eq!(
            probe(&socket, Path::new("/fixture"), "status").unwrap()["status"],
            "skipped"
        );
        server.join().unwrap();
    }
}
