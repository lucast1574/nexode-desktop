//! Owned-server subscribe/disconnect memory probe; no inference requests.
use super::{
    io,
    isolated::pause,
    single::{Server, wait_socket},
};
use anyhow::{Context, Result, ensure};
use clap::Parser;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::{fs::PermissionsExt, net::UnixStream, process::CommandExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
fn memory_text(text: &str) -> Result<BTreeMap<String, u64>> {
    let mut values = BTreeMap::new();
    for line in text.lines() {
        if let Some((key, rest)) = line.split_once(':')
            && ["VmRSS", "RssAnon", "VmHWM", "Threads"].contains(&key)
        {
            let mut parts = rest.split_whitespace();
            let n = parts
                .next()
                .context("empty process memory value")?
                .parse()?;
            if key != "Threads" {
                ensure!(parts.next() == Some("kB"), "unexpected status memory units");
            }
            values.insert(key.into(), n);
        }
    }
    for key in ["VmRSS", "RssAnon", "VmHWM", "Threads"] {
        ensure!(values.contains_key(key), "process status missing {key}");
    }
    Ok(values)
}
fn memory(pid: u32) -> Result<BTreeMap<String, u64>> {
    memory_text(&fs::read_to_string(format!("/proc/{pid}/status"))?)
}
fn subscribe(socket: &Path, work: &Path, abrupt: bool, realistic: bool) -> Result<UnixStream> {
    let mut stream = UnixStream::connect(socket)?;
    stream.set_write_timeout(Some(Duration::from_secs(15)))?;
    writeln!(
        stream,
        "{}",
        json!({"type":"subscribe","id":1,"working_dir":work})
    )?;
    if realistic {
        for (i, kind) in ["get_history", "get_model_catalog", "state", "ping"]
            .into_iter()
            .enumerate()
        {
            writeln!(stream, "{}", json!({"type":kind,"id":i+2}))?;
        }
    }
    if abrupt {
        return Ok(stream);
    }
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut reader = BufReader::new(&stream);
    let mut bytes = vec![];
    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .context("subscribe acknowledgement timed out")?;
        stream.set_read_timeout(Some(remaining))?;
        let chunk = reader.fill_buf()?;
        ensure!(
            !chunk.is_empty(),
            "server closed before subscription acknowledgement"
        );
        let n = chunk
            .iter()
            .position(|b| *b == b'\n')
            .map(|n| n + 1)
            .unwrap_or(chunk.len());
        ensure!(
            bytes.len() + n <= 1024 * 1024,
            "subscription frame too large"
        );
        bytes.extend_from_slice(&chunk[..n]);
        reader.consume(n);
        if bytes.last() == Some(&b'\n') {
            let value: Value = serde_json::from_slice(&bytes)?;
            bytes.clear();
            ensure!(value["type"] != "error", "subscription rejected");
            if value["type"] == "history" || (value["type"] == "done" && value["id"] == 1) {
                break;
            }
        }
    }
    drop(reader);
    if realistic {
        pause(0.5)?;
    }
    Ok(stream)
}
fn copy_private(source: &Path, destination: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(source)?;
    ensure!(
        !metadata.file_type().is_symlink(),
        "source-home snapshot cannot follow symlinks"
    );
    if metadata.is_dir() {
        fs::create_dir_all(destination)?;
        fs::set_permissions(destination, fs::Permissions::from_mode(0o700))?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            copy_private(&entry.path(), &destination.join(entry.file_name()))?;
        }
    } else {
        ensure!(
            metadata.is_file(),
            "source-home contains a non-regular entry"
        );
        fs::copy(source, destination)?;
        fs::set_permissions(destination, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}
#[derive(Parser)]
struct Args {
    #[arg(long)]
    binary: Option<PathBuf>,
    #[arg(long, default_value_t = 20)]
    sessions: usize,
    #[arg(long, default_value_t = 4)]
    rounds: usize,
    #[arg(long)]
    abrupt: bool,
    #[arg(long, default_value_t = 40.0)]
    settle: f64,
    #[arg(long)]
    realistic: bool,
    #[arg(long)]
    source_home: Option<PathBuf>,
    #[arg(long)]
    trim: bool,
    #[arg(long)]
    sudo: bool,
    #[arg(long, default_value = "gdb")]
    gdb: PathBuf,
}
pub(super) fn run(args: impl Iterator<Item = String>) -> Result<i32> {
    let a = Args::parse_from(std::iter::once("repro-idle-session-rss".into()).chain(args));
    ensure!(
        a.sessions > 0
            && a.sessions <= 1000
            && a.rounds > 0
            && a.rounds <= 1000
            && a.settle.is_finite()
            && a.settle >= 0.0
            && a.settle <= 3600.0,
        "invalid RSS probe bounds"
    );
    ensure!(!a.sudo || a.trim, "--sudo requires explicit --trim");
    let binary = a
        .binary
        .or_else(|| {
            std::env::var_os("HOME")
                .map(|h| PathBuf::from(h).join(".nexode/builds/shared-server/nexode"))
        })
        .context("provide --binary")?
        .canonicalize()?;
    let root = tempfile::Builder::new()
        .prefix("orbit-idle-rss-")
        .tempdir()?
        .keep();
    eprintln!("logs: {}", root.display());
    let home = root.join("home");
    let runtime = root.join("run");
    let work = root.join("work");
    for dir in [&home, &runtime, &work] {
        fs::create_dir(dir)?;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    }
    if let Some(source) = &a.source_home {
        for name in [
            "auth.json",
            "anthropic-auth.json",
            "openai-auth.json",
            "config.toml",
            "models",
        ] {
            let path = source.join(name);
            if path.exists() {
                copy_private(&path, &home.join(name))?;
            }
        }
    }
    let socket = runtime.join("t.sock");
    let debug = runtime.join("t-debug.sock");
    let mut command = Command::new(binary);
    command
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", &home)
        .env("NEXODE_HOME", &home)
        .env("NEXODE_RUNTIME_DIR", &runtime)
        .env("NEXODE_TEMP_SERVER", "1")
        .env("NEXODE_SERVER_OWNER_PID", std::process::id().to_string())
        .env("NEXODE_NO_TELEMETRY", "1")
        .env("NEXODE_DEBUG_CONTROL", "1")
        .current_dir(&work)
        .args(["--no-update", "--no-selfdev", "serve", "--socket"])
        .arg(&socket)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(fs::File::create(root.join("server.log"))?)
        .process_group(0);
    if a.source_home.is_none() {
        command.env("ANTHROPIC_API_KEY", "sk-ant-local-rss-fixture");
    }
    let mut server = Server(command.spawn()?);
    wait_socket(&socket, &mut server, Duration::from_secs(30))?;
    drop(subscribe(&socket, &work, false, false)?);
    pause(a.settle)?;
    let base = memory(server.0.id())?;
    let mut baseline = json!(base);
    baseline["phase"] = json!("baseline");
    println!("{baseline}");
    for round in 0..a.rounds {
        let mut connections = vec![];
        for _ in 0..a.sessions {
            connections.push(subscribe(&socket, &work, a.abrupt, a.realistic)?);
        }
        let opened = memory(server.0.id())?;
        drop(connections);
        pause(a.settle)?;
        let after = memory(server.0.id())?;
        let get = |cmd: &str| {
            io::debug(&debug, cmd, Duration::from_secs(30))
                .ok()
                .and_then(|v| serde_json::from_str::<Value>(&v).ok())
        };
        let info = get("info").unwrap_or(Value::Null);
        let allocator = get("server:memory").unwrap_or(Value::Null);
        let stats = &allocator["process"]["allocator"]["stats"];
        let mut row = json!({"phase":format!("round{}",round+1),"open_rss":opened["VmRSS"],"after_rss":after["VmRSS"],"after_anon":after["RssAnon"],"threads":after["Threads"],"delta_anon_vs_base":after["RssAnon"] as i64-base["RssAnon"] as i64,"live_sessions":info["session_count"].as_i64().unwrap_or(-1),"swarm_members":info["swarm_member_count"].as_i64().unwrap_or(-1)});
        if stats.is_object() {
            for (out, key) in [
                ("heap_allocated_kb", "allocated_bytes"),
                ("heap_mapped_kb", "mapped_bytes"),
                ("heap_retained_kb", "retained_bytes"),
            ] {
                row[out] = json!(stats[key].as_u64().unwrap_or(0) / 1024);
            }
        }
        println!("{row}");
    }
    if a.trim {
        let mut c = if a.sudo {
            let mut c = Command::new("sudo");
            c.arg("-n").arg(&a.gdb);
            c
        } else {
            Command::new(&a.gdb)
        };
        c.args([
            "-p",
            &server.0.id().to_string(),
            "-batch",
            "-ex",
            "call (int)malloc_trim(0)",
        ]);
        let result =
            crate::process::capture_interruptible(&mut c, Duration::from_secs(30), || {
                crate::ci_suites::unix::interrupted().is_some()
            })?;
        let trimmed = memory(server.0.id())?;
        println!(
            "{}",
            json!({"phase":"after_trim","trim_succeeded":result.status.success(),"rss":trimmed["VmRSS"],"anon":trimmed["RssAnon"],"delta_anon_vs_base":trimmed["RssAnon"] as i64-base["RssAnon"] as i64})
        );
    }
    let report = io::debug(&debug, "server:memory", Duration::from_secs(30))?;
    serde_json::from_str::<Value>(&report).context("invalid final memory report")?;
    fs::write(root.join("server_memory.json"), report)?;
    println!(
        "server:memory -> {}",
        root.join("server_memory.json").display()
    );
    Ok(0)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn status_fields_preserve_kib_and_reject_missing_data() {
        let m = memory_text(
            "Name: fixture\nVmRSS: 1024 kB\nRssAnon: 512 kB\nVmHWM: 2048 kB\nThreads: 3\n",
        )
        .unwrap();
        assert_eq!(m["RssAnon"], 512);
        assert_eq!(m["Threads"], 3);
        assert!(memory_text("VmRSS: 1024 MB").is_err());
        assert!(memory_text("Threads: 1").is_err());
    }
    #[test]
    fn fragmented_subscription_waits_for_own_done_not_unrelated_done() {
        use std::os::unix::net::UnixListener;
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("s");
        let listener = UnixListener::bind(&path).unwrap();
        let worker = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut input = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut input)
                .unwrap();
            assert_eq!(
                serde_json::from_str::<Value>(&input).unwrap()["type"],
                "subscribe"
            );
            stream
                .write_all(b"{\"type\":\"done\",\"id\":99}\n{\"type\":\"do")
                .unwrap();
            std::thread::sleep(Duration::from_millis(40));
            stream.write_all(b"ne\",\"id\":1}\n").unwrap();
        });
        let start = Instant::now();
        drop(subscribe(&path, d.path(), false, false).unwrap());
        assert!(start.elapsed() >= Duration::from_millis(30));
        worker.join().unwrap();
    }
}
