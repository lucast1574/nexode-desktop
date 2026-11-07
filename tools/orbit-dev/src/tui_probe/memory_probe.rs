//! Memory tool contract and explicitly requested live extraction checks.
use super::debug_fixture::{Fixture, OwnedSession};
use anyhow::{Context, Result, ensure};
use clap::Parser;
use regex::Regex;
use serde_json::{Value, json};
use std::path::PathBuf;
#[derive(Parser)]
struct Args {
    #[arg(long)]
    fresh: bool,
    #[arg(long, default_value = "./target/release/nexode")]
    binary: PathBuf,
    #[arg(long, required_unless_present = "fresh", conflicts_with = "fresh")]
    socket: Option<PathBuf>,
    #[arg(long,value_parser=["claude","openai"])]
    provider: Option<String>,
    /// Send provider requests and exercise extraction on the explicitly supplied daemon.
    #[arg(long, conflicts_with = "fresh")]
    live: bool,
}
fn tool(session: &OwnedSession, input: Value) -> Result<String> {
    let s = session
        .command(&format!("tool:memory {input}"))
        .with_context(|| format!("memory action {}", input["action"]))?;
    if let Ok(v) = serde_json::from_str::<Value>(&s) {
        ensure!(
            v["is_error"] != true && v["error"].is_null(),
            "memory tool failed"
        );
        Ok(v["output"].as_str().unwrap_or(&s).into())
    } else {
        Ok(s)
    }
}
fn remember(session: &OwnedSession, content: &str) -> Result<String> {
    let output = tool(
        session,
        json!({"action":"remember","content":content,"tags":["test","provider-pref"],"scope":"project"}),
    )?;
    ensure!(output.contains("Remembered"), "remember did not succeed");
    Ok(Regex::new(r"id: ([^\]\s]+)")?
        .captures(&output)
        .context("remember returned no memory id")?[1]
        .into())
}
struct Memories<'a> {
    session: &'a OwnedSession,
    ids: Vec<String>,
}
impl Drop for Memories<'_> {
    fn drop(&mut self) {
        for id in &self.ids {
            if let Err(e) = tool(
                self.session,
                json!({"action":"forget","id":id,"scope":"project"}),
            ) {
                eprintln!("test memory cleanup failed: {e:#}");
            }
        }
    }
}
fn checks(session: &OwnedSession, provider: &str, live: bool) -> Result<Value> {
    session
        .command(&format!("set_provider:{provider}"))
        .with_context(|| format!("select {provider}"))?;
    let content = format!("User prefers {provider} for testing");
    let mut memories = Memories {
        session,
        ids: vec![],
    };
    memories.ids.push(remember(session, &content)?);
    let id = memories.ids[0].clone();
    let list = tool(session, json!({"action":"list","scope":"project"}))?;
    ensure!(list.contains(&content), "list missing remembered content");
    let search = tool(
        session,
        json!({"action":"search","query":format!("{provider} testing"),"scope":"project"}),
    )?;
    ensure!(
        search.contains(&content),
        "search missed remembered content"
    );
    let semantic = if live {
        let recall = tool(
            session,
            json!({"action":"recall","query":"testing preferences","mode":"cascade","scope":"project"}),
        )?;
        ensure!(
            recall.contains(&content),
            "semantic recall missed remembered content"
        );
        json!({"status":"passed"})
    } else {
        json!({"status":"skipped","reason":"Jev semantic retrieval requires --live with --socket"})
    };
    let recent = tool(
        session,
        json!({"action":"recall","limit":5,"scope":"project"}),
    )?;
    ensure!(recent.contains(&content), "recent recall missed content");
    ensure!(tool(session,json!({"action":"tag","id":id,"tags":[format!("extra-tag-{provider}")],"scope":"project"}))?.contains("Tagged"),"tag failed");
    memories.ids.push(remember(
        session,
        &format!("Second memory for {provider} link test"),
    )?);
    ensure!(tool(session,json!({"action":"link","from_id":id,"to_id":memories.ids[1],"weight":0.75,"scope":"project"}))?.contains("Linked"),"link failed");
    let related = tool(
        session,
        json!({"action":"related","id":id,"depth":2,"scope":"project"}),
    )?;
    ensure!(
        related.contains("Second memory"),
        "graph traversal missed linked memory"
    );
    let extraction = if live {
        for msg in [
            format!("Hello, I'm testing with {provider}"),
            "Remember that my favorite editor is vim".into(),
            "What text editors do you know about?".into(),
            "Thanks for the information!".into(),
        ] {
            session.command(&format!("message:{msg}"))?;
        }
        let extracted: Value = serde_json::from_str(&session.command("trigger_extraction")?)?;
        ensure!(
            extracted["message_count"].as_u64().is_some_and(|n| n >= 4),
            "extraction lacked conversation messages"
        );
        json!({"status":"passed","result":extracted})
    } else {
        json!({"status":"skipped","reason":"live provider exchange and extraction require --live with --socket"})
    };
    Ok(
        json!({"provider":provider,"memory_tool_checks":"passed","semantic_recall":semantic,"extraction":extraction}),
    )
}
pub(super) fn run(args: impl Iterator<Item = String>) -> Result<i32> {
    let a = Args::try_parse_from(std::iter::once("test-memory".into()).chain(args))?;
    let fixture = if a.fresh {
        Some(Fixture::new(&a.binary.canonicalize()?)?)
    } else {
        None
    };
    let temp = tempfile::tempdir()?;
    let socket = fixture
        .as_ref()
        .map(|f| &f.socket)
        .or(a.socket.as_ref())
        .context("socket required")?;
    let cwd = fixture
        .as_ref()
        .map(|f| f.workspace.as_path())
        .unwrap_or(temp.path());
    let providers = a
        .provider
        .map(|p| vec![p])
        .unwrap_or_else(|| vec!["claude".into(), "openai".into()]);
    let mut results = vec![];
    for provider in providers {
        let session = OwnedSession::new(socket, cwd)?;
        results.push(checks(&session, &provider, a.live)?);
    }
    println!("{}", serde_json::to_string_pretty(&results)?);
    Ok(0)
}
