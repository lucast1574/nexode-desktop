use super::*;
use std::{net::TcpListener, path::Path};

fn files(directory: &Path, extension: &str) -> Result<Vec<PathBuf>> {
    if !directory.exists() {
        return Ok(vec![]);
    }
    Ok(fs::read_dir(directory)?
        .collect::<std::io::Result<Vec<_>>>()?
        .into_iter()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == extension))
        .collect())
}
fn values(directory: &Path) -> Result<Vec<Value>> {
    files(directory, "json")?
        .iter()
        .map(|p| Ok(serde_json::from_slice(&fs::read(p)?)?))
        .collect()
}
pub(super) fn run(command: &str, args: &[String]) -> Result<()> {
    match command {
        "free-ports" => {
            ensure!(args.is_empty(), "free-ports takes no arguments");
            let ssh = TcpListener::bind("127.0.0.1:0")?;
            let model = TcpListener::bind("127.0.0.1:0")?;
            println!(
                "{} {}",
                ssh.local_addr()?.port(),
                model.local_addr()?.port()
            );
        }
        "ssh-saved-session" | "ssh-child-session" => {
            ensure!(
                args.len() == if command == "ssh-saved-session" { 1 } else { 2 },
                "expected workspace and optional parent"
            );
            let sessions = values(&Path::new(&args[0]).join("remote-home/.nexode/sessions"))?;
            let found = sessions.iter().find(|s| {
                if command == "ssh-saved-session" {
                    s["saved"] == true
                } else {
                    s["parent_id"] == args[1]
                }
            });
            if let Some(session) = found {
                println!("{}", session["id"].as_str().context("session id missing")?);
            } else {
                ensure!(
                    command == "ssh-child-session",
                    "bookmark was not persisted on the host"
                );
            }
        }
        "ssh-client-resumed" => {
            ensure!(args.len() == 2, "ssh-client-resumed PID SESSION");
            let pid: u32 = args[0].parse()?;
            let bytes = fs::read(format!("/proc/{pid}/cmdline"))?;
            let argv: Vec<_> = bytes.split(|b| *b == 0).collect();
            ensure!(
                argv.contains(&b"--resume".as_slice()) && argv.contains(&args[1].as_bytes()),
                "client has not resumed the original session"
            );
        }
        "assert-ssh-workflows" => {
            ensure!(args.len() == 1, "assert-ssh-workflows WORKSPACE");
            let w = Path::new(&args[0]);
            let home = w.join("remote-home/.nexode");
            let config: toml::Value =
                toml::from_str(&fs::read_to_string(home.join("config.toml"))?)?;
            ensure!(
                config["display"]["centered"].as_bool() == Some(true),
                "host display setting not persisted"
            );
            ensure!(
                config["provider"]["default_model"].as_str() == Some("fake-model"),
                "host default model not persisted"
            );
            ensure!(
                config["provider"]["openai_service_tier"].as_str() == Some("off"),
                "host fast setting not persisted"
            );
            ensure!(
                fs::read_to_string(w.join("project/.nexode/swarm-prompt.md"))?
                    == "isolated-prompt-marker",
                "host swarm prompt mismatch"
            );
            let loops = values(&home.join("remote-loops"))?;
            ensure!(
                !loops.is_empty() && loops.iter().all(|v| v["workflow"]["active"] != true),
                "workflow was not persisted and stopped"
            );
            let logs = files(&home.join("logs"), "log")?
                .iter()
                .map(fs::read_to_string)
                .collect::<std::io::Result<Vec<_>>>()?;
            ensure!(
                logs.iter().any(|s| s.contains("isolated-log-marker")),
                "host log marker missing"
            );
            ensure!(
                values(&home.join("sessions"))?
                    .iter()
                    .any(|s| s["parent_id"].as_str().is_some_and(|s| !s.is_empty())),
                "fork did not persist a remote child"
            );
            let rows = fs::read_to_string(w.join("model.log"))?
                .lines()
                .map(serde_json::from_str::<Value>)
                .collect::<std::result::Result<Vec<_>, _>>()?;
            ensure!(
                rows.iter()
                    .any(|r| r.to_string().contains("isolated-workflow-marker")),
                "workflow never reached model"
            );
            ensure!(
                rows.iter().filter(|r| r["agent"] == true).count() == 1,
                "control commands started an extra model turn"
            );
            ensure!(
                fs::read_to_string(w.join("local-project/host.txt"))? == "client-file-sentinel\n",
                "client file changed"
            );
            ensure!(
                files(&w.join("local-home/.nexode/sessions"), "json")?.is_empty(),
                "remote sessions saved locally"
            );
        }
        _ => anyhow::bail!("unknown SSH fixture operation"),
    }
    Ok(())
}
