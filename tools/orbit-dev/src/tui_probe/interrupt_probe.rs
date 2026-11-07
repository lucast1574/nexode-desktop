//! Deterministic interrupts against real turns and tools, plus opt-in live providers.
use super::{
    debug_fixture::{Fixture, OwnedSession},
    io,
};
use anyhow::{Context, Result, ensure};
use clap::Parser;
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};
#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "./target/release/nexode")]
    binary: PathBuf,
    #[arg(long)]
    live: bool,
    #[arg(long, requires = "live")]
    socket: Option<PathBuf>,
    #[arg(long, requires = "live")]
    native_socket: Option<PathBuf>,
    #[arg(long, default_value = "claude,openai", value_delimiter = ',')]
    providers: Vec<String>,
    #[arg(long)]
    keep: bool,
}
/// Validate real IDs in both internal/Anthropic blocks and OpenAI wire messages.
fn pairing(messages: &[Value]) -> Result<usize> {
    let mut pending = BTreeSet::new();
    let mut count = 0;
    for m in messages {
        let role = m["role"].as_str().unwrap_or("");
        if role == "tool" {
            let id = m["tool_call_id"].as_str().context("missing result id")?;
            ensure!(pending.remove(id), "unexpected tool result {id}");
            continue;
        }
        if let Some(calls) = m["tool_calls"].as_array() {
            for call in calls {
                let id = call["id"].as_str().context("missing call id")?;
                ensure!(pending.insert(id.to_owned()), "duplicate call id");
                count += 1;
            }
        }
        if let Some(blocks) = m["content"].as_array() {
            for block in blocks {
                match block["type"].as_str() {
                    Some("tool_use") => {
                        let id = block["id"].as_str().context("missing tool id")?;
                        ensure!(pending.insert(id.to_owned()), "duplicate tool id");
                        count += 1;
                    }
                    Some("tool_result") => {
                        let id = block["tool_use_id"].as_str().context("missing result id")?;
                        ensure!(pending.remove(id), "unexpected result {id}");
                    }
                    Some("text") if role == "user" => ensure!(
                        pending.is_empty(),
                        "user text injected before all tool results"
                    ),
                    _ => {}
                }
            }
        } else if role == "user" && m["content"].as_str().is_some_and(|s| !s.is_empty()) {
            ensure!(pending.is_empty(), "user text injected before tool results");
        }
    }
    ensure!(pending.is_empty(), "missing tool results: {pending:?}");
    Ok(count)
}
fn wait_file(p: &Path) -> Result<()> {
    let start = Instant::now();
    while !p.exists() {
        ensure!(
            start.elapsed() < Duration::from_secs(30),
            "gate not reached: {}",
            p.display()
        );
        thread::sleep(Duration::from_millis(20));
    }
    Ok(())
}
fn history(s: &OwnedSession) -> Result<Vec<Value>> {
    Ok(serde_json::from_str(&s.command("history")?)?)
}
fn streaming_message(socket: &Path, id: &str, prompt: &str) -> Result<String> {
    let mut writer = UnixStream::connect(socket)?;
    writer.set_read_timeout(Some(Duration::from_secs(90)))?;
    writer.set_write_timeout(Some(Duration::from_secs(5)))?;
    let mut reader = BufReader::new(writer.try_clone()?);
    writeln!(
        writer,
        "{}",
        json!({"type":"subscribe","id":1,"target_session_id":id,"continue_on_disconnect":true,"allow_session_takeover":true})
    )?;
    let deadline = Instant::now() + Duration::from_secs(90);
    let mut sent = false;
    let mut text = String::new();
    loop {
        ensure!(Instant::now() < deadline, "stream deadline exceeded");
        reader.get_ref().set_read_timeout(Some(
            deadline
                .saturating_duration_since(Instant::now())
                .max(Duration::from_millis(1)),
        ))?;
        let mut line = String::new();
        ensure!(
            reader.read_line(&mut line)? > 0,
            "stream closed before done"
        );
        ensure!(line.len() < 8 * 1024 * 1024, "oversized event");
        let event: Value = serde_json::from_str(&line)?;
        ensure!(event["type"] != "error", "stream error: {event}");
        if event["type"] == "done" && event["id"] == 1 && !sent {
            writeln!(
                writer,
                "{}",
                json!({"type":"message","id":2,"content":prompt})
            )?;
            sent = true;
        } else if event["type"] == "done" && event["id"] == 2 {
            return Ok(text);
        }
        if let Some(chunk) = event["text"].as_str() {
            text.push_str(chunk);
        }
    }
}
fn assert_interrupt(history: &[Value], marker: &str, after_assistant: bool) -> Result<()> {
    let index = history
        .iter()
        .position(|m| m["role"] == "user" && m.to_string().contains(marker))
        .context("interrupt absent from history")?;
    if after_assistant {
        ensure!(
            history[..index].iter().any(|m| m["role"] == "assistant"),
            "interrupt preceded first assistant"
        );
    }
    ensure!(
        history[index + 1..]
            .iter()
            .any(|m| m["role"] == "assistant"),
        "no response after interrupt"
    );
    Ok(())
}
fn synthetic(a: &Args) -> Result<Value> {
    let root = tempfile::Builder::new()
        .prefix("orbit-interrupt-")
        .tempdir()?;
    let log = root.path().join("requests.jsonl");
    let provider = crate::fake_openai::Server::start(
        0,
        crate::fake_openai::Options {
            log: log.clone(),
            label: "interrupt".into(),
            control: Some(root.path().into()),
        },
    )?;
    let fixture = Fixture::with_provider(&a.binary.canonicalize()?, Some(provider.port))?;
    let mut reports = vec![];
    for case in [
        "basic",
        "stream",
        "read-tool",
        "multiple-tools",
        "urgent-tools",
        "long-response",
        "message-order",
        "prequeued",
    ] {
        let mut s = OwnedSession::new(&fixture.socket, &fixture.workspace)?;
        s.command("set_model:openai-compatible:fake-model")
            .context("route synthetic model")?;
        let marker = format!("interrupt-marker-{case}");
        let tools = matches!(case, "read-tool" | "multiple-tools" | "urgent-tools");
        let urgent = case == "urgent-tools";
        let gate = root.path().join(format!("{case}.held"));
        let release = root.path().join(format!("{case}.release"));
        if tools {
            let count = if case == "read-tool" { 1 } else { 3 };
            let fruit = fixture.workspace.join("fruit.txt");
            fs::write(&fruit, "apple\nbanana\ncherry\ndate\nelderberry\n")?;
            let mut calls = vec![];
            for i in 0..count {
                let command = if i == 0 {
                    format!(
                        "touch {}; for n in $(seq 1 1500); do [ -f {} ] && break; sleep .02; done; test -f {} && cat {}",
                        shlex::try_quote(gate.to_str().unwrap())?,
                        shlex::try_quote(release.to_str().unwrap())?,
                        shlex::try_quote(release.to_str().unwrap())?,
                        shlex::try_quote(fruit.to_str().unwrap())?
                    )
                } else {
                    format!(
                        "printf executed > {}",
                        shlex::try_quote(
                            root.path()
                                .join(format!("{case}-{i}.executed"))
                                .to_str()
                                .unwrap()
                        )?
                    )
                };
                calls.push(json!({"id":format!("call_{case}_{i}"),"type":"function","function":{"name":"bash","arguments":json!({"command":command,"intent":"Read synthetic fruit and verify interrupt ordering","timeout":40}).to_string()}}));
            }
            fs::write(
                root.path().join(format!("{case}.tools.json")),
                serde_json::to_vec(&calls)?,
            )?;
        }
        let prompt = if tools {
            format!("Read the fruit fixture [e2e:tools:{case}]")
        } else if case == "basic" || case == "prequeued" {
            "What is 2+2?".into()
        } else {
            format!("Write a response [e2e:hold:{case}]")
        };
        if case == "prequeued" {
            s.command(&format!("queue_interrupt:{marker}"))?;
        }
        let socket = fixture.socket.with_file_name("nexode.sock");
        let id = s.id.clone();
        let job = thread::spawn(move || streaming_message(&socket, &id, &prompt));
        let interrupt_result = (|| -> Result<()> {
            if case != "basic" && case != "prequeued" {
                wait_file(&gate)?;
                let output = io::debug_session(
                    &s.socket,
                    &format!(
                        "{}:{marker}",
                        if urgent {
                            "queue_interrupt_urgent"
                        } else {
                            "queue_interrupt"
                        }
                    ),
                    Some(&s.id),
                    Duration::from_secs(5),
                )?;
                ensure!(output.contains("queued"), "interrupt not acknowledged");
            }
            Ok(())
        })();
        fs::write(&release, "")?;
        let response = job
            .join()
            .map_err(|_| anyhow::anyhow!("message worker panicked"))?;
        interrupt_result.context(format!("queue in {case}"))?;
        response.context(format!("message in {case}"))?;
        let h = history(&s)?;
        if case == "basic" {
            ensure!(
                h.iter()
                    .map(|m| m["role"].as_str().unwrap_or(""))
                    .collect::<Vec<_>>()
                    == ["user", "assistant"],
                "unexpected basic history"
            );
        } else {
            assert_interrupt(&h, &marker, !tools && case != "prequeued")
                .with_context(|| format!("case {case}: {h:?}"))?;
        }
        // Inspect every submitted provider request, including the request after
        // injection, instead of guessing tool IDs from display strings.
        let mut max_calls = 0;
        for line in fs::read_to_string(&log)?.lines() {
            let r: Value = serde_json::from_str(line)?;
            max_calls = max_calls.max(pairing(
                r["messages"].as_array().context("request messages")?,
            )?);
        }
        if tools {
            ensure!(
                max_calls >= if case == "read-tool" { 1 } else { 3 },
                "expected tool calls were not observed"
            );
        }
        if urgent {
            ensure!(
                h.iter().any(|m| m.to_string().contains("Skipped")),
                "no skipped tool results"
            );
            ensure!(
                !root.path().join(format!("{case}-1.executed")).exists(),
                "urgent interrupt did not skip remaining tools"
            );
        }
        reports.push(
            json!({"case":case,"passed":true,"history_messages":h.len(),"tool_calls":max_calls}),
        );
        s.close()?;
        fs::write(&log, "")?;
    }
    let artifacts = if a.keep { Some(root.keep()) } else { None };
    Ok(
        json!({"mode":"loopback provider; real runtime and tools","cases":reports,"artifacts":artifacts}),
    )
}
fn live(a: &Args) -> Result<Value> {
    let socket = a.socket.as_ref().context("--live requires --socket")?;
    let workspace = tempfile::tempdir()?;
    let mut reports = vec![];
    for provider in &a.providers {
        for (name, prompt, urgent, delay) in [
            ("basic", "What is 2+2? Just the number.", false, 0),
            (
                "stream",
                "Count from 1 to 10, one number per line.",
                false,
                1000,
            ),
            (
                "tools",
                "Run three bash commands in sequence: echo one; echo two; echo three.",
                false,
                300,
            ),
            (
                "urgent",
                "Run three bash commands in sequence: sleep 2; echo two; echo three.",
                true,
                300,
            ),
            (
                "long",
                "Write a detailed five paragraph essay about computing history.",
                false,
                2000,
            ),
            ("order", "Write a haiku about coding.", false, 500),
        ] {
            let mut s = OwnedSession::new(socket, workspace.path())?;
            s.command(&format!("set_provider:{provider}"))?;
            let marker = format!("INTERRUPT_{name}: stop and just say OK");
            let socket = a
                .native_socket
                .clone()
                .unwrap_or_else(|| s.socket.with_file_name("nexode.sock"));
            let id = s.id.clone();
            let job = thread::spawn(move || streaming_message(&socket, &id, prompt));
            let queued = if delay > 0 {
                thread::sleep(Duration::from_millis(delay));
                s.command(&format!(
                    "{}:{marker}",
                    if urgent {
                        "queue_interrupt_urgent"
                    } else {
                        "queue_interrupt"
                    }
                ))
            } else {
                Ok(String::new())
            };
            job.join()
                .map_err(|_| anyhow::anyhow!("worker panicked"))??;
            queued?;
            let h = history(&s)?;
            if delay > 0 {
                assert_interrupt(&h, &marker, true)?;
            }
            reports.push(
                json!({"provider":provider,"case":name,"passed":true,"history_messages":h.len()}),
            );
            s.close()?;
        }
    }
    Ok(json!({"mode":"live external providers","cases":reports}))
}
pub(super) fn run(args: impl Iterator<Item = String>) -> Result<i32> {
    let a = Args::try_parse_from(std::iter::once("test-soft-interrupt".into()).chain(args))?;
    let result = if a.live { live(&a) } else { synthetic(&a) }?;
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(0)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_wrong_missing_and_interleaved_tool_results() {
        let call = json!({"role":"assistant","tool_calls":[{"id":"a"},{"id":"b"}]});
        let a = json!({"role":"tool","tool_call_id":"a","content":"ok"});
        let b = json!({"role":"tool","tool_call_id":"b","content":"Skipped"});
        let user = json!({"role":"user","content":"interrupt"});
        assert_eq!(
            pairing(&[call.clone(), a.clone(), b.clone(), user.clone()]).unwrap(),
            2
        );
        assert!(pairing(&[call.clone(), a.clone(), user, b.clone()]).is_err());
        assert!(pairing(&[call.clone(), a.clone()]).is_err());
        assert!(pairing(&[call, a.clone(), a, b]).is_err());
    }
    #[test]
    fn requires_actual_interrupt_and_subsequent_answer() {
        assert!(assert_interrupt(&[json!({"role":"assistant"})], "missing", true).is_err());
        assert!(
            assert_interrupt(&[json!({"role":"user","content":"marker"})], "marker", true).is_err()
        );
    }
}
