//! Linux process-tree PSS benchmarks; owns every process it terminates.
mod common;
mod headless;
mod protocols;
use crate::tui_probe::screen::Client;
use anyhow::{Context, Result, ensure};
use clap::Parser;
pub use headless::run as headless;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    process::Command,
    time::{Duration, Instant},
};
#[derive(Parser)]
struct Args {
    #[arg(long)]
    sessions: usize,
    #[arg(long,num_args=1..,default_values=["nexode_memory_off","nexode_memory_on","pi","prime_agent","codex","opencode","copilot_cli","cursor_agent","claude_code","antigravity_cli"])]
    tools: Vec<String>,
    #[arg(long, default_value_t = 20.)]
    timeout: f64,
    #[arg(long, default_value_t = 1.)]
    settle: f64,
    #[arg(long, default_value = ".")]
    cwd: PathBuf,
    #[arg(long)]
    json_out: Option<PathBuf>,
    #[arg(long)]
    nexode_bin: Option<PathBuf>,
}
fn tool_name(name: &str) -> Result<&str> {
    Ok(match name {
        "nexode_memory_off" | "nexode_memory_on" => "nexode",
        "pi" => "pi",
        "prime_agent" => "prime-agent",
        "codex" => "codex",
        "opencode" => "opencode",
        "copilot_cli" => "copilot",
        "cursor_agent" => "cursor-agent",
        "claude_code" => "claude",
        "antigravity_cli" => "agy",
        _ => anyhow::bail!("unknown tool: {name}"),
    })
}
pub fn interactive(args: impl Iterator<Item = String>) -> Result<i32> {
    let a = Args::try_parse_from(std::iter::once("bench-memory-cli".into()).chain(args))?;
    ensure!((1..=256).contains(&a.sessions), "sessions must be 1..256");
    let timeout = common::duration(a.timeout, false)?;
    let settle = common::duration(a.settle, true)?;
    let cwd = a.cwd.canonicalize()?;
    for name in &a.tools {
        tool_name(name)?;
    }
    let mut results = vec![];
    let mut failed = false;
    for name in &a.tools {
        let result = interactive_tool(name, &a, &cwd, timeout, settle);
        let value = match result {
            Ok(v) => {
                failed |= v["ready"] == false;
                v
            }
            Err(e) => {
                failed = true;
                json!({"tool":name,"sessions":a.sessions,"error":e.to_string()})
            }
        };
        println!("{}", serde_json::to_string_pretty(&value)?);
        results.push(value);
        if let Some(p) = &a.json_out {
            std::fs::write(p, serde_json::to_vec_pretty(&results)?)?;
        }
    }
    Ok(i32::from(failed))
}
fn interactive_tool(
    name: &str,
    a: &Args,
    cwd: &std::path::Path,
    timeout: Duration,
    settle: Duration,
) -> Result<Value> {
    let program = tool_name(name)?;
    let binary = common::binary(
        program,
        if program == "nexode" {
            a.nexode_bin.as_deref()
        } else {
            None
        },
    )?;
    let root = tempfile::tempdir()?;
    let mut ownership = common::Ownership::new();
    let mut env = BTreeMap::new();
    env.insert("ORBIT_MEMORY_BENCH_OWNER".into(), ownership.marker.clone());
    env.insert("TERM".into(), "xterm-256color".into());
    let mut server = None;
    if program == "claude" {
        env.extend(common::claude_home(root.path())?);
    }
    let mut argv = vec![];
    if program == "nexode" {
        env.extend(common::nexode_home(
            root.path(),
            name == "nexode_memory_on",
        )?);
        let socket = root.path().join("run/bench.sock");
        let mut c = Command::new(&binary);
        c.args(["--no-update", "--no-selfdev", "serve", "--socket"])
            .arg(&socket)
            .envs(&env)
            .current_dir(cwd);
        server = Some(common::ChildGuard::spawn(&mut c)?);
        ownership.roots.push(server.as_ref().unwrap().pid());
        common::wait_socket(&socket, timeout)?;
        argv = vec![
            "--no-update".into(),
            "--no-selfdev".into(),
            "--socket".into(),
            socket.to_string_lossy().into_owned(),
        ];
        if name == "nexode_memory_on" {
            std::thread::sleep(settle.max(Duration::from_secs(5)));
        }
    }
    let version = common::version(&binary, &env)?;
    let mut clients = vec![];
    let mut launches = vec![];
    for i in 0..a.sessions {
        let mut command = Command::new(&binary);
        command.args(&argv).envs(&env).current_dir(cwd);
        let client = Client::spawn(&mut command, 24, 120)?;
        ownership.roots.push(client.pid);
        let start = Instant::now();
        let mut visible = None;
        let mut input = None;
        let mut excerpt = String::new();
        while start.elapsed() < timeout {
            let state = client.snapshot()?;
            if let Some(e) = state.error {
                anyhow::bail!("terminal error: {e}")
            };
            if state.eof {
                break;
            }
            let text = state.rows.join("\n");
            if visible.is_none()
                && let Some(line) = text.lines().find(|l| {
                    l.chars().filter(|c| c.is_alphanumeric()).count() >= 3 && l.len() >= 4
                })
            {
                visible = Some(start.elapsed().as_secs_f64());
                excerpt = line.chars().take(160).collect();
                client.send(b"jqx92")?;
            }
            if visible.is_some() && text.contains("jqx92") {
                input = Some(start.elapsed().as_secs_f64());
                client.send(b"\x7f\x7f\x7f\x7f\x7f")?;
                break;
            }
            std::thread::sleep(Duration::from_millis(30));
        }
        ownership.observe();
        std::thread::sleep(if name == "nexode_memory_on" {
            settle.max(Duration::from_secs(2))
        } else {
            settle
        });
        launches.push(json!({"session":i+1,"ready":visible.is_some(),"input_ready":input.is_some(),"seconds_to_visible":visible,"seconds_to_input_ready":input,"excerpt":excerpt}));
        clients.push(client);
    }
    let (pss, rss, count, unreadable) = ownership.sample();
    let ready = launches.iter().all(|v| v["input_ready"] == true);
    let value = json!({"tool":name,"sessions":a.sessions,"pss_mb":pss,"rss_mb":rss,"process_count":count,"unreadable_processes":unreadable,"version":version,"ready":ready,"launches":launches,"notes":[format!("rss_mb={rss}"),"Only owned descendants and tagged detached workers are measured; unrelated pre-existing workers are excluded."]});
    drop(clients);
    drop(server);
    drop(ownership);
    Ok(value)
}
