//! Validation helpers for the existing live OpenRelay shell runner.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{fs, io::Read};
fn events(text: &str) -> Vec<Value> {
    text.lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}
fn trace(agent: &str, server: &str) -> Value {
    let agent = events(agent);
    let server = events(server);
    let browse = server.iter().any(|v| {
        v["event"] == "discovery_request"
            && v["phase"] == "browse"
            && v["category"] == "cloud-infrastructure"
    });
    let select = server.iter().any(|v| {
        v["event"] == "discovery_request"
            && v["phase"] == "select"
            && v["tool"] == "openrelay-rivet"
    });
    let done = agent
        .iter()
        .any(|v| v["type"] == "tool_done" && v["name"] == "bash");
    let mut active = "";
    let mut input = String::new();
    for v in &agent {
        match v["type"].as_str() {
            Some("tool_start") => active = v["name"].as_str().unwrap_or(""),
            Some("tool_input") if active == "bash" => {
                input.push_str(v["delta"].as_str().unwrap_or(""));
                input.push('\n');
            }
            Some("tool_exec" | "tool_done") => active = "",
            _ => {}
        }
    }
    let input = input.to_lowercase();
    let all = serde_json::to_string(&agent)
        .expect("JSON values")
        .to_lowercase();
    let queried = ["etc.rivet.link", "eth_chainid", "eth_getblockbynumber"]
        .iter()
        .all(|t| input.contains(t));
    let answered = ["ethereum classic", "utc", "chain id"]
        .iter()
        .all(|t| all.contains(t));
    json!({"browse_openrelay_category":browse,"selected_openrelay":select,"bash_tool_completed":done,"bash_queried_openrelay_rpc":queried,"answer_mentions_requested_fields":answered})
}
pub fn run(mut args: impl Iterator<Item = String>) -> Result<()> {
    match args.next().as_deref() {
        Some("endpoint") => {
            let path = args.next().context("ready file required")?;
            ensure!(args.next().is_none(), "unexpected arguments");
            let v: Value = serde_json::from_slice(&fs::read(path)?)?;
            println!("{}", v["endpoint"].as_str().context("missing endpoint")?);
        }
        Some("rpc") => {
            ensure!(args.next().is_none(), "pipe RPC JSON on stdin");
            let mut text = String::new();
            std::io::stdin().take(65537).read_to_string(&mut text)?;
            ensure!(text.len() <= 65536, "RPC response too large");
            let v: Value = serde_json::from_str(&text)?;
            ensure!(
                v["result"] == "0x3d",
                "unexpected Ethereum Classic chain ID"
            );
        }
        Some("trace") => {
            let agent = args.next().context("agent trace required")?;
            let server = args.next().context("server trace required")?;
            ensure!(args.next().is_none(), "unexpected arguments");
            let summary = trace(
                &String::from_utf8_lossy(&fs::read(agent)?),
                &String::from_utf8_lossy(&fs::read(server)?),
            );
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({"openrelay_discovery_test":summary}))?
            );
            ensure!(
                summary
                    .as_object()
                    .context("summary")?
                    .values()
                    .all(|v| *v == true),
                "OpenRelay Discovery trace validation failed"
            );
        }
        _ => anyhow::bail!(
            "expected endpoint <ready-file>, rpc (JSON stdin), or trace <agent-log> <server-log>"
        ),
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn requires_actual_bash_input_and_completed_browse_select() {
        let server = "noise\n{\"event\":\"discovery_request\",\"phase\":\"browse\",\"category\":\"cloud-infrastructure\"}\n{\"event\":\"discovery_request\",\"phase\":\"select\",\"tool\":\"openrelay-rivet\"}";
        let agent = "{\"type\":\"tool_start\",\"name\":\"bash\"}\n{\"type\":\"tool_input\",\"delta\":\"etc.rivet.link eth_chainId eth_getBlockByNumber\"}\n{\"type\":\"tool_done\",\"name\":\"bash\"}\n{\"text\":\"Ethereum Classic UTC chain ID\"}";
        assert!(
            trace(agent, server)
                .as_object()
                .unwrap()
                .values()
                .all(|v| *v == true)
        );
        assert_eq!(
            trace(
                &agent.replace("\"name\":\"bash\"", "\"name\":\"read\""),
                server
            )["bash_queried_openrelay_rpc"],
            false
        );
    }
}
