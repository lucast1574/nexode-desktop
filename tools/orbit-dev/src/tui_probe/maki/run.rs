use super::super::{
    isolated::pause,
    process_metrics::snapshot,
    screen::Client,
    single::{Server, wait_socket},
};
use super::median;
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    os::unix::process::CommandExt,
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};
fn text(client: &Client) -> Result<String> {
    Ok(client.snapshot()?.rows.join("\n"))
}
fn echo(client: &Client, probe: &str) -> Result<f64> {
    let start = Instant::now();
    client.send(probe.as_bytes())?;
    while start.elapsed() < Duration::from_secs(10) {
        let s = client.snapshot()?;
        ensure!(!s.eof, "client exited before application echo");
        if s.rows.join("\n").contains(probe) {
            return Ok(start.elapsed().as_secs_f64() * 1000.0);
        }
        pause(0.002)?;
    }
    anyhow::bail!("application input echo failed")
}
pub(super) fn measure(
    tool: &str,
    binary: &Path,
    sessions: usize,
    idle_seconds: f64,
    no_splash: bool,
) -> Result<Value> {
    let root = tempfile::Builder::new()
        .prefix("orbit-resource-")
        .tempdir()?;
    let work = root.path().join("workspace");
    fs::create_dir(&work)?;
    let mut env = BTreeMap::new();
    env.insert("PATH".to_owned(), std::env::var("PATH").unwrap_or_default());
    for (key, dir) in [
        ("HOME", "home"),
        ("XDG_CONFIG_HOME", "config"),
        ("XDG_DATA_HOME", "data"),
        ("XDG_STATE_HOME", "state"),
        ("XDG_CACHE_HOME", "cache"),
        ("NEXODE_HOME", "nexode"),
        ("NEXODE_RUNTIME_DIR", "runtime"),
    ] {
        let path = root.path().join(dir);
        fs::create_dir(&path)?;
        env.insert(key.into(), path.to_string_lossy().into_owned());
    }
    for (k, v) in [
        ("TERM", "xterm-256color"),
        ("COLORTERM", "truecolor"),
        (
            "ANTHROPIC_API_KEY",
            "sk-ant-benchmark-placeholder-not-valid",
        ),
        ("NEXODE_NO_TELEMETRY", "1"),
        ("NEXODE_MEMORY_ENABLED", "0"),
        ("NEXODE_TEMP_SERVER", "1"),
    ] {
        env.insert(k.into(), v.into());
    }
    env.insert(
        "NEXODE_SERVER_OWNER_PID".into(),
        std::process::id().to_string(),
    );
    fs::write(
        root.path().join("nexode/setup_hints.json"),
        br#"{"launch_count":10,"onboarding_start_choice_shown":true}"#,
    )?;
    let command = || {
        let mut c = Command::new(binary);
        c.env_clear().envs(&env).current_dir(&work);
        c
    };
    let socket = root.path().join("runtime/bench.sock");
    let start = Instant::now();
    let mut server = None;
    let mut server_ms = 0.0;
    if tool == "nexode" {
        let log = fs::File::create(root.path().join("server.log"))?;
        let mut c = command();
        c.args([
            "--no-update",
            "--no-selfdev",
            "--provider",
            "anthropic-api",
            "serve",
            "--socket",
        ])
        .arg(&socket)
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log)
        .process_group(0);
        let mut owned = Server(c.spawn()?);
        wait_socket(&socket, &mut owned, Duration::from_secs(15))?;
        server_ms = start.elapsed().as_secs_f64() * 1000.0;
        server = Some(owned);
    } else if no_splash {
        let config = root.path().join("config/maki");
        fs::create_dir_all(&config)?;
        fs::write(
            config.join("init.lua"),
            "maki.setup({ ui = { splash_animation = false } })\n",
        )?;
    }
    let mut clients = vec![];
    let mut startup = vec![];
    for i in 0..sessions {
        let mut c = command();
        if tool == "nexode" {
            c.args(["--no-update", "--no-selfdev", "--socket"])
                .arg(&socket);
        } else {
            if !no_splash {
                c.arg("--no-plugins");
            }
            c.args(["--no-commands", "--model", "anthropic/claude-sonnet-4-6"]);
        }
        let start = Instant::now();
        let client = Client::spawn(&mut c, 40, 120)?;
        let prompt = if tool == "nexode" { "1>" } else { "❯" };
        while !text(&client)?.contains(prompt) {
            ensure!(
                start.elapsed() < Duration::from_secs(15) && !client.snapshot()?.eof,
                "chat prompt did not appear"
            );
            pause(0.002)?;
        }
        let latency = echo(&client, &format!("bench{i}x"))?;
        let visible = client
            .snapshot()?
            .visible_at
            .map(|at| at.duration_since(start).as_secs_f64() * 1000.0);
        startup.push(json!({"first_visible_ms":visible,"input_ready_ms":start.elapsed().as_secs_f64()*1000.0,"initial_echo_ms":latency}));
        clients.push(client);
    }
    let first_screen = text(&clients[0])?;
    pause(2.0)?;
    let mut roots: Vec<_> = clients.iter().map(|c| c.pid).collect();
    if let Some(server) = &server {
        roots.push(server.0.id());
    }
    let before = snapshot(&roots)?;
    let bytes_before = clients
        .iter()
        .map(|c| c.snapshot().map(|s| s.bytes))
        .collect::<Result<Vec<_>>>()?
        .iter()
        .sum::<u64>();
    let start = Instant::now();
    let mut samples = vec![];
    for _ in 0..((idle_seconds * 5.0) as usize).max(1) {
        pause(0.2)?;
        samples.push(snapshot(&roots)?);
    }
    let after = snapshot(&roots)?;
    let wall = start.elapsed().as_secs_f64();
    ensure!(
        after["processes"].as_u64().unwrap_or(0) >= roots.len() as u64,
        "owned processes missing from accounting"
    );
    let bytes = clients
        .iter()
        .map(|c| c.snapshot().map(|s| s.bytes))
        .collect::<Result<Vec<_>>>()?
        .iter()
        .sum::<u64>();
    let mut latencies = vec![];
    for i in 0..20 {
        latencies.push(echo(&clients[i % clients.len()], &format!("q{i:02}z"))?);
    }
    let median_field = |key: &str| {
        median(
            samples
                .iter()
                .map(|v| v[key].as_f64().context("missing memory sample"))
                .collect::<Result<Vec<_>>>()?,
        )
    };
    let executables = roots
        .iter()
        .map(|pid| {
            Ok((
                pid.to_string(),
                json!(fs::read_link(format!("/proc/{pid}/exe"))?),
            ))
        })
        .collect::<Result<serde_json::Map<_, _>>>()?;
    Ok(
        json!({"tool":tool,"sessions":sessions,"server_ready_ms":server_ms,"cold_input_ready_ms":server_ms+startup[0]["input_ready_ms"].as_f64().unwrap(),"startup":startup,"idle_seconds":wall,"idle_cpu_percent_one_core":100.0*(after["cpu_seconds"].as_f64().unwrap()-before["cpu_seconds"].as_f64().unwrap())/wall,"idle_output_bytes":bytes.saturating_sub(bytes_before),"pss_mib":median_field("pss_mib")?,"private_mib":median_field("private_mib")?,"rss_mib":median_field("rss_mib")?,"processes":after["processes"],"input_echo_median_ms":median(latencies.clone())?,"input_echo_max_ms":latencies.iter().copied().max_by(f64::total_cmp),"input_echo_samples_ms":latencies,"screen":first_screen,"settled_screen":text(&clients[0])?,"executables":executables}),
    )
}
