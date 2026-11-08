//! First terminal-output latency and repaint rate, with optional external daemon.
use super::{
    debug_fixture::{Fixture, OwnedSession},
    io,
    isolated::pause,
    screen::Client,
};
use anyhow::{Result, ensure};
use clap::Parser;
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};
#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "./target/release/nexode")]
    binary: PathBuf,
    #[arg(long)]
    json: bool,
    #[arg(short, long)]
    verbose: bool,
    #[arg(long)]
    keep: bool,
    #[arg(long, default_value_t = 1)]
    repeat: usize,
    #[arg(long)]
    live: bool,
    #[arg(long, requires = "live")]
    socket: Option<PathBuf>,
    #[arg(long, requires = "live")]
    debug_socket: Option<PathBuf>,
    #[arg(long)]
    immediate: bool,
    #[arg(long, default_value_t = 0)]
    background_clients: usize,
    #[arg(long)]
    no_idle_animation: bool,
}
struct Ui {
    client: Client,
    _session: OwnedSession,
    cmd: PathBuf,
    resp: PathBuf,
    first_answer_ms: f64,
}
impl Ui {
    fn command(&self, cmd: &str) -> Result<String> {
        io::client_command(&self.cmd, &self.resp, cmd, Duration::from_secs(6))
    }
    fn spawn(
        mut command: Command,
        session: OwnedSession,
        root: &Path,
        index: usize,
        no_animation: bool,
    ) -> Result<Self> {
        let cmd = root.join(format!("cmd-{index}"));
        let resp = root.join(format!("resp-{index}"));
        command
            .args(["--resume", &session.id])
            .env("NEXODE_DEBUG_CONTROL", "1")
            .env("NEXODE_DEBUG_CMD_PATH", &cmd)
            .env("NEXODE_DEBUG_RESPONSE_PATH", &resp)
            .env("NEXODE_THEME", "dark")
            .env("TERM", "xterm-256color");
        if no_animation {
            command.env("NEXODE_IDLE_ANIMATION", "false");
        }
        let start = Instant::now();
        let client = Client::spawn(&mut command, 40, 120)?;
        let mut ui = Self {
            client,
            _session: session,
            cmd,
            resp,
            first_answer_ms: 0.0,
        };
        loop {
            if ui.command("input").is_ok() {
                break;
            }
            ensure!(
                start.elapsed() < Duration::from_secs(60) && !ui.client.snapshot()?.eof,
                "client never became ready"
            );
            pause(0.05)?;
        }
        ui.first_answer_ms = start.elapsed().as_secs_f64() * 1000.0;
        Ok(ui)
    }
}
fn stats(values: &[f64]) -> Value {
    let mut v = values.to_vec();
    v.sort_by(f64::total_cmp);
    let n = v.len();
    json!({"p50":(v[(n-1)/2]+v[n/2])/2.0,"p95":v[(n*95/100).saturating_sub(1)],"max":v[n-1],"mean":v.iter().sum::<f64>()/n as f64})
}
fn measure(ui: &Ui, name: &str, text: &str, burst: bool, verbose: bool) -> Result<Value> {
    let mut firsts = vec![];
    let mut settled = vec![];
    let mut missing = 0;
    for ch in text.chars() {
        if !burst {
            pause(0.12)?;
        }
        let start = Instant::now();
        let mut encoded = [0; 4];
        ui.client.send(ch.encode_utf8(&mut encoded).as_bytes())?;
        let deadline = start + Duration::from_secs(2);
        let first = loop {
            let s = ui.client.snapshot()?;
            ensure!(!s.eof, "client exited while typing");
            if let Some(t) = s.output_events.iter().find(|t| **t >= start) {
                break Some(*t);
            }
            if Instant::now() >= deadline {
                break None;
            }
            pause(0.001)?;
        };
        let Some(first) = first else {
            firsts.push(2000.0);
            settled.push(2000.0);
            missing += 1;
            continue;
        };
        let mut last = first;
        while Instant::now() < deadline {
            pause(0.005)?;
            if let Some(t) = ui.client.snapshot()?.last_output {
                last = last.max(t);
            }
            if last.elapsed() > Duration::from_millis(25) {
                break;
            }
        }
        let ms = first.duration_since(start).as_secs_f64() * 1000.0;
        firsts.push(ms);
        settled.push(last.duration_since(start).as_secs_f64() * 1000.0);
        if verbose {
            eprintln!("{name} {ch:?}: {ms:.2}ms");
        }
    }
    let f = stats(&firsts);
    let s = stats(&settled);
    Ok(
        json!({"scenario":name,"text":text,"count":firsts.len(),"missing_repaints":missing,"p50_ms":f["p50"],"p95_ms":f["p95"],"max_ms":f["max"],"mean_ms":f["mean"],"settled_p50_ms":s["p50"],"settled_p95_ms":s["p95"],"settled_max_ms":s["max"],"samples_ms":firsts,"settled_samples_ms":settled}),
    )
}
fn diagnostic(ui: &Ui, command: &str) -> Value {
    match ui
        .command(command)
        .and_then(|s| Ok(serde_json::from_str::<Value>(&s)?))
    {
        Ok(v) => v,
        Err(_) => json!({"status":"unavailable","command":command}),
    }
}
pub(super) fn run(args: impl Iterator<Item = String>) -> Result<i32> {
    match run_inner(args) {
        Ok(code) => Ok(code),
        Err(error) => {
            eprintln!("input-lag setup/probe failed: {error:#}");
            Ok(3)
        }
    }
}
fn run_inner(args: impl Iterator<Item = String>) -> Result<i32> {
    let a = Args::try_parse_from(std::iter::once("repro-input-lag".into()).chain(args))?;
    ensure!(
        (1..=100).contains(&a.repeat) && a.background_clients <= 100,
        "invalid repeat or background count"
    );
    let binary = a.binary.canonicalize()?;
    let fixture = if a.live {
        None
    } else {
        Some(Fixture::new(&binary)?)
    };
    let root = tempfile::Builder::new()
        .prefix("orbit-input-lag-")
        .tempdir()?;
    let runtime = std::env::var_os("NEXODE_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(format!("/run/user/{}", unsafe { libc::getuid() })));
    let socket = a
        .socket
        .or_else(|| std::env::var_os("NEXODE_SOCKET").map(PathBuf::from))
        .unwrap_or_else(|| runtime.join("nexode.sock"));
    let debug = a
        .debug_socket
        .unwrap_or_else(|| runtime.join("nexode-debug.sock"));
    let debug = fixture.as_ref().map(|f| &f.socket).unwrap_or(&debug);
    let cwd = std::env::current_dir()?;
    let spawn = |i| -> Result<Ui> {
        let session = OwnedSession::new(debug, &cwd)?;
        let mut command = if let Some(f) = &fixture {
            f.client_command(&binary)
        } else {
            let mut c = Command::new(&binary);
            c.args(["--no-update", "--no-selfdev", "--socket"])
                .arg(&socket);
            c
        };
        command.current_dir(&cwd);
        Ui::spawn(command, session, root.path(), i, a.no_idle_animation)
    };
    let mut background = vec![];
    for i in 0..a.background_clients {
        background.push(spawn(i)?);
    }
    if !background.is_empty() {
        pause(3.0)?;
    }
    let mut ui = spawn(a.background_clients)?;
    if !a.immediate {
        pause(2.0)?;
    }
    let before = ui.client.snapshot()?;
    let idle_start = Instant::now();
    pause(2.0)?;
    let after = ui.client.snapshot()?;
    let seconds = idle_start.elapsed().as_secs_f64();
    let chunks = (after.frames - before.frames) as f64 / seconds;
    let idle = json!({"idle_chunks_per_s":chunks,"idle_bytes_per_s":(after.bytes-before.bytes) as f64/seconds});
    let draw = diagnostic(&ui, "draw-stats 16");
    let mut runs = vec![];
    for _ in 0..a.repeat {
        for (name, text, burst) in [
            ("plain-typing", "hello world", false),
            ("slash-popup", "/model", false),
            ("plain-after-slash", " gpt", false),
            ("plain-burst", "the quick brown fox", true),
            ("slash-burst", "/model gpt", true),
        ] {
            runs.push(measure(&ui, name, text, burst, a.verbose)?);
            ui.command("set_input:")?;
            pause(0.2)?;
        }
    }
    let reproduced = chunks > 8.0
        || runs.iter().any(|r| {
            r["p50_ms"].as_f64().unwrap() > 40.0
                || r["p95_ms"].as_f64().unwrap() > 120.0
                || r["missing_repaints"].as_u64().unwrap() > 0
        });
    let mut output = json!({"binary":binary,"mode":if a.live{"live"}else{"isolated"},"background_clients":background.len(),"client_first_debug_answer_ms":ui.first_answer_ms,"idle":idle,"idle_draw_stats":draw,"runs":runs,"slow_frames":diagnostic(&ui,"slow-frames 32"),"budget":{"p50_ms":40.0,"p95_ms":120.0,"idle_chunks_per_s":8.0},"lag_reproduced":reproduced,"idle_loop_busy":chunks>8.0});
    ui._session.close()?;
    for bg in &mut background {
        bg._session.close()?;
    }
    drop(ui);
    drop(background);
    if a.keep {
        output["artifacts"] = json!(root.keep());
        if let Some(f) = fixture {
            output["server_artifacts"] = json!(f.retain_artifacts());
        }
    }
    if !a.json {
        eprintln!(
            "Input-lag probe: {}",
            if reproduced {
                "budget exceeded"
            } else {
                "within budget"
            }
        );
    }
    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(i32::from(reproduced))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn percentiles_match_original_nearest_rank() {
        let v = stats(&[5.0, 1.0, 4.0, 2.0, 3.0]);
        assert_eq!(v["p50"], 3.0);
        assert_eq!(v["p95"], 4.0);
        assert_eq!(stats(&[2.0])["p95"], 2.0);
    }
}
