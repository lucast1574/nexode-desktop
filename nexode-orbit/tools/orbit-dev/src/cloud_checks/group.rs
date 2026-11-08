use super::*;
use std::path::Path;
const MEMBERS: [&str; 3] = ["group-root", "group-worker", "independent"];
fn json_file(path: impl AsRef<Path>) -> Result<Value> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}
fn store(path: impl AsRef<Path>, value: Value) -> Result<()> {
    fs::write(path, serde_json::to_vec(&value)?)?;
    Ok(())
}
pub(super) fn run(command: &str, args: &[String]) -> Result<()> {
    match command {
        "seed-group" => {
            ensure!(args.len() == 3, "seed-group HOME REPO RUNTIME");
            let home = Path::new(&args[0]);
            let repo = &args[1];
            let runtime = Path::new(&args[2]);
            let now = chrono::Utc::now().to_rfc3339();
            for sid in ["group-root", "group-worker", "independent", "excluded"] {
                store(
                    home.join("sessions").join(format!("{sid}.json")),
                    json!({"id":sid,"title":sid,"created_at":now,"updated_at":now,"working_dir":repo,"status":"Active","messages":[{"id":format!("message-{sid}"),"role":"user","timestamp":now,"content":[{"type":"text","text":format!("Fixture {sid}")}]}]}),
                )?;
                store(
                    home.join("todos").join(format!("{sid}.json")),
                    json!({"items":[],"goal":sid}),
                )?;
            }
            let member = |sid: &str, role: &str, owner: Option<&str>| json!({"session_id":sid,"working_dir":repo,"swarm_id":"fixture-swarm","swarm_enabled":true,"status":"ready","detail":null,"friendly_name":sid,"report_back_to_session_id":owner,"role":role,"is_headless":false});
            store(
                runtime.join("durable-state/swarm/fixture-swarm.json"),
                json!({"swarm_id":"fixture-swarm","coordinator_session_id":"group-root","updated_at_unix_ms":chrono::Utc::now().timestamp_millis(),"plan":{"items":[{"id":"work","content":"Continue fixture","status":"pending","priority":"normal","assigned_to":"group-worker"}],"version":7,"participants":["group-root","group-worker"]},"members":[member("group-root","coordinator",None),member("group-worker","agent",Some("group-root"))]}),
            )?;
        }
        "assert-group-away" => {
            ensure!(args.len() == 1, "assert-group-away HOME");
            let home = Path::new(&args[0]);
            for sid in MEMBERS {
                ensure!(
                    json_file(home.join("session_leases").join(format!("{sid}.json")))?["away_host"]
                        == "fixture-cloud",
                    "group member not away: {sid}"
                );
            }
            ensure!(
                !home.join("session_leases/excluded.json").exists(),
                "excluded session moved"
            );
        }
        "assert-group-recovered" => {
            ensure!(args.len() == 3, "assert-group-recovered LOCAL CLOUD STATE");
            let local = Path::new(&args[0]);
            let cloud = Path::new(&args[1]);
            let state = Path::new(&args[2]);
            for sid in MEMBERS {
                let filename = format!("{sid}.json");
                let original = json_file(local.join("sessions").join(&filename))?;
                let moved = json_file(cloud.join("sessions").join(&filename))?;
                ensure!(
                    moved["id"] == sid && moved["migration_epoch"] == 1,
                    "incorrect recovered session {sid}"
                );
                let mut a = original["messages"][0].clone();
                let mut b = moved["messages"][0].clone();
                let ta = a
                    .as_object_mut()
                    .context("original message missing")?
                    .remove("timestamp")
                    .context("timestamp missing")?;
                let tb = b
                    .as_object_mut()
                    .context("moved message missing")?
                    .remove("timestamp")
                    .context("timestamp missing")?;
                ensure!(
                    chrono::DateTime::parse_from_rfc3339(
                        ta.as_str().context("timestamp invalid")?
                    )? == chrono::DateTime::parse_from_rfc3339(
                        tb.as_str().context("timestamp invalid")?
                    )? && a == b,
                    "original message changed: {sid}"
                );
                let text = content(&moved["messages"]).join("\n");
                let expected = if sid == "group-root" {
                    "setup once for the workspace"
                } else {
                    "do not independently repeat workspace setup"
                };
                ensure!(
                    text.contains(expected),
                    "missing group setup instruction: {sid}"
                );
                ensure!(
                    fs::read(cloud.join("todos").join(&filename))?
                        == fs::read(local.join("todos").join(&filename))?,
                    "todos changed: {sid}"
                );
                ensure!(
                    json_file(cloud.join("session_leases").join(&filename))?["away_host"]
                        .as_str()
                        .is_none_or(str::is_empty),
                    "cloud does not own member: {sid}"
                );
            }
            ensure!(
                !cloud.join("sessions/excluded.json").exists(),
                "excluded session transferred"
            );
            let swarm = json_file(state.join("fixture-swarm.json"))?;
            let members: std::collections::BTreeSet<_> = swarm["members"]
                .as_array()
                .context("members missing")?
                .iter()
                .filter_map(|m| m["session_id"].as_str())
                .collect();
            ensure!(
                swarm["coordinator_session_id"] == "group-root"
                    && members == std::collections::BTreeSet::from(["group-root", "group-worker"]),
                "swarm membership changed"
            );
            ensure!(
                swarm["plan"]["version"] == 7
                    && swarm["plan"]["items"][0]["assigned_to"] == "group-worker",
                "swarm plan changed"
            );
        }
        "assert-group-environment" => {
            ensure!(
                args.len() == 2,
                "assert-group-environment DIRECTORY TRANSFERS"
            );
            let mut files = vec![];
            for entry in fs::read_dir(&args[0])? {
                let path = entry?.path();
                if path.extension().is_some_and(|e| e == "json") {
                    files.push(path);
                }
            }
            ensure!(files.len() == 1, "expected one environment transfer");
            ensure!(
                json_file(&files[0])?["GROUP_RESOURCE_TOKEN"] == "synthetic-group-resource",
                "environment token missing"
            );
            ensure!(
                fs::read_to_string(&args[1])?.lines().collect::<Vec<_>>() == ["receive"],
                "setup must replay once for the whole group"
            );
        }
        _ => unreachable!(),
    }
    Ok(())
}
