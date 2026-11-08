use super::common::ChildGuard;
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    process::{ChildStdin, Command},
    sync::mpsc,
    time::{Duration, Instant},
};
pub(super) struct StdioSession {
    pub child: ChildGuard,
    input: ChildStdin,
    lines: mpsc::Receiver<Result<Value>>,
    kind: String,
    next: u64,
    thread: String,
    pub tool_uses: usize,
    pub last: String,
}
impl StdioSession {
    pub fn new(c: &mut Command, kind: &str, cwd: &str, model: &str) -> Result<Self> {
        let mut child = ChildGuard::piped(c)?;
        let input = child.child.stdin.take().context("stdin")?;
        let out = child.child.stdout.take().context("stdout")?;
        let (tx, rx) = mpsc::sync_channel(256);
        std::thread::spawn(move || {
            let mut reader = BufReader::new(out);
            loop {
                let mut bytes = vec![];
                let result = (|| -> Result<Value> {
                    loop {
                        let b = reader.fill_buf()?;
                        ensure!(!b.is_empty(), "agent stream closed");
                        let n = b
                            .iter()
                            .position(|b| *b == b'\n')
                            .map(|i| i + 1)
                            .unwrap_or(b.len());
                        ensure!(
                            bytes.len() + n <= 8 * 1024 * 1024,
                            "agent frame exceeds 8 MiB"
                        );
                        bytes.extend_from_slice(&b[..n]);
                        reader.consume(n);
                        if bytes.last() == Some(&b'\n') {
                            break;
                        }
                    }
                    Ok(serde_json::from_slice(&bytes)?)
                })();
                let terminal = result.is_err() && bytes.is_empty();
                if tx.send(result).is_err() || terminal {
                    break;
                }
            }
        });
        let mut s = Self {
            child,
            input,
            lines: rx,
            kind: kind.into(),
            next: 1,
            thread: String::new(),
            tool_uses: 0,
            last: String::new(),
        };
        if kind == "codex" {
            s.request(
                "initialize",
                json!({"clientInfo":{"name":"nexode-bench","version":"1"}}),
            )?;
            s.send(json!({"jsonrpc":"2.0","method":"initialized"}))?;
            let v=s.request("thread/start",json!({"cwd":cwd,"model":if model.is_empty(){Value::Null}else{json!(model)},"approvalPolicy":"never","sandbox":"read-only"}))?;
            s.thread = v["thread"]["id"]
                .as_str()
                .context("Codex thread id")?
                .into();
        }
        Ok(s)
    }
    fn send(&mut self, v: Value) -> Result<()> {
        writeln!(self.input, "{v}")?;
        self.input.flush()?;
        Ok(())
    }
    fn receive(&self, end: Instant) -> Result<Value> {
        let remaining = end.saturating_duration_since(Instant::now());
        ensure!(!remaining.is_zero(), "agent turn timed out");
        self.lines
            .recv_timeout(remaining)
            .context("agent turn timed out or stream disconnected")?
    }
    fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next;
        self.next += 1;
        self.send(json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))?;
        let end = Instant::now() + Duration::from_secs(120);
        loop {
            let v = self.receive(end)?;
            if v["id"] == id && v.get("method").is_none() {
                ensure!(v.get("error").is_none(), "Codex request rejected");
                return Ok(v["result"].clone());
            }
        }
    }
    pub fn turn(&mut self, prompt: &str, timeout: Duration) -> Result<()> {
        let id = self.next;
        self.next += 1;
        let req = match self.kind.as_str() {
            "claude" => {
                json!({"type":"user","message":{"role":"user","content":[{"type":"text","text":prompt}]}})
            }
            "pi" => json!({"type":"prompt","message":prompt}),
            _ => {
                json!({"jsonrpc":"2.0","id":id,"method":"turn/start","params":{"threadId":self.thread,"input":[{"type":"text","text":prompt}]}})
            }
        };
        self.send(req)?;
        let end = Instant::now() + timeout;
        loop {
            let v = self.receive(end)?;
            match self.kind.as_str() {
                "claude" => {
                    if v["type"] == "assistant" {
                        self.tool_uses += v["message"]["content"]
                            .as_array()
                            .map(|a| a.iter().filter(|v| v["type"] == "tool_use").count())
                            .unwrap_or(0);
                    }
                    if v["type"] == "result" {
                        self.last = v["result"]
                            .as_str()
                            .unwrap_or("")
                            .chars()
                            .take(200)
                            .collect();
                        ensure!(v["is_error"] != true, "Claude turn failed: {}", self.last);
                        return Ok(());
                    }
                }
                "pi" => {
                    ensure!(
                        v["type"] != "response" || v["success"] != false,
                        "Pi rejected prompt"
                    );
                    if v["type"] == "tool_execution_start" {
                        self.tool_uses += 1
                    }
                    if v["type"] == "agent_end" {
                        let last = v["messages"]
                            .as_array()
                            .and_then(|a| a.last())
                            .cloned()
                            .unwrap_or(Value::Null);
                        self.last = last.to_string().chars().take(200).collect();
                        ensure!(
                            last["stopReason"] != "error"
                                && last.get("errorMessage").is_none_or(Value::is_null),
                            "Pi turn failed"
                        );
                        return Ok(());
                    }
                }
                _ => {
                    if v.get("id").is_some() && v.get("method").is_some() {
                        self.send(
                            json!({"jsonrpc":"2.0","id":v["id"],"result":{"decision":"decline"}}),
                        )?;
                        continue;
                    }
                    ensure!(
                        v["id"] != id || v.get("error").is_none(),
                        "Codex rejected turn"
                    );
                    if v["method"] == "item/completed" {
                        let item = &v["params"]["item"];
                        if matches!(
                            item["type"].as_str(),
                            Some("commandExecution" | "mcpToolCall" | "fileChange" | "webSearch")
                        ) {
                            self.tool_uses += 1
                        }
                        if item["type"] == "agentMessage" {
                            self.last = item["text"]
                                .as_str()
                                .unwrap_or("")
                                .chars()
                                .take(200)
                                .collect();
                        }
                    }
                    if v["method"] == "turn/completed" {
                        ensure!(
                            v["params"]["turn"]["status"]
                                .as_str()
                                .is_none_or(|s| s == "completed"),
                            "Codex turn failed"
                        );
                        return Ok(());
                    }
                }
            }
        }
    }
}
