use super::{common::*, protocols::StdioSession, *};
use crate::tui_probe::io::debug_session;
use std::{fs, net::TcpListener, path::Path};
const TURNS: [&str; 5] = [
    "Use a tool to list the files in the current directory, then reply with just the number of entries.",
    "Read the first 20 lines of README.md with a tool and reply with its top-level title only.",
    "Search the repository for the string 'fn main' with a tool and reply with how many files matched.",
    "In one sentence, summarize what you have learned about this repository so far.",
    "Reply with exactly the word: done",
];
#[derive(Parser)]
struct Headless {
    #[arg(long,num_args=1..,default_values=["1","5","10","20"])]
    sessions: Vec<usize>,
    #[arg(long, default_value_t = 5)]
    turns: usize,
    #[arg(long,num_args=1..,default_values=["nexode","claude"])]
    tools: Vec<String>,
    #[arg(long, default_value = "claude-sonnet-4-6")]
    nexode_model: String,
    #[arg(long, default_value = "claude-sonnet-4-6")]
    claude_model: String,
    #[arg(long, default_value = "gpt-5.4")]
    openai_model: String,
    #[arg(long)]
    openai_account: Option<String>,
    #[arg(long)]
    codex_bin: Option<PathBuf>,
    #[arg(long)]
    opencode_bin: Option<PathBuf>,
    #[arg(long)]
    pi_bin: Option<PathBuf>,
    #[arg(long)]
    nexode_bin: Option<PathBuf>,
    #[arg(long)]
    claude_bin: Option<PathBuf>,
    #[arg(long, default_value = ".")]
    cwd: PathBuf,
    #[arg(long)]
    json_out: Option<PathBuf>,
    #[arg(long, default_value_t = 600.)]
    timeout: f64,
    #[arg(long, default_value_t = 1.)]
    settle: f64,
}
pub fn run(args: impl Iterator<Item = String>) -> Result<i32> {
    let a = Headless::try_parse_from(std::iter::once("bench-headless-memory".into()).chain(args))?;
    ensure!(
        a.sessions.iter().all(|n| (1..=256).contains(n)) && a.turns <= 10000,
        "invalid session/turn count"
    );
    duration(a.timeout, false)?;
    duration(a.settle, true)?;
    for tool in &a.tools {
        ensure!(
            matches!(
                tool.as_str(),
                "nexode" | "nexode_memory_off" | "claude" | "codex" | "opencode" | "pi"
            ),
            "unknown tool {tool}"
        );
    }
    let root = tempfile::tempdir()?;
    let homes = if a
        .tools
        .iter()
        .any(|t| matches!(t.as_str(), "codex" | "opencode" | "pi"))
    {
        openai_homes(root.path(), a.openai_account.as_deref())?
    } else {
        BTreeMap::new()
    };
    let mut results = vec![];
    let mut failed = false;
    for n in &a.sessions {
        for tool in &a.tools {
            let result = run_tool(tool, *n, &a, homes.get(tool).cloned().unwrap_or_default());
            let v = match result {
                Ok(v) => {
                    failed |= v["errors"].as_array().is_none_or(|a| !a.is_empty());
                    v
                }
                Err(e) => {
                    failed = true;
                    json!({"tool":tool,"sessions":n,"errors":[e.to_string()]})
                }
            };
            println!("{v}");
            results.push(v);
            if let Some(p) = &a.json_out {
                fs::write(p, serde_json::to_vec_pretty(&results)?)?;
            }
        }
    }
    Ok(i32::from(failed))
}
enum Session {
    Stdio(StdioSession),
    Nexode {
        socket: PathBuf,
        id: String,
    },
    Http {
        base: String,
        id: String,
        model: String,
        client: reqwest::blocking::Client,
        last: String,
    },
}
impl Session {
    fn turn(&mut self, prompt: &str, timeout: Duration) -> Result<()> {
        match self {
            Self::Stdio(s) => s.turn(prompt, timeout),
            Self::Nexode { socket, id } => {
                debug_session(socket, &format!("message:{prompt}"), Some(id), timeout)?;
                Ok(())
            }
            Self::Http {
                base,
                id,
                model,
                client,
                last,
            } => {
                let mut body = json!({"parts":[{"type":"text","text":prompt}]});
                if !model.is_empty() {
                    body["model"] = json!({"providerID":"openai","modelID":model});
                }
                let v = http(
                    client,
                    "POST",
                    &format!("{base}/session/{id}/message"),
                    Some(body),
                )?;
                ensure!(
                    v["info"].get("error").is_none_or(Value::is_null),
                    "OpenCode turn failed"
                );
                if let Some(text) = v["parts"]
                    .as_array()
                    .and_then(|p| p.iter().rfind(|v| v["type"] == "text"))
                    .and_then(|v| v["text"].as_str())
                {
                    *last = text.chars().take(200).collect();
                }
                Ok(())
            }
        }
    }
    fn checks(&self) -> Result<Value> {
        match self {
            Self::Stdio(s) => Ok(json!({"tool_uses":s.tool_uses,"last_response":s.last})),
            Self::Nexode { socket, id } => {
                let history: Value = serde_json::from_str(&debug_session(
                    socket,
                    "history",
                    Some(id),
                    Duration::from_secs(30),
                )?)?;
                let tools = history
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .map(|v| v["tool_calls"].as_array().map(Vec::len).unwrap_or(0))
                            .sum::<usize>()
                    })
                    .unwrap_or(0);
                Ok(
                    json!({"history_messages":history.as_array().map(Vec::len),"tool_uses":tools,"last_response":debug_session(socket,"last_response",Some(id),Duration::from_secs(30))?.chars().take(200).collect::<String>()}),
                )
            }
            Self::Http {
                base,
                id,
                client,
                last,
                ..
            } => {
                let history = http(client, "GET", &format!("{base}/session/{id}/message"), None)?;
                let tools = history
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .map(|v| {
                                v["parts"]
                                    .as_array()
                                    .map(|a| a.iter().filter(|v| v["type"] == "tool").count())
                                    .unwrap_or(0)
                            })
                            .sum::<usize>()
                    })
                    .unwrap_or(0);
                Ok(json!({"tool_uses":tools,"last_response":last}))
            }
        }
    }
}
fn http(
    client: &reqwest::blocking::Client,
    method: &str,
    url: &str,
    body: Option<Value>,
) -> Result<Value> {
    let mut r = client.request(method.parse()?, url);
    if let Some(body) = body {
        r = r.json(&body)
    }
    let bytes = r.send()?.error_for_status()?.bytes()?;
    ensure!(
        bytes.len() <= 32 * 1024 * 1024,
        "OpenCode response too large"
    );
    if bytes.is_empty() {
        Ok(Value::Null)
    } else {
        Ok(serde_json::from_slice(&bytes)?)
    }
}
fn run_tool(
    tool: &str,
    n: usize,
    a: &Headless,
    mut env: BTreeMap<String, String>,
) -> Result<Value> {
    let root = tempfile::tempdir()?;
    let mut own = Ownership::new();
    env.insert("ORBIT_MEMORY_BENCH_OWNER".into(), own.marker.clone());
    if tool == "claude" {
        env.extend(claude_home(root.path())?);
    }
    let cwd = a.cwd.canonicalize()?;
    let timeout = duration(a.timeout, false)?;
    let settle = duration(a.settle, true)?;
    let mut server = None;
    let mut sessions = vec![];
    let mut embeddings = Value::Null;
    let binary = match tool {
        "nexode" | "nexode_memory_off" => binary("nexode", a.nexode_bin.as_deref())?,
        "claude" => binary("claude", a.claude_bin.as_deref())?,
        "codex" => binary("codex", a.codex_bin.as_deref())?,
        "opencode" => binary("opencode", a.opencode_bin.as_deref())?,
        _ => binary("pi", a.pi_bin.as_deref())?,
    };
    if tool.starts_with("nexode") {
        let memory = tool == "nexode";
        env.extend(nexode_home(root.path(), memory)?);
        let main = root.path().join("run/bench.sock");
        let debug = root.path().join("run/bench-debug.sock");
        let mut c = Command::new(&binary);
        c.args(["--no-update", "--no-selfdev", "serve", "--socket"])
            .arg(main)
            .arg("--debug-socket")
            .envs(&env)
            .current_dir(&cwd);
        server = Some(ChildGuard::spawn(&mut c)?);
        own.roots.push(server.as_ref().unwrap().pid());
        wait_socket(&debug, Duration::from_secs(30))?;
        for _ in 0..n {
            let out: Value = serde_json::from_str(&debug_session(
                &debug,
                &format!("create_session:{}", cwd.display()),
                None,
                Duration::from_secs(120),
            )?)?;
            let id = out["session_id"]
                .as_str()
                .context("missing session ID")?
                .to_owned();
            if !a.nexode_model.is_empty() {
                debug_session(
                    &debug,
                    &format!("set_model:{}", a.nexode_model),
                    Some(&id),
                    Duration::from_secs(60),
                )?;
            }
            sessions.push(Session::Nexode {
                socket: debug.clone(),
                id,
            });
        }
        embeddings = if memory {
            match debug_session(&debug, "embeddings:load", None, Duration::from_secs(120)) {
                Ok(s) => serde_json::from_str(&s).unwrap_or(json!(s)),
                Err(e) => json!({"error":e.to_string()}),
            }
        } else {
            json!("memory disabled")
        };
    } else if tool == "opencode" {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        drop(listener);
        let base = format!("http://127.0.0.1:{port}");
        let mut c = Command::new(&binary);
        c.args([
            "serve",
            "--port",
            &port.to_string(),
            "--hostname",
            "127.0.0.1",
        ])
        .envs(&env)
        .current_dir(&cwd);
        server = Some(ChildGuard::spawn(&mut c)?);
        own.roots.push(server.as_ref().unwrap().pid());
        let client = reqwest::blocking::Client::builder()
            .timeout(timeout)
            .build()?;
        let startup = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()?;
        let end = Instant::now() + Duration::from_secs(60);
        loop {
            if http(&startup, "GET", &format!("{base}/session"), None).is_ok() {
                break;
            }
            ensure!(Instant::now() < end, "OpenCode server not ready");
            std::thread::sleep(Duration::from_millis(100));
        }
        for _ in 0..n {
            let v = http(&client, "POST", &format!("{base}/session"), Some(json!({})))?;
            sessions.push(Session::Http {
                base: base.clone(),
                id: v["id"].as_str().context("OpenCode session ID")?.into(),
                model: a.openai_model.clone(),
                client: client.clone(),
                last: String::new(),
            });
        }
    } else {
        for _ in 0..n {
            let mut c = Command::new(&binary);
            c.envs(&env).current_dir(&cwd);
            let model = if tool == "claude" {
                &a.claude_model
            } else {
                &a.openai_model
            };
            match tool {
                "claude" => {
                    c.args([
                        "-p",
                        "--input-format",
                        "stream-json",
                        "--output-format",
                        "stream-json",
                        "--verbose",
                        "--permission-mode",
                        "bypassPermissions",
                    ]);
                    if !model.is_empty() {
                        c.args(["--model", model]);
                    }
                }
                "codex" => {
                    c.arg("app-server");
                }
                _ => {
                    c.args([
                        "--mode",
                        "rpc",
                        "--no-session",
                        "--provider",
                        if std::env::var_os("OPENAI_API_KEY").is_some() {
                            "openai"
                        } else {
                            "openai-codex"
                        },
                    ]);
                    if !model.is_empty() {
                        c.args(["--model", model]);
                    }
                }
            }
            let s = StdioSession::new(&mut c, tool, &cwd.to_string_lossy(), model)?;
            own.roots.push(s.child.pid());
            sessions.push(Session::Stdio(s));
        }
    }
    let version = version(&binary, &env)?;
    let mut samples = vec![];
    let mut errors = vec![];
    std::thread::sleep(settle);
    sample(&mut samples, "idle", &mut own);
    for turn in 0..a.turns {
        let prompt = TURNS[turn % TURNS.len()];
        let outcomes = std::thread::scope(|scope| {
            sessions
                .iter_mut()
                .map(|s| scope.spawn(move || s.turn(prompt, timeout)))
                .collect::<Vec<_>>()
                .into_iter()
                .map(|h| {
                    h.join()
                        .unwrap_or_else(|_| Err(anyhow::anyhow!("turn worker panicked")))
                })
                .collect::<Vec<_>>()
        });
        for (index, result) in outcomes.into_iter().enumerate() {
            if let Err(e) = result {
                errors.push(format!("turn {} session {}: {e}", turn + 1, index + 1));
            }
        }
        std::thread::sleep(settle);
        sample(&mut samples, &format!("turn{}", turn + 1), &mut own);
    }
    let mut checks = sessions[0].checks().unwrap_or_else(|e| {
        errors.push(format!("history: {e}"));
        json!({"error":e.to_string()})
    });
    if !embeddings.is_null() {
        checks["embeddings"] = embeddings;
    }
    if let Ok(dump) = std::env::var("NEXODE_BENCH_MEMORY_DUMP")
        && let Session::Nexode { socket, .. } = &sessions[0]
    {
        fs::create_dir_all(&dump)?;
        match debug_session(socket, "server:memory", None, Duration::from_secs(60)) {
            Ok(s) => fs::write(Path::new(&dump).join(format!("server-memory-{n}.json")), s)?,
            Err(e) => checks["memory_dump_error"] = json!(e.to_string()),
        }
    }
    for session in &sessions {
        if let Session::Nexode { socket, id } = session
            && let Err(e) = debug_session(
                socket,
                &format!("destroy_session:{id}"),
                None,
                Duration::from_secs(20),
            )
        {
            errors.push(format!("session cleanup: {e}"));
        }
    }
    let label = match tool {
        "nexode_memory_off" => "nexode (memory off)",
        "claude" => "Claude Code",
        "codex" => "Codex CLI",
        "opencode" => "OpenCode",
        s => s,
    };
    let v = json!({"tool":label,"sessions":n,"samples":samples,"errors":errors,"version":version,"checks":checks});
    drop(sessions);
    drop(server);
    drop(own);
    Ok(v)
}
fn sample(samples: &mut Vec<Value>, label: &str, own: &mut Ownership) {
    let (pss, _, n, _) = own.sample();
    samples.push(json!([label, pss, n]));
    eprintln!("{label}: {pss:.1} MiB PSS, {n} processes");
}
