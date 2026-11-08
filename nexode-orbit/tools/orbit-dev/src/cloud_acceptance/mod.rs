//! Isolated cloud handoff gate, with retained JSON and JUnit evidence.
mod scenarios;
#[cfg(test)]
mod tests;
mod tui;
use anyhow::{Context, Result, ensure};
use clap::Parser;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};
type Environment = BTreeMap<String, String>;
#[derive(Parser)]
struct Args {
    #[arg(long)]
    binary: Option<PathBuf>,
    #[arg(long,num_args=1..,value_parser=["cli","agent","credentials","group","tui"],default_values=["cli","agent","credentials","group","tui"])]
    layers: Vec<String>,
    #[arg(long)]
    scenario: Option<String>,
    #[arg(long,default_value_t=1,value_parser=clap::value_parser!(u32).range(1..))]
    repeat: u32,
    #[arg(long)]
    artifacts: Option<PathBuf>,
    #[arg(long,default_value_t=240,value_parser=clap::value_parser!(u64).range(1..))]
    timeout: u64,
}
fn executable(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|dir| dir.join(name))
        .find(|path| {
            fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        })
}
fn sshd() -> Option<PathBuf> {
    executable("sshd").or_else(|| {
        ["/usr/sbin/sshd", "/usr/bin/sshd"]
            .into_iter()
            .map(PathBuf::from)
            .find(|p| p.is_file())
    })
}
fn checked(command: &mut Command, timeout: Duration) -> Result<String> {
    let out = crate::process::capture_interruptible(command, timeout, || {
        crate::ci_suites::unix::interrupted().is_some()
    })?;
    ensure!(
        out.status.success(),
        "command failed ({}): {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    Ok(String::from_utf8(out.stdout)?)
}
fn preflight(layers: &[String]) -> Result<()> {
    let mut required = vec!["bash", "git"];
    let shell = layers
        .iter()
        .any(|l| matches!(l.as_str(), "cli" | "agent" | "group"));
    if shell {
        required.push("bwrap");
    }
    if layers.iter().any(|l| l == "group") {
        required.extend(["setsid", "rg", "timeout"]);
    }
    let tui = layers.iter().any(|l| l == "tui");
    if tui {
        required.extend(["ssh", "ssh-keygen", "tmux", "unshare", "mount"]);
    }
    let mut missing: Vec<_> = required
        .into_iter()
        .filter(|name| executable(name).is_none())
        .collect();
    if tui && sshd().is_none() {
        missing.push("sshd");
    }
    ensure!(
        missing.is_empty(),
        "Missing test prerequisites: {}",
        missing.join(", ")
    );
    ensure!(
        unsafe { libc::geteuid() } != 0,
        "Run cloud acceptance as an ordinary user, not root"
    );
    if tui {
        checked(
            Command::new("unshare").args(["--user", "--map-root-user", "--mount", "true"]),
            Duration::from_secs(10),
        )
        .context("Isolation unavailable")?;
    }
    if shell {
        checked(
            Command::new("bwrap").args([
                "--ro-bind",
                "/",
                "/",
                "--unshare-pid",
                "--proc",
                "/proc",
                "true",
            ]),
            Duration::from_secs(10),
        )
        .context("Isolation unavailable")?;
    }
    Ok(())
}
fn child(command: &mut Command, logfile: &Path, timeout: Duration) -> Result<()> {
    let log = fs::File::create(logfile)?;
    command
        .stdin(std::process::Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log);
    let code = crate::ci_suites::unix::execute(command, timeout)?;
    ensure!(
        code != 124,
        "Stage timed out after {}s. See {}",
        timeout.as_secs_f64(),
        logfile.display()
    );
    ensure!(code == 0, "Stage exited {code}. See {}", logfile.display());
    Ok(())
}
fn xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
fn write_report(path: &Path, results: &[Value], binary: &Path, version: &str) -> Result<()> {
    fs::write(
        path.join("report.json"),
        format!(
            "{}\n",
            serde_json::to_string_pretty(
                &json!({"binary":binary,"version":version,"results":results,"limitations":["No hosted VM provisioning/suspend/resume or production OAuth service is exercised.","The provider is deterministic; SSH, TUI, daemons and transfer code are real."]})
            )?
        ),
    )?;
    let failures = results.iter().filter(|r| r["status"] != "passed").count();
    let mut report = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<testsuite name=\"cloud-acceptance\" tests=\"{}\" failures=\"{failures}\">",
        results.len()
    );
    for result in results {
        report += &format!(
            "<testcase name=\"{}\" time=\"{}\">",
            xml(result["name"].as_str().unwrap_or("")),
            result["seconds"]
        );
        if result["status"] != "passed" {
            report += &format!(
                "<failure message=\"{}\">{}</failure>",
                xml(result["error"].as_str().unwrap_or("")),
                xml(result["traceback"].as_str().unwrap_or(""))
            );
        }
        report += &format!(
            "<system-out>{}</system-out></testcase>",
            xml(result["artifacts"].as_str().unwrap_or(""))
        );
    }
    report += "</testsuite>\n";
    fs::write(path.join("junit.xml"), report)?;
    Ok(())
}
pub fn run(root: &Path, args: impl Iterator<Item = String>) -> Result<i32> {
    let args = Args::parse_from(std::iter::once("test-cloud-acceptance".into()).chain(args));
    let _signals = crate::ci_suites::unix::Signals::install()?;
    if let Some(name) = &args.scenario {
        ensure!(
            scenarios::NAMES.contains(&name.as_str()),
            "unknown TUI scenario {name}"
        );
    }
    let binary = args
        .binary
        .or_else(|| std::env::var_os("NEXODE_BIN").map(PathBuf::from))
        .context("--binary or NEXODE_BIN required")?
        .canonicalize()?;
    let artifacts = if let Some(path) = args.artifacts {
        ensure!(!path.exists(), "artifact directory must be new");
        fs::create_dir_all(&path)?;
        path.canonicalize()?
    } else {
        tempfile::Builder::new()
            .prefix("cloud-acceptance-")
            .tempdir_in(
                std::env::var_os("NEXODE_SCRATCH_DIR")
                    .map(PathBuf::from)
                    .unwrap_or(std::env::temp_dir()),
            )?
            .keep()
    };
    fs::set_permissions(&artifacts, fs::Permissions::from_mode(0o700))?;
    let version = checked(
        Command::new(&binary).args(["--no-update", "--version"]),
        Duration::from_secs(10),
    )?;
    println!(
        "Testing {}: {}\nArtifacts: {}",
        version.trim(),
        binary.display(),
        artifacts.display()
    );
    let mut results = vec![];
    if let Err(error) = preflight(&args.layers) {
        results.push(json!({"name":"preflight","status":"failed","seconds":0,"error":format!("{error:#}"),"artifacts":artifacts}));
        write_report(&artifacts, &results, &binary, version.trim())?;
        eprintln!("FAIL preflight: {error:#}");
        return Ok(1);
    }
    let mut stages: Vec<_> = args
        .layers
        .iter()
        .filter(|l| l.as_str() != "tui")
        .map(|l| (l.as_str(), None))
        .collect();
    if args.layers.iter().any(|l| l == "tui") {
        for name in scenarios::NAMES {
            if args.scenario.as_deref().is_none_or(|s| s == *name) {
                stages.push(("tui", Some(*name)));
            }
        }
    }
    for iteration in 1..=args.repeat {
        for (layer, scenario) in &stages {
            ensure!(
                crate::ci_suites::unix::interrupted().is_none(),
                "acceptance interrupted"
            );
            let name = format!("{layer}.{}.{iteration}", scenario.unwrap_or("suite"));
            let directory = artifacts.join(&name);
            let started = Instant::now();
            println!("RUN {name}");
            let outcome = (|| -> Result<()> {
                if let Some(scenario) = scenario {
                    let mut fixture = tui::CloudTui::new(&binary, &directory)?;
                    let run = fixture
                        .start()
                        .and_then(|_| scenarios::run(scenario, &mut fixture));
                    let cleanup = fixture.close();
                    match (run, cleanup) {
                        (Ok(()), Ok(())) => Ok(()),
                        (Err(e), Ok(())) => Err(e),
                        (Ok(()), Err(e)) => Err(e).context("Cleanup failed"),
                        (Err(e), Err(cleanup)) => {
                            Err(e).context(format!("Cleanup failed: {cleanup:#}"))
                        }
                    }
                } else {
                    fs::create_dir(&directory)?;
                    let env = Environment::from([
                        (
                            "PATH".into(),
                            std::env::var("PATH").unwrap_or("/usr/bin:/bin".into()),
                        ),
                        ("HOME".into(), directory.to_string_lossy().into_owned()),
                        ("LANG".into(), "C.UTF-8".into()),
                        ("NEXODE_BIN".into(), binary.to_string_lossy().into_owned()),
                        (
                            "NEXODE_SCRATCH_DIR".into(),
                            directory.to_string_lossy().into_owned(),
                        ),
                        ("TMPDIR".into(), directory.to_string_lossy().into_owned()),
                        ("NEXODE_NO_AUTO_UPDATE".into(), "1".into()),
                        ("NEXODE_NO_TELEMETRY".into(), "1".into()),
                    ]);
                    let mut command = if *layer == "credentials" {
                        let mut c = Command::new(std::env::current_exe()?);
                        c.arg("test-cloud-credentials").arg(&binary);
                        c
                    } else {
                        let script = match *layer {
                            "cli" => "test_cloud_move_e2e.sh",
                            "agent" => "test_cloud_move_agent_e2e.sh",
                            "group" => "test_cloud_group_e2e.sh",
                            _ => unreachable!(),
                        };
                        let mut c = Command::new("bash");
                        c.arg(root.join("scripts").join(script));
                        c
                    };
                    command
                        .env_clear()
                        .envs(env)
                        .env("ORBIT_DEV_BIN", std::env::current_exe()?)
                        .current_dir(root);
                    child(
                        &mut command,
                        &directory.join("output.log"),
                        Duration::from_secs(args.timeout),
                    )
                }
            })();
            let mut record = json!({"name":name,"artifacts":directory,"status":"passed","seconds":(started.elapsed().as_secs_f64()*1000.).round()/1000.});
            if let Err(error) = outcome {
                record["status"] = json!("failed");
                record["error"] = json!(format!("{error:#}"));
                record["traceback"] = json!(format!("{error:?}"));
                eprintln!("FAIL {name}: {error:#}");
            } else {
                println!("PASS {name} ({}s)", record["seconds"]);
            }
            results.push(record);
            write_report(&artifacts, &results, &binary, version.trim())?;
        }
    }
    let failures = results.iter().filter(|r| r["status"] != "passed").count();
    println!(
        "{} passed, {failures} failed. Report: {}",
        results.len() - failures,
        artifacts.join("report.json").display()
    );
    Ok(i32::from(failures > 0))
}
