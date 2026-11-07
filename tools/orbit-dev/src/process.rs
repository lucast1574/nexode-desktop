use anyhow::{Context, Result, ensure};
use std::{
    fs::OpenOptions,
    io::Write,
    path::Path,
    process::{Child, Command},
    time::{Duration, Instant},
};
#[cfg(unix)]
fn kill_tree(child: &mut Child) {
    // Children inherit the process group created before exec.
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    let _ = child.kill();
}
#[cfg(windows)]
fn kill_tree(child: &mut Child) {
    let _ = Command::new("taskkill.exe")
        .args(["/PID", &child.id().to_string(), "/T", "/F"])
        .status();
    let _ = child.kill();
}
pub fn run_with_timeout(seconds: u64, argv: Vec<String>) -> Result<i32> {
    ensure!(
        seconds > 0 && !argv.is_empty(),
        "usage: cargo xtask timeout <positive-seconds> <command> [args...]"
    );
    let deadline = Instant::now()
        .checked_add(Duration::from_secs(seconds))
        .context("timeout overflow")?;
    let mut command = Command::new(&argv[0]);
    command.args(&argv[1..]);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command
        .spawn()
        .with_context(|| format!("launch {}", argv[0]))?;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                #[cfg(unix)]
                {
                    use std::os::unix::process::ExitStatusExt;
                    return Ok(status
                        .code()
                        .unwrap_or_else(|| 128 + status.signal().unwrap_or(1)));
                }
                #[cfg(not(unix))]
                {
                    return Ok(status.code().unwrap_or(1));
                }
            }
            Ok(None) => {}
            Err(error) => {
                kill_tree(&mut child);
                let _ = child.wait();
                return Err(error.into());
            }
        }
        if Instant::now() >= deadline {
            eprintln!(
                "::error::Command exceeded timeout after {seconds}s: {}",
                argv[0]
            );
            kill_tree(&mut child);
            let _ = child.wait();
            return Ok(124);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
pub fn log_build(path: &Path, argv: Vec<String>) -> Result<()> {
    let env = |name: &str| {
        std::env::var(format!("NEXODE_LOG_{name}"))
            .with_context(|| format!("missing NEXODE_LOG_{name}"))
    };
    let duration = env("DURATION_MS")?.parse::<u64>()?;
    let gate = env("GATE_WAIT_MS")?.parse::<u64>()?;
    let code = env("EXIT_CODE")?.parse::<i32>()?;
    let record = serde_json::json!({"started_at":env("STARTED_AT")?,"duration_ms":duration,"gate_wait_ms":gate,
        "execution_duration_ms":duration.saturating_sub(gate),"exit_code":code,"success":code==0,
        "action":env("ACTION")?,"profile":env("PROFILE")?,"repository":env("REPO")?,"execution":env("EXECUTION")?,"argv":argv});
    append_record(path, &record)
}
fn append_record(path: &Path, record: &serde_json::Value) -> Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    let mut line = serde_json::to_vec(record)?;
    line.push(b'\n');
    file.write_all(&line)?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    #[test]
    fn preserves_exit_code_and_bounds_runtime() {
        assert_eq!(
            run_with_timeout(2, vec!["sh".into(), "-c".into(), "exit 17".into()]).unwrap(),
            17
        );
        let start = Instant::now();
        assert_eq!(
            run_with_timeout(1, vec!["sh".into(), "-c".into(), "sleep 60 & wait".into()]).unwrap(),
            124
        );
        assert!(start.elapsed() < Duration::from_secs(4));
        assert!(run_with_timeout(0, vec!["sh".into()]).is_err());
        assert!(run_with_timeout(2, vec!["/definitely-missing-orbit-command".into()]).is_err());
    }
    #[test]
    fn appends_json_lines_and_protects_new_file() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("build.jsonl");
        let record = serde_json::json!({"argv":["a\nb","x\"y"]});
        append_record(&path, &record).unwrap();
        append_record(&path, &record).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.lines().count(), 2);
        for line in text.lines() {
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(line).unwrap(),
                record
            );
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
}

/// Run a configured command without pipe deadlocks, preserving both output
/// streams and bounding runtime. Temporary spool files never reach the repo.
pub fn capture(command: &mut Command, timeout: Duration) -> Result<std::process::Output> {
    capture_interruptible(command, timeout, || false)
}
pub fn capture_interruptible(
    command: &mut Command,
    timeout: Duration,
    interrupted: impl Fn() -> bool,
) -> Result<std::process::Output> {
    use std::io::{Read, Seek, SeekFrom};
    use std::process::Stdio;
    let mut stdout = tempfile::tempfile()?;
    let mut stderr = tempfile::tempfile()?;
    command
        .stdin(Stdio::null())
        .stdout(stdout.try_clone()?)
        .stderr(stderr.try_clone()?);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command.spawn().context("start bounded child process")?;
    let deadline = Instant::now()
        .checked_add(timeout)
        .context("timeout overflow")?;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(error) => {
                kill_tree(&mut child);
                let _ = child.wait();
                return Err(error.into());
            }
        }
        if interrupted() {
            kill_tree(&mut child);
            let _ = child.wait();
            anyhow::bail!("command interrupted");
        }
        if Instant::now() >= deadline {
            kill_tree(&mut child);
            let _ = child.wait();
            anyhow::bail!("command timed out after {:.1}s", timeout.as_secs_f64());
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    ensure!(
        stdout.metadata()?.len() <= 64 * 1024 * 1024
            && stderr.metadata()?.len() <= 64 * 1024 * 1024,
        "command output exceeded 64 MiB"
    );
    stdout.seek(SeekFrom::Start(0))?;
    stderr.seek(SeekFrom::Start(0))?;
    let mut out = Vec::new();
    let mut err = Vec::new();
    stdout.read_to_end(&mut out)?;
    stderr.read_to_end(&mut err)?;
    Ok(std::process::Output {
        status,
        stdout: out,
        stderr: err,
    })
}

#[cfg(all(test, unix))]
mod capture_tests {
    use super::*;
    #[test]
    fn captures_both_streams_and_preserves_failure() {
        let mut c = Command::new("sh");
        c.args(["-c", "printf evidence; printf diagnostic >&2; exit 7"]);
        let output = capture(&mut c, Duration::from_secs(2)).unwrap();
        assert_eq!(output.status.code(), Some(7));
        assert_eq!(output.stdout, b"evidence");
        assert_eq!(output.stderr, b"diagnostic");
    }
    #[test]
    fn capture_timeout_is_an_error_and_kills_process_group() {
        let mut c = Command::new("sh");
        c.args(["-c", "sleep 60 & wait"]);
        let start = Instant::now();
        assert!(
            capture(&mut c, Duration::from_millis(100))
                .unwrap_err()
                .to_string()
                .contains("timed out")
        );
        assert!(start.elapsed() < Duration::from_secs(3));
    }
}

#[cfg(all(test, unix))]
mod interrupt_tests {
    use super::*;
    #[test]
    fn cancellation_stops_a_running_child_before_its_timeout() {
        let start = Instant::now();
        let mut c = Command::new("sh");
        c.args(["-c", "sleep 60 & wait"]);
        let error = capture_interruptible(&mut c, Duration::from_secs(60), || {
            start.elapsed() > Duration::from_millis(50)
        })
        .unwrap_err();
        assert!(error.to_string().contains("interrupted"));
        assert!(start.elapsed() < Duration::from_secs(2));
    }
}
