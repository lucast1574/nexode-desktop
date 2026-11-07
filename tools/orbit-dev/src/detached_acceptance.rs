//! Exercise actual detached turns using the native loopback provider.
use anyhow::{Context, Result, ensure};
use clap::Parser;
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};
#[derive(Parser)]
struct Args {
    #[arg(long)]
    binary: PathBuf,
}
struct Fixture {
    root: PathBuf,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if let Err(e) = fs::write(self.root.join("gates/detached.release"), "release\n") {
            eprintln!("release gate cleanup: {e}");
        }
        if let Err(e) = cleanup_workspace(&self.root) {
            eprintln!("workspace cleanup: {e:#}");
        }
    }
}
#[cfg(target_os = "linux")]
pub fn cleanup_workspace(root: &Path) -> Result<()> {
    let marker = format!("NEXODE_TEST_WORKSPACE={}", root.display());
    let owned = |pid: i32| {
        fs::read(format!("/proc/{pid}/environ"))
            .is_ok_and(|env| env.split(|b| *b == 0).any(|v| v == marker.as_bytes()))
    };
    let mut pids = Vec::new();
    for entry in fs::read_dir("/proc")? {
        let entry = entry?;
        if let Ok(pid) = entry.file_name().to_string_lossy().parse::<i32>()
            && pid != std::process::id() as i32
            && owned(pid)
        {
            pids.push(pid);
        }
    }
    for signal in [libc::SIGTERM, libc::SIGKILL] {
        for pid in &pids {
            if owned(*pid) {
                unsafe {
                    libc::kill(*pid, signal);
                }
            }
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        while pids.iter().any(|p| owned(*p)) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        pids.retain(|p| owned(*p));
        if pids.is_empty() {
            return Ok(());
        }
    }
    ensure!(
        pids.is_empty(),
        "owned workspace processes did not terminate"
    );
    Ok(())
}
#[cfg(not(target_os = "linux"))]
pub fn cleanup_workspace(_root: &Path) -> Result<()> {
    anyhow::bail!("workspace process cleanup requires Linux /proc")
}
fn run_command(
    binary: &Path,
    root: &Path,
    env: &BTreeMap<String, String>,
    args: &[&str],
    expected: i32,
) -> Result<String> {
    let mut c = Command::new(binary);
    c.env_clear()
        .envs(env)
        .current_dir(root)
        .args(["--no-update", "--no-selfdev"])
        .args(args);
    let out = crate::process::capture(&mut c, Duration::from_secs(60))?;
    ensure!(
        out.status.code() == Some(expected),
        "{args:?}: exit {}\n{}\n{}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    Ok(if expected == 0 {
        String::from_utf8(out.stdout)?
    } else {
        String::from_utf8(out.stderr)?
    })
}
fn private_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}
pub fn run(args: impl Iterator<Item = String>) -> Result<()> {
    ensure!(
        cfg!(target_os = "linux"),
        "detached cloud acceptance currently requires Linux"
    );
    let args = Args::parse_from(std::iter::once("test-cloud-detached-turn".into()).chain(args));
    let binary = args.binary.canonicalize()?;
    let scratch =
        std::env::var_os("NEXODE_SCRATCH_DIR").context("NEXODE_SCRATCH_DIR is required")?;
    let temp = tempfile::Builder::new()
        .prefix("cloud-detached-")
        .tempdir_in(scratch)?;
    let root = temp.path();
    let home = root.join("home");
    let nexode = home.join(".nexode");
    let runtime = root.join("runtime");
    let gates = root.join("gates");
    for p in [&nexode, &runtime, &gates] {
        private_dir(p)?;
    }
    let _fixture = Fixture {
        root: root.to_owned(),
    };
    let server = crate::fake_openai::Server::start(
        0,
        crate::fake_openai::Options {
            log: root.join("requests.jsonl"),
            label: "detached".into(),
            control: Some(gates.clone()),
        },
    )?;
    let mut env = BTreeMap::from([
        ("PATH".into(), "/usr/local/bin:/usr/bin:/bin".into()),
        ("LANG".into(), "C.UTF-8".into()),
    ]);
    for (k, v) in [
        ("HOME", home.as_path()),
        ("NEXODE_HOME", nexode.as_path()),
        ("XDG_RUNTIME_DIR", runtime.as_path()),
        ("NEXODE_RUNTIME_DIR", runtime.as_path()),
        ("TMPDIR", root),
        ("NEXODE_TEST_WORKSPACE", root),
    ] {
        env.insert(k.into(), v.to_string_lossy().into_owned());
    }
    for (k, v) in [
        ("NEXODE_NO_TELEMETRY", "1"),
        ("NEXODE_NO_AUTO_UPDATE", "1"),
        ("NEXODE_NON_INTERACTIVE", "1"),
        ("OPENAI_COMPAT_API_KEY", "isolated-fixture"),
        ("NEXODE_OPENAI_COMPAT_DEFAULT_MODEL", "fake-model"),
    ] {
        env.insert(k.into(), v.into());
    }
    env.insert(
        "NEXODE_OPENAI_COMPAT_API_BASE".into(),
        format!("http://127.0.0.1:{}/v1", server.port),
    );
    fs::write(
        nexode.join("config.toml"),
        "[provider]\ndefault_provider = \"openai-compatible\"\n[server]\nmobile_push = false\n",
    )?;
    let run = |args: &[&str], code| run_command(&binary, root, &env, args, code);
    let inspect = |session: &str| -> Result<Value> {
        Ok(serde_json::from_str(&run(
            &["cloud", "inspect", "--session", session],
            0,
        )?)?)
    };
    let seed: Value = serde_json::from_str(&run(
        &[
            "--provider",
            "openai-compatible",
            "--model",
            "fake-model",
            "run",
            "--json",
            "seed detached fixture",
        ],
        0,
    )?)?;
    let session = seed["session_id"]
        .as_str()
        .context("seed session missing")?;
    let receipt: Value = serde_json::from_str(&run(
        &[
            "cloud",
            "run-detached",
            "--session",
            session,
            "--",
            "[e2e:hold:detached] continue this task",
        ],
        0,
    )?)?;
    ensure!(
        receipt["session_id"] == session && receipt["state"] == "running",
        "invalid detached receipt: {receipt}"
    );
    let deadline = Instant::now() + Duration::from_secs(30);
    while !gates.join("detached.held").exists() {
        let report = inspect(session)?;
        ensure!(
            report["state"] == "running" && Instant::now() < deadline,
            "detached turn did not reach provider gate: {report}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    ensure!(
        inspect(session)?["state"] == "running",
        "detached turn stopped at gate"
    );
    let duplicate = run(
        &[
            "cloud",
            "run-detached",
            "--session",
            session,
            "--",
            "must not run concurrently",
        ],
        1,
    )?;
    ensure!(
        ["migration", "active", "owns"]
            .iter()
            .any(|s| duplicate.contains(s)),
        "unexpected busy rejection: {duplicate}"
    );
    fs::write(gates.join("detached.release"), "release\n")?;
    let deadline = Instant::now() + Duration::from_secs(30);
    let report = loop {
        let report: Value = serde_json::from_str(&run(
            &[
                "cloud",
                "inspect",
                "--session",
                session,
                "--messages",
                "4",
                "--log-lines",
                "20",
            ],
            0,
        )?)?;
        if report["state"] != "running" {
            break report;
        }
        ensure!(
            Instant::now() < deadline,
            "detached supervisor did not finish"
        );
        std::thread::sleep(Duration::from_millis(100));
    };
    ensure!(
        report["state"] == "idle" && report["exit_code"] == 0,
        "detached turn failed: {report}"
    );
    ensure!(
        report["log"]
            .as_str()
            .is_some_and(|s| s.contains("[detached] ack")),
        "missing provider acknowledgement"
    );
    ensure!(
        !fs::read_to_string(root.join("requests.jsonl"))?.contains("must not run concurrently"),
        "duplicate turn reached provider"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        ensure!(
            fs::metadata(nexode.join("cloud_runs").join(session).join("run.log"))?
                .permissions()
                .mode()
                & 0o077
                == 0,
            "run log is not private"
        );
    }
    println!(
        "PASS: real detached turn, busy rejection, supervisor completion, private logs, session inspection"
    );
    Ok(())
}
#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    #[test]
    fn cleanup_matches_exact_workspace_marker() {
        let t = tempfile::tempdir().unwrap();
        let mut owned = Command::new("sleep")
            .arg("60")
            .env("NEXODE_TEST_WORKSPACE", t.path())
            .spawn()
            .unwrap();
        let mut other = Command::new("sleep")
            .arg("60")
            .env(
                "NEXODE_TEST_WORKSPACE",
                format!("{}-other", t.path().display()),
            )
            .spawn()
            .unwrap();
        let result = cleanup_workspace(t.path());
        let still_running = other.try_wait().unwrap().is_none();
        let _ = other.kill();
        let _ = other.wait();
        let _ = owned.kill();
        let _ = owned.wait();
        result.unwrap();
        assert!(still_running);
    }
}
