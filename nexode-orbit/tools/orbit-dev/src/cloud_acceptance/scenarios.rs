use super::{tui::CloudTui, *};
pub(super) const NAMES: &[&str] = &[
    "idle_round_trip",
    "midturn_auto_poke",
    "cancel_prepared",
    "draft_preserved",
    "prepare_outage_retry",
    "transfer_outage_retry",
    "destination_binary_retry",
];
fn local(t: &CloudTui) -> Result<()> {
    ensure!(
        t.lease()["away_host"].as_str().is_none_or(str::is_empty),
        "source ownership changed"
    );
    ensure!(
        t.session(false)?["migration_epoch"].as_u64().unwrap_or(0) == 0,
        "migration epoch changed"
    );
    Ok(())
}
pub(super) fn run(name: &str, t: &mut CloudTui) -> Result<()> {
    match name {
        "idle_round_trip" => {
            t.send("acceptance original laptop prompt", true)?;
            t.wait_text("[model] ack")?;
            t.discover_session()?;
            for cycle in 1..=2 {
                t.go_cloud(true)?;
                fs::write(
                    t.cloud_repo.join(format!("cloud-{cycle}.txt")),
                    format!("cloud work {cycle}\n"),
                )?;
                fs::write(
                    t.repo.join(format!("local-{cycle}.txt")),
                    format!("concurrent local work {cycle}\n"),
                )?;
                t.go_local()?;
                ensure!(
                    fs::read_to_string(t.repo.join(format!("cloud-{cycle}.txt")))?
                        == format!("cloud work {cycle}\n"),
                    "cloud work lost"
                );
                ensure!(
                    t.repo.join(format!("local-{cycle}.txt")).exists(),
                    "concurrent local work lost"
                );
                ensure!(
                    t.session(false)?["migration_epoch"] == cycle * 2,
                    "incorrect round-trip epoch"
                );
            }
            ensure!(
                t.records()?.last().context("no model requests")?["messages"]
                    .to_string()
                    .contains("acceptance original laptop prompt"),
                "original context lost"
            );
        }
        "midturn_auto_poke" => {
            t.held_turn("poke", true)?;
            let filename = format!("{}.json", t.sid.as_ref().context("session missing")?);
            let todos: Value = serde_json::from_slice(&fs::read(
                t.root.join("local-home/todos").join(&filename),
            )?)?;
            ensure!(
                todos
                    .as_array()
                    .context("todo list missing")?
                    .iter()
                    .any(|todo| todo["id"] == "acceptance-open" && todo["status"] == "in_progress"),
                "open todo missing"
            );
            t.prepare_while_held()?;
            let start = t.records()?.len();
            t.release("poke")?;
            t.await_cloud(start)?;
            ensure!(
                t.root.join("cloud-home/todos").join(filename).exists(),
                "todos were not transferred"
            );
        }
        "cancel_prepared" => {
            t.held_turn("cancel", false)?;
            t.prepare_while_held()?;
            t.send("/cloud cancel", true)?;
            t.wait_text("Pending cloud handoff canceled.")?;
            t.release("cancel")?;
            t.wait_text("[model] ack")?;
            t.send("still working locally after cancel", true)?;
            t.wait_request("still working locally after cancel", 0)?;
            local(t)?;
        }
        "draft_preserved" => {
            t.held_turn("draft", false)?;
            t.prepare_while_held()?;
            t.send("my unsent acceptance draft", false)?;
            t.release("draft")?;
            t.wait_text("composer draft would be lost")?;
            ensure!(
                t.lease()["away_host"].as_str().is_none_or(str::is_empty),
                "draft changed ownership"
            );
            t.tmux(&["send-keys", "-t", "acceptance", "Enter"])?;
            t.wait_request("my unsent acceptance draft", 0)?;
            local(t)?;
        }
        "prepare_outage_retry" => {
            t.send("before SSH outage", true)?;
            t.wait_text("[model] ack")?;
            t.discover_session()?;
            fs::write(t.control.join("ssh-fail"), "")?;
            t.send("/cloud", true)?;
            t.wait_text("Injected acceptance SSH outage")?;
            ensure!(
                t.lease()["away_host"].as_str().is_none_or(str::is_empty),
                "outage changed ownership"
            );
            fs::remove_file(t.control.join("ssh-fail"))?;
            t.go_cloud(false)?;
        }
        "transfer_outage_retry" => {
            t.held_turn("outage", false)?;
            t.prepare_while_held()?;
            fs::write(t.control.join("ssh-fail"), "")?;
            t.release("outage")?;
            t.wait_text("Injected acceptance SSH outage")?;
            local(t)?;
            fs::remove_file(t.control.join("ssh-fail"))?;
            t.go_cloud(false)?;
        }
        "destination_binary_retry" => {
            t.send("check destination compatibility", true)?;
            t.wait_text("[model] ack")?;
            t.discover_session()?;
            t.wait_idle()?;
            for (fault, expected) in [
                ("binary-missing", "could not start"),
                ("binary-legacy", "too old for cloud handoff"),
            ] {
                let marker = t.control.join(fault);
                fs::write(&marker, "")?;
                t.send("/cloud", true)?;
                t.wait_text(expected)?;
                local(t)?;
                fs::remove_file(marker)?;
            }
            t.go_cloud(false)?;
        }
        _ => anyhow::bail!("unknown TUI scenario {name}"),
    }
    Ok(())
}
