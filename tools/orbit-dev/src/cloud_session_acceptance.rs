//! Real cloud CLI acceptance with a native, nonexecuting transport fixture.
use anyhow::{Context, Result, ensure};
use clap::Parser;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
    process::{Command, Output},
    time::Duration,
};
const SESSION: &str = "session_fixture";
#[derive(Parser)]
struct Args {
    #[arg(long)]
    binary: PathBuf,
}
pub fn fixture(invoked: &str) -> Result<()> {
    let root =
        PathBuf::from(std::env::var_os("CLOUD_CLI_FIXTURE").context("missing fixture root")?);
    if invoked != "cloud-transport" {
        fs::write(root.join("unexpected-helper"), "called")?;
        anyhow::bail!("unexpected provisioning helper");
    }
    let args: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        args.len() == 1,
        "transport expects one quoted remote script"
    );
    let words = shlex::split(&args[0]).context("invalid remote quoting")?;
    let boundary = words
        .iter()
        .position(|s| s == "exec")
        .context("missing exec boundary")?;
    let argv = &words[boundary + 1..];
    writeln!(
        fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(root.join("transport.jsonl"))?,
        "{}",
        json!(argv)
    )?;
    ensure!(
        argv.len() >= 7 && argv[..4] == ["nexode", "--no-update", "--no-selfdev", "cloud"],
        "unexpected remote command prefix"
    );
    let session = argv
        .iter()
        .position(|a| a == "--session")
        .and_then(|i| argv.get(i + 1));
    ensure!(
        session.map(String::as_str) == Some(SESSION),
        "unexpected session"
    );
    let state: Value = serde_json::from_slice(&fs::read(root.join("state.json"))?)?;
    match argv[4].as_str() {
        "run-detached" => {
            ensure!(
                argv.len() == 9 && argv[7] == "--",
                "prompt escaped single argument"
            );
            fs::write(
                root.join("received_prompt.json"),
                serde_json::to_vec(&argv[8])?,
            )?;
            println!(
                "{}",
                json!({"session_id":SESSION,"state":"running","pid":12345})
            );
        }
        "inspect" => {
            if state["malformed"] == true {
                println!("{{not-json");
            } else {
                println!(
                    "{}",
                    json!({"session_id":state["session_id"].as_str().unwrap_or(SESSION),"state":state["state"].as_str().unwrap_or("idle"),"messages":[],"log":"fixture log\n","exit_code":null})
                );
            }
        }
        command => anyhow::bail!("unexpected remote operation: {command}"),
    }
    Ok(())
}
struct Harness {
    binary: PathBuf,
    root: PathBuf,
    env: BTreeMap<String, String>,
}
impl Harness {
    fn invoke(&self, args: &[&str]) -> Result<Output> {
        crate::process::capture_interruptible(
            Command::new(&self.binary)
                .args(["--no-update", "--no-selfdev", "cloud"])
                .args(args)
                .env_clear()
                .envs(&self.env)
                .current_dir(&self.root),
            Duration::from_secs(20),
            || crate::ci_suites::unix::interrupted().is_some(),
        )
    }
    fn run(&self, args: &[&str], code: i32) -> Result<String> {
        let result = self.invoke(args)?;
        ensure!(
            result.status.code() == Some(code),
            "cloud {args:?}: expected {code}, got {}\n{}\n{}",
            result.status,
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        Ok(String::from_utf8(result.stdout)?)
    }
    fn json(&self, args: &[&str], code: i32) -> Result<Value> {
        Ok(serde_json::from_str(&self.run(args, code)?)?)
    }
    fn rejects(&self, args: &[&str], contains: Option<&str>) -> Result<()> {
        let result = self.invoke(args)?;
        ensure!(!result.status.success(), "unexpected success: {args:?}");
        if let Some(expected) = contains {
            ensure!(
                String::from_utf8_lossy(&result.stderr)
                    .to_lowercase()
                    .contains(expected),
                "missing {expected}: {}",
                String::from_utf8_lossy(&result.stderr)
            );
        }
        Ok(())
    }
    fn state(&self, value: Value) -> Result<()> {
        fs::write(
            self.root.join("fixture/state.json"),
            serde_json::to_vec(&value)?,
        )?;
        Ok(())
    }
}
fn snapshot(paths: &[PathBuf]) -> Result<BTreeMap<PathBuf, Option<String>>> {
    fn visit(path: &Path, result: &mut BTreeMap<PathBuf, Option<String>>) -> Result<()> {
        if !path.exists() {
            result.insert(path.into(), None);
        } else if path.is_dir() {
            result.insert(path.into(), Some("directory".into()));
            for entry in fs::read_dir(path)? {
                visit(&entry?.path(), result)?;
            }
        } else {
            result.insert(
                path.into(),
                Some(format!("{:x}", Sha256::digest(fs::read(path)?))),
            );
        }
        Ok(())
    }
    let mut result = BTreeMap::new();
    for path in paths {
        visit(path, &mut result)?;
    }
    Ok(result)
}
pub fn run(args: impl Iterator<Item = String>) -> Result<()> {
    let args = Args::parse_from(std::iter::once("test-cloud-session-cli".into()).chain(args));
    let _signals = crate::ci_suites::unix::Signals::install()?;
    let binary = args.binary.canonicalize()?;
    ensure!(
        binary.is_file() && fs::metadata(&binary)?.permissions().mode() & 0o111 != 0,
        "binary is not executable"
    );
    let scratch = PathBuf::from(
        std::env::var_os("NEXODE_SCRATCH_DIR").context("NEXODE_SCRATCH_DIR required")?,
    );
    fs::create_dir_all(&scratch)?;
    let temp = tempfile::Builder::new()
        .prefix("cloud-cli-")
        .tempdir_in(scratch)?;
    exercise(binary, temp.path())
}
fn exercise(binary: PathBuf, root: &Path) -> Result<()> {
    let home = root.join("home");
    let nexode = home.join(".nexode");
    let runtime = root.join("runtime");
    let bin = root.join("bin");
    let fixture = root.join("fixture");
    for path in [
        &home,
        &nexode.join("session_leases"),
        &runtime,
        &bin,
        &fixture,
    ] {
        fs::create_dir_all(path)?;
    }
    fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700))?;
    let transport = bin.join("cloud-transport");
    ensure!(
        !transport.to_string_lossy().chars().any(char::is_whitespace),
        "scratch path must have no whitespace (transport splits whitespace)"
    );
    let executable = std::env::current_exe()?;
    for name in ["cloud-transport", "ssh", "ssh-keygen", "curl", "wget"] {
        symlink(&executable, bin.join(name))?;
    }
    let mut env = BTreeMap::from([
        ("PATH".into(), format!("{}:/usr/bin:/bin", bin.display())),
        ("NO_COLOR".into(), "1".into()),
        ("TERM".into(), "dumb".into()),
    ]);
    for key in ["LANG", "LC_ALL", "TZ"] {
        if let Ok(value) = std::env::var(key) {
            env.insert(key.into(), value);
        }
    }
    for (key, path) in [
        ("HOME", &home),
        ("NEXODE_HOME", &nexode),
        ("XDG_RUNTIME_DIR", &runtime),
        ("TMPDIR", &root.to_owned()),
        ("XDG_CONFIG_HOME", &home.join(".config")),
        ("XDG_DATA_HOME", &home.join(".local/share")),
        ("XDG_CACHE_HOME", &home.join(".cache")),
        ("NEXODE_CLOUD_TRANSPORT", &transport),
        ("CLOUD_CLI_FIXTURE", &fixture),
    ] {
        env.insert(key.into(), path.to_string_lossy().into_owned());
    }
    let harness = Harness {
        binary,
        root: root.into(),
        env,
    };
    let lease = nexode.join(format!("session_leases/{SESSION}.json"));
    fs::write(&lease,json!({"session_id":SESSION,"away_host":"fixture-host","epoch":1,"updated_at":"2026-11-06T10:34:29Z"}).to_string())?;
    harness.state(json!({"state":"idle"}))?;
    let report = harness.json(&["status", "--session", SESSION, "--json"], 0)?;
    ensure!(
        report["session_id"] == SESSION
            && report["state"] == "idle"
            && report["messages"] == json!([])
            && report["log"] == "fixture log\n",
        "bad idle status: {report}"
    );
    ensure!(
        harness.json(
            &["wait", "--session", SESSION, "--timeout", "0", "--json"],
            0
        )?["state"]
            == "idle",
        "idle wait did not complete"
    );
    harness.state(json!({"state":"running"}))?;
    ensure!(
        harness.json(
            &["wait", "--session", SESSION, "--timeout", "0", "--json"],
            2
        )?["state"]
            == "running",
        "timeout lost status"
    );
    harness.state(json!({"state":"failed"}))?;
    harness.rejects(
        &["wait", "--session", SESSION, "--timeout", "0", "--json"],
        Some("cloud turn failed"),
    )?;
    ensure!(
        harness.run(&["logs", "--session", SESSION, "--lines", "7"], 0)? == "fixture log\n",
        "bad logs"
    );
    let marker = root.join("must-not-exist");
    let prompt = format!(
        "$(touch {0}); 'single' \"double\" `touch {0}`\nsecond line\\end",
        marker.display()
    );
    let receipt = harness.json(&["send", "--session", SESSION, "--json", "--", &prompt], 0)?;
    ensure!(
        receipt["session_id"] == SESSION && receipt["state"] == "running",
        "bad send receipt"
    );
    ensure!(
        serde_json::from_slice::<String>(&fs::read(fixture.join("received_prompt.json"))?)?
            == prompt
            && !marker.exists(),
        "prompt changed or executed"
    );
    for (state, message) in [
        (
            json!({"state":"idle","session_id":"session_other"}),
            "different session",
        ),
        (json!({"state":"unexpected"}), "unknown session state"),
        (json!({"malformed":true}), "invalid cloud inspect response"),
    ] {
        harness.state(state)?;
        harness.rejects(&["status", "--session", SESSION, "--json"], Some(message))?;
    }
    let missing = harness.json(&["problems", "--json"], 0)?;
    ensure!(
        missing["problems"] == json!([]) && missing["log_status"] == "missing",
        "bad missing-log handling"
    );
    fs::create_dir_all(nexode.join("logs"))?;
    fs::write(
        nexode.join("logs/cloud-problems.jsonl"),
        format!(
            "not-json\n{}\n{{partial",
            json!({"ts":"2026-11-06T10:34:29Z","version":"0.99.714","command":"/cloud --host secret-value","session_id":SESSION,"exit_code":255,"stderr":"Bearer secret-value permission denied"})
        ),
    )?;
    let raw = harness.run(&["problems", "--limit", "999999", "--json"], 0)?;
    let problems: Value = serde_json::from_str(&raw)?;
    ensure!(
        problems["malformed_lines"] == 2
            && problems["problems"]
                .as_array()
                .is_some_and(|v| v.len() == 1),
        "bad malformed log handling"
    );
    ensure!(!raw.contains("secret-value"), "problems leaked secret");
    let calls = fs::read(fixture.join("transport.jsonl"))?;
    let protected = [
        lease,
        nexode.join("cloud"),
        nexode.join("auth"),
        nexode.join("credentials.json"),
        home.join(".ssh"),
    ];
    let before = snapshot(&protected)?;
    let doctor = harness.json(&["doctor", "--json"], 0)?;
    ensure!(
        doctor["read_only"] == true
            && doctor["account"] == "not configured"
            && doctor["moved_sessions"] == 1,
        "bad isolated doctor: {doctor}"
    );
    ensure!(
        snapshot(&protected)? == before,
        "doctor mutated protected state"
    );
    ensure!(
        fs::read(fixture.join("transport.jsonl"))? == calls
            && !fixture.join("unexpected-helper").exists(),
        "doctor executed transport or provisioning"
    );
    let protected = [nexode.join("cloud_runs"), nexode.join("session_leases")];
    let before = snapshot(&protected)?;
    for traversal in [
        "../escape",
        "../../outside",
        "/tmp/escape",
        ".",
        "session/child",
    ] {
        harness.rejects(&["inspect", "--session", traversal], None)?;
    }
    ensure!(
        snapshot(&protected)? == before && !marker.exists(),
        "traversal mutated state or prompt executed"
    );
    println!(
        "PASS: status, wait, logs, send quoting, response validation, problems, read-only doctor, inspect traversal"
    );
    Ok(())
}
