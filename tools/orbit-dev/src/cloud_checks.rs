use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{fs, path::PathBuf};
mod group;
mod ssh;
fn read(path: &str) -> Result<Value> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}
fn content(messages: &Value) -> Vec<String> {
    messages
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|m| m["content"].as_array().into_iter().flatten())
        .filter(|b| b["type"] == "text")
        .filter_map(|b| b["text"].as_str().map(String::from))
        .collect()
}
pub fn run(args: impl Iterator<Item = String>) -> Result<()> {
    let args: Vec<_> = args.collect();
    let command = args.first().context("cloud-fixture command required")?;
    match command.as_str() {
        "free-ports"
        | "ssh-saved-session"
        | "ssh-child-session"
        | "ssh-client-resumed"
        | "assert-ssh-workflows" => ssh::run(command, &args[1..])?,
        "assert-environment-status" => {
            ensure!(args.len() == 3, "assert-environment-status PATH BOOL");
            let value = read(&args[1])?;
            let object = value.as_object().context("status must be an object")?;
            ensure!(
                object.len() == 4
                    && ["ok", "code", "applied", "skipped"]
                        .iter()
                        .all(|k| object.contains_key(*k)),
                "unexpected status fields"
            );
            ensure!(
                value["ok"].as_bool() == Some(args[2].parse::<bool>()?),
                "unexpected status result"
            );
            ensure!(
                matches!(
                    value["code"].as_str(),
                    Some("available" | "received" | "unavailable")
                ),
                "unexpected status code"
            );
            ensure!(
                value["applied"].is_i64() && value["skipped"].is_i64(),
                "status counts must be integers"
            );
        }
        "assert-environment-token" => {
            use std::os::unix::fs::PermissionsExt;
            ensure!(args.len() == 2, "assert-environment-token DIRECTORY");
            let files = fs::read_dir(&args[1])?
                .collect::<std::io::Result<Vec<_>>>()?
                .into_iter()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == "json"))
                .collect::<Vec<_>>();
            ensure!(files.len() == 1, "expected exactly one environment file");
            let value: Value = serde_json::from_slice(&fs::read(&files[0])?)?;
            ensure!(
                value["CLOUD_TEST_RESOURCE_TOKEN"] == "opaque-token-sentinel",
                "token did not transfer"
            );
            ensure!(
                fs::metadata(&files[0])?.permissions().mode() & 0o777 == 0o600,
                "environment token permissions are not private"
            );
        }
        "replacement-backup" => {
            ensure!(
                args.len() == 4,
                "replacement-backup REPORT LOCAL_ROOT CLOUD_ROOT"
            );
            let value = read(&args[1])?;
            let path = value["backup_path"]
                .as_str()
                .context("backup path missing")?;
            let relative = path
                .strip_prefix(&format!("{}/", args[2]))
                .context("backup is outside the local fixture")?;
            ensure!(
                !relative.is_empty()
                    && std::path::Path::new(relative)
                        .components()
                        .all(|c| matches!(c, std::path::Component::Normal(_))),
                "invalid backup path"
            );
            println!("{}", PathBuf::from(&args[3]).join(relative).display());
        }
        "seed-group"
        | "assert-group-away"
        | "assert-group-recovered"
        | "assert-group-environment" => group::run(command, &args[1..])?,
        "wait-paths" => {
            ensure!(args.len() > 1, "wait-paths requires paths");
            for path in &args[1..] {
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
                while !std::path::Path::new(path).exists() {
                    ensure!(
                        crate::ci_suites::unix::interrupted().is_none(),
                        "wait interrupted"
                    );
                    ensure!(
                        std::time::Instant::now() < deadline,
                        "daemon did not start: {path}"
                    );
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
            }
        }
        "seed-session" => {
            ensure!(args.len() == 4, "seed-session PATH SID REPO");
            let now = chrono::Utc::now().to_rfc3339();
            let message = |i: u32, role: &str, blocks: Value| json!({"id":format!("message_{i}"),"role":role,"content":blocks,"timestamp":now});
            let session = json!({"id":args[2],"title":"cloud e2e","created_at":now,"updated_at":now,"working_dir":args[3],"status":"Active","messages":[message(1,"user",json!([{"type":"text","text":"Please change main.rs to print v2"}])),message(2,"assistant",json!([{"type":"text","text":"Editing main.rs now."},{"type":"tool_use","id":"toolu_1","name":"bash","input":{"command":"nexode cloud move"}}]))]});
            fs::write(&args[1], serde_json::to_vec(&session)?)?;
        }
        "append-cloud-turn" => {
            ensure!(args.len() == 2, "append-cloud-turn PATH");
            let mut session = read(&args[1])?;
            session["messages"].as_array_mut().context("messages missing")?.push(json!({"id":"message_cloud","role":"assistant","content":[{"type":"text","text":"Done on the cloud: committed v2 and added cloud.txt."}]}));
            fs::write(&args[1], serde_json::to_vec(&session)?)?;
        }
        "assert-move-notice" => {
            ensure!(args.len() == 2, "assert-move-notice PATH");
            let session = read(&args[1])?;
            let last = session["messages"]
                .as_array()
                .and_then(|m| m.last())
                .context("no messages")?;
            let notice = content(&json!([last])).join("\n");
            for needle in [
                "moved from `laptop-e2e` to cloud host `cloud-e2e`",
                "same path",
                "uncommitted",
                "Build caches",
            ] {
                ensure!(
                    notice.contains(needle),
                    "missing migration notice: {needle}"
                );
            }
            let result = last["content"]
                .as_array()
                .context("no content")?
                .iter()
                .find(|b| b["type"] == "tool_result");
            ensure!(
                result.is_some_and(|r| r["tool_use_id"] == "toolu_1"),
                "dangling tool call not closed"
            );
            ensure!(session["migration_epoch"] == 1, "wrong move epoch");
            println!("PASS agent-facing migration notice + dangling tool call closed");
        }
        "assert-return-notice" => {
            ensure!(args.len() == 2, "assert-return-notice PATH");
            let session = read(&args[1])?;
            let texts = content(&session["messages"]);
            let last = texts.last().context("no text")?;
            ensure!(
                texts.iter().any(|t| t.contains("Done on the cloud")),
                "cloud turn missing"
            );
            ensure!(
                last.contains("returned from cloud host `cloud-e2e`")
                    && last.contains("merged into the local checkout"),
                "return notice missing"
            );
            ensure!(session["migration_epoch"] == 2, "wrong return epoch");
            println!("PASS returned transcript has cloud turns + return notice (epoch 2)");
        }
        "session-id" => {
            ensure!(args.len() == 2, "session-id PATH");
            println!(
                "{}",
                read(&args[1])?["session_id"]
                    .as_str()
                    .context("session_id missing")?
            );
        }
        "assert-cloud-context" | "assert-local-context" => {
            ensure!(args.len() == 2, "context check requires model log");
            let log = fs::read_to_string(&args[1])?;
            let last: Value = serde_json::from_str(log.lines().last().context("model log empty")?)?;
            let text = last["messages"].to_string();
            let local = command == "assert-local-context";
            let needles = if local {
                vec![
                    "turn one on the laptop",
                    "continue the task",
                    "returned from cloud host",
                    "back on the laptop",
                ]
            } else {
                vec!["turn one on the laptop", "continue the task"]
            };
            for needle in needles {
                ensure!(text.contains(needle), "missing model context: {needle}");
            }
            if local {
                ensure!(
                    !text.contains("should be blocked"),
                    "blocked prompt leaked into transcript"
                );
            }
            println!("PASS conversation context preserved");
        }
        "cleanup-workspace" => {
            ensure!(args.len() == 3, "cleanup-workspace ROOT CALLER_PID");
            let root = PathBuf::from(&args[1]).canonicalize()?;
            let caller = args[2].parse::<i32>()?;
            ensure!(caller > 1, "invalid caller PID");
            crate::detached_acceptance::cleanup_workspace_except(&root, &[caller])?;
        }
        _ => anyhow::bail!("unknown cloud-fixture command {command}"),
    }
    Ok(())
}
