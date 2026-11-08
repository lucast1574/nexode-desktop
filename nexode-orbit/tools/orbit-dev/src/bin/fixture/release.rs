use anyhow::{Result, ensure};
use serde_json::{Value, json};
use std::{fs, io::Write, path::PathBuf, process::Command};
pub fn run() -> Result<i32> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let config: Value = serde_json::from_slice(&fs::read(std::env::var("TEST_CI_CONFIG")?)?)?;
    let scenario = config["scenario"].as_str().unwrap();
    let mut log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(std::env::var("TEST_GH_LOG")?)?;
    writeln!(log, "{}", serde_json::to_string(&args)?)?;
    if args.starts_with(&["repo".into(), "view".into()]) {
        if scenario == "repo_error" {
            return Ok(1);
        }
        println!("owner/repo");
        return Ok(0);
    }
    if args[0] == "release" {
        if args[1] == "view" && args.contains(&"--json".into()) {
            println!("{}", scenario != "public_release");
        }
        return Ok(0);
    }
    ensure!(args[0] == "api", "unexpected gh command");
    if scenario == "api_error" {
        return Ok(1);
    }
    let endpoint = &args[1];
    let mut run = json!({"id":100,"run_number":10,"run_attempt":1,"head_sha":config["sha"],"event":"push","head_branch":"master","path":".github/workflows/ci.yml","status":"completed","conclusion":"success"});
    match scenario {
        "failure" => run["conclusion"] = json!("failure"),
        "pending" => {
            run["status"] = json!("in_progress");
            run["conclusion"] = Value::Null;
        }
        "other_sha" => run["head_sha"] = json!("b".repeat(40)),
        "pull_request" => run["event"] = json!("pull_request"),
        _ => {}
    }
    if endpoint.contains("/workflows/ci.yml/runs?") {
        println!(
            "{}",
            json!([{"workflow_runs":if scenario=="missing"{vec![]}else{vec![run]}}])
        );
    } else if endpoint.contains("/jobs?") {
        let mut jobs: Vec<_> = config["jobs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| json!({"name":n,"status":"completed","conclusion":"success"}))
            .collect();
        if scenario == "missing_job" {
            jobs.pop();
        } else if scenario == "failed_job" {
            jobs.last_mut().unwrap()["conclusion"] = json!("failure");
        }
        println!("{}", json!([{"jobs":jobs}]));
    } else {
        let marker = PathBuf::from(std::env::var("TEST_CONFIRMATION_MARKER")?);
        if marker.exists() {
            match scenario {
                "head_changed" => {
                    ensure!(
                        Command::new("git")
                            .args(["commit", "--allow-empty", "-m", "Concurrent change"])
                            .output()?
                            .status
                            .success(),
                        "git fixture failure"
                    );
                }
                "local_tag_changed" => {
                    ensure!(
                        Command::new("git")
                            .args(["tag", "-f", "v1.2.3", "HEAD^"])
                            .output()?
                            .status
                            .success(),
                        "git fixture failure"
                    );
                }
                "rerun" => run["run_attempt"] = json!(2),
                "dirty_during_ci" => {
                    writeln!(
                        fs::OpenOptions::new().append(true).open("Cargo.toml")?,
                        "# Concurrent source change"
                    )?;
                }
                _ => {}
            }
        }
        fs::write(marker, [])?;
        println!("{run}");
    }
    Ok(0)
}
