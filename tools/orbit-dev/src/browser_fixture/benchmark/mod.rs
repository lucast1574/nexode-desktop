//! Paired browser benchmark. Offline self-test never launches inference/browser.
mod fixture;
mod tests;
mod trace;
use anyhow::{Context, Result, ensure};
use clap::Parser;
use serde_json::{Value, json};
use std::{
    fs::File,
    io::Write,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
#[derive(Parser)]
struct Args {
    #[arg(long)]
    binary: Option<PathBuf>,
    #[arg(long)]
    model: Option<String>,
    #[arg(long)]
    nexode_home: Option<PathBuf>,
    #[arg(long)]
    provider: Option<String>,
    #[arg(long,default_value="nexode",value_parser=["nexode","openrouter"])]
    expected_handoff_provider: String,
    #[arg(long)]
    tab_id: Option<u64>,
    #[arg(long, default_value_t = 3)]
    trials: usize,
    #[arg(long,num_args=1..,default_values=["navigation","search","form"],value_parser=["navigation","search","form"])]
    tasks: Vec<String>,
    #[arg(long,default_value="all",value_parser=["discovery","heldout","all"])]
    phase: String,
    #[arg(long,default_value="jev",value_parser=["jev","normal"])]
    arm: String,
    #[arg(long, default_value_t = 240.)]
    timeout: f64,
    #[arg(long)]
    output: Option<PathBuf>,
    #[arg(long)]
    self_test: bool,
}
fn bridge(home: &Path, tab: u64, action: &str, mut params: Value) -> Result<Value> {
    params["tabId"] = json!(tab);
    let mut c = Command::new(home.join("browser/browser"));
    c.arg(action)
        .arg(params.to_string())
        .env("NEXODE_HOME", home);
    let out = crate::process::capture(&mut c, Duration::from_secs(20))?;
    ensure!(out.status.success(), "browser bridge {action} failed");
    Ok(serde_json::from_slice(&out.stdout)?)
}
fn guard(home: &Path, tab: u64) -> Result<File> {
    let lock = super::live::tab_lock(tab)?;
    let listing = bridge(home, tab, "listTabs", json!({}))?;
    let found = listing["windows"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|w| w["tabs"].as_array().into_iter().flatten())
        .find(|t| t["tabId"] == tab)
        .context("designated tab not found")?;
    let url = url::Url::parse(found["url"].as_str().context("tab URL")?)?;
    ensure!(
        matches!(url.scheme(), "http" | "https")
            && matches!(
                url.host_str(),
                Some("127.0.0.1" | "localhost" | "::1" | "[::1]")
            )
            && matches!(
                found["title"].as_str(),
                Some("Nexode isolated browser fixture" | "Jev hybrid verified")
            ),
        "refusing non-loopback/non-fixture tab"
    );
    Ok(lock)
}
fn configure(c: &mut Command, home: &Path, runtime: &Path, mode: &str) {
    c.env_remove("NEXODE_SOCKET")
        .env_remove("NEXODE_SESSION_ID")
        .env_remove("NEXODE_PARENT_SESSION_ID")
        .env("NEXODE_HOME", home)
        .env("NEXODE_RUNTIME_DIR", runtime)
        .env(
            "NEXODE_BROWSER_HANDOFF_DISABLED",
            if mode == "direct" { "1" } else { "0" },
        )
        .env("NEXODE_DEBUG_CONTROL", "1");
}
struct Owned(Child);
impl Drop for Owned {
    fn drop(&mut self) {
        unsafe {
            libc::kill(-(self.0.id() as i32), libc::SIGKILL);
        }
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn spawn(c: &mut Command) -> Result<Owned> {
    use std::os::unix::process::CommandExt;
    c.process_group(0);
    Ok(Owned(c.spawn()?))
}
fn daemon(binary: &Path, home: &Path, runtime: &Path, mode: &str, log: &Path) -> Result<Owned> {
    let socket = runtime.join("server.sock");
    let file = File::create(log)?;
    let mut c = Command::new(binary);
    configure(&mut c, home, runtime, mode);
    c.args(["--no-update", "--no-selfdev", "--socket"])
        .arg(&socket)
        .args(["serve", "--server-name", "browser-handoff-benchmark"])
        .stdin(Stdio::null())
        .stdout(file.try_clone()?)
        .stderr(file);
    let mut child = spawn(&mut c)?;
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(30) {
        let mut probe = Command::new(binary);
        configure(&mut probe, home, runtime, mode);
        probe
            .args(["--no-update", "--no-selfdev", "--socket"])
            .arg(&socket)
            .args(["debug", "server:info"]);
        if crate::process::capture(&mut probe, Duration::from_secs(2))
            .is_ok_and(|o| o.status.success())
        {
            return Ok(child);
        }
        ensure!(
            child.0.try_wait()?.is_none(),
            "isolated daemon exited; inspect {}",
            log.display()
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    anyhow::bail!(
        "isolated daemon readiness timeout; inspect {}",
        log.display()
    )
}
#[expect(
    clippy::too_many_arguments,
    reason = "paired trial carries explicit isolated runtime and browser ownership"
)]
fn trial(
    a: &Args,
    f: &fixture::Fixture,
    binary: &Path,
    home: &Path,
    runtime: &Path,
    output: &Path,
    index: usize,
    mode: &str,
    task: &str,
    phase: &str,
) -> Result<Value> {
    let (token, url) = f.add(task, phase)?;
    let trial = output.join(format!("{index:02}-{mode}"));
    std::fs::create_dir(&trial)?;
    let workspace = trial.join("workspace");
    std::fs::create_dir(&workspace)?;
    let tab = a.tab_id.unwrap();
    let mut prompt = format!(
        "Use the browser in tab {tab} to visit {url}. {} Tell me the receipt displayed on the final page and leave that page open. Use only the browser tool for this task. Stay in this tab.",
        fixture::prompt(task, phase)
    );
    if mode == "direct" {
        prompt.push_str(" Do not use browser action=\"handoff\". Complete the task using direct browser actions only.");
    } else if mode == "jev" {
        prompt.push_str(" Delegate the whole browser task to Jev using browser action=\"handoff\", with this tab and all supplied task text.");
    }
    std::fs::write(trial.join("prompt.txt"), &prompt)?;
    let mut cmd = Command::new(binary);
    configure(&mut cmd, home, runtime, mode);
    cmd.args(["--no-update", "--no-selfdev", "--socket"])
        .arg(runtime.join("server.sock"))
        .arg("--model")
        .arg(a.model.as_ref().unwrap())
        .arg("-C")
        .arg(&workspace);
    if let Some(p) = &a.provider {
        cmd.args(["--provider", p]);
    }
    cmd.args(["run", "--ndjson", &prompt])
        .stdin(Stdio::null())
        .stdout(File::create(trial.join("transcript.ndjson"))?)
        .stderr(File::create(trial.join("stderr.log"))?);
    let start = Instant::now();
    let relative_start = f.start.elapsed().as_secs_f64();
    let mut child = spawn(&mut cmd)?;
    let mut timed_out = false;
    let status = loop {
        if let Some(s) = child.0.try_wait()? {
            break s;
        }
        if start.elapsed().as_secs_f64() >= a.timeout {
            timed_out = true;
            unsafe {
                libc::kill(-(child.0.id() as i32), libc::SIGKILL);
            }
            break child.0.wait()?;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let elapsed = start.elapsed().as_secs_f64();
    let text = std::fs::read_to_string(trial.join("transcript.ndjson"))?;
    let mut trace = trace::extract(&text);
    let state = f.state(&token);
    let calls = trace["browser_calls"].as_array().unwrap();
    let actions: Vec<_> = calls
        .iter()
        .filter(|c| c["executed"] == true)
        .map(|c| c["action"].clone())
        .collect();
    let unknown = actions.iter().any(Value::is_null);
    let used = actions.contains(&json!("handoff"));
    let providers: Vec<_> = calls
        .iter()
        .filter(|c| c["executed"] == true && c["action"] == "handoff")
        .map(|c| c["decision_provider"].clone())
        .collect();
    let metrics = trace::handoff(&trace);
    let expected = url.rsplit_once('/').unwrap().0.to_string() + "/receipt";
    let script = format!(
        "return {{correct: location.href === {} && document.body.innerText.includes({}) && document.querySelector(\"#result\")?.textContent === \"Navigation complete\"}};",
        json!(expected),
        state["receipt"]
    );
    let probe = bridge(home, tab, "evaluate", json!({"frameId":0,"script":script}));
    let dom = probe.as_ref().is_ok_and(|v| v["result"]["correct"] == true);
    let server = fixture::outcome(&state);
    let correct = server && state["confirmation_dom_observed"] == true && dom;
    let receipt = trace["final_text"]
        .as_str()
        .unwrap_or("")
        .contains(state["receipt"].as_str().unwrap());
    trace.as_object_mut().unwrap().remove("final_text");
    let compliant = (mode != "jev" || metrics["handoff_effective"] == true)
        && providers.iter().all(|p| p == &a.expected_handoff_provider)
        && trace["other_tools"].as_array().unwrap().is_empty()
        && !unknown
        && (mode != "direct" || !used);
    use std::os::unix::process::ExitStatusExt;
    let mut result = json!({"type":"trial","pair":index,"mode":mode,"task":task,"phase":phase,"model":a.model,"direct_guard_requested":mode=="direct","parent_tool_count":trace["parent_tool_count"],"model_call_count":trace["model_call_count"],"usage_tokens":trace["usage_tokens"],"usage_tokens_scope":trace["usage_tokens_scope"],"actual_cost_usd":trace["actual_cost_usd"],"elapsed_seconds":elapsed,"exit_code":status.code().or_else(||status.signal().map(|n|-n)),"timed_out":timed_out,"handoff_used":used,"decision_providers":providers,"expected_handoff_provider":a.expected_handoff_provider,"actions":actions,"trace_complete":!unknown&&!actions.is_empty(),"correct_page_state":correct,"server_outcome_correct":server,"final_dom_correct":dom,"validation_error":probe.err().map(|_|"Independent scoped DOM probe failed"),"receipt_correct":receipt,"protocol_compliant":compliant,"valid_success":correct&&receipt&&compliant&&!actions.is_empty()&&!timed_out&&status.success()&&trace["errors"].as_array().unwrap().is_empty(),"fixture":state,"trace":trace,"seconds_to_confirmation":state["visible_at"].as_f64().map(|n|n-relative_start)});
    for (k, v) in metrics.as_object().unwrap() {
        result[k] = v.clone();
    }
    std::fs::write(
        trial.join("result.json"),
        serde_json::to_vec_pretty(&result)?,
    )?;
    Ok(result)
}
pub fn run(args: impl Iterator<Item = String>) -> Result<i32> {
    let a = Args::try_parse_from(std::iter::once("benchmark-browser-handoff".into()).chain(args))?;
    if a.self_test {
        tests::self_test()?;
        println!(
            "{}",
            json!({"type":"self_test","passed":true,"live_browser_or_nexode_launched":false})
        );
        return Ok(0);
    }
    let binary = a
        .binary
        .as_ref()
        .context("--binary required")?
        .canonicalize()?;
    let home = a
        .nexode_home
        .as_ref()
        .context("--nexode-home required")?
        .canonicalize()?;
    let output = a.output.as_ref().context("--output required")?;
    let tab = a.tab_id.context("--tab-id required")?;
    ensure!(
        a.model.is_some() && tab > 0 && a.trials > 0 && a.timeout.is_finite() && a.timeout > 0.,
        "model, positive tab/trials/timeout required"
    );
    ensure!(
        home.is_dir()
            && home
                != dirs::home_dir()
                    .context("user home")?
                    .join(".nexode")
                    .canonicalize()
                    .unwrap_or_default(),
        "must use prepared isolated home"
    );
    ensure!(
        !std::env::var("BROWSER_SESSION")
            .unwrap_or_default()
            .trim()
            .is_empty(),
        "existing BROWSER_SESSION required"
    );
    let _lock = guard(&home, tab)?;
    ensure!(!output.exists(), "output directory already exists");
    std::fs::create_dir_all(output)?;
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(output, std::fs::Permissions::from_mode(0o700))?;
    let output = output.canonicalize()?;
    use sha2::{Digest, Sha256};
    let mut digest = Sha256::new();
    std::io::copy(&mut File::open(&binary)?, &mut digest)?;
    let metadata = json!({"binary":binary,"binary_sha256":format!("{:x}",digest.finalize()),"model":a.model,"provider":a.provider,"tab_id":tab,"pairs_per_task_phase":a.trials,"tasks":a.tasks,"phase":a.phase,"arm":a.arm,"expected_handoff_provider":a.expected_handoff_provider,"timing":"run process start through process exit, daemon startup excluded"});
    std::fs::write(
        output.join("metadata.json"),
        serde_json::to_vec_pretty(&metadata)?,
    )?;
    let runtime = tempfile::Builder::new().prefix("jbh-").tempdir()?;
    let fixture = fixture::Fixture::new()?;
    let mut results = vec![];
    let mut pair = 0;
    let phases = if a.phase == "all" {
        vec!["discovery", "heldout"]
    } else {
        vec![a.phase.as_str()]
    };
    let mut metrics = File::create(output.join("metrics.ndjson"))?;
    for phase in phases {
        for task in &a.tasks {
            for _ in 0..a.trials {
                pair += 1;
                let modes = if pair % 2 == 1 {
                    [a.arm.as_str(), "direct"]
                } else {
                    ["direct", a.arm.as_str()]
                };
                for mode in modes {
                    let rt = runtime.path().join(format!("{pair}-{mode}"));
                    std::fs::create_dir(&rt)?;
                    let start = Instant::now();
                    let result = (|| {
                        let _daemon = daemon(
                            &binary,
                            &home,
                            &rt,
                            mode,
                            &output.join(format!("{pair:02}-{mode}-server.log")),
                        )?;
                        trial(
                            &a, &fixture, &binary, &home, &rt, &output, pair, mode, task, phase,
                        )
                    })();
                    let result=result.unwrap_or_else(|e|json!({"type":"trial","pair":pair,"mode":mode,"task":task,"phase":phase,"valid_success":false,"handoff_used":false,"handoff_effective":false,"protocol_compliant":false,"timed_out":e.to_string().contains("timed out"),"elapsed_seconds":start.elapsed().as_secs_f64(),"infrastructure_error":format!("{e:#}"),"timing_scope":"infrastructure failure including daemon startup"}));
                    println!("{result}");
                    writeln!(metrics, "{result}")?;
                    metrics.flush()?;
                    results.push(result);
                }
            }
        }
    }
    let summary = trace::summary(&results);
    println!("{summary}");
    writeln!(metrics, "{summary}")?;
    std::fs::write(
        output.join("summary.json"),
        serde_json::to_vec_pretty(&summary)?,
    )?;
    Ok(i32::from(
        results.iter().any(|r| r["valid_success"] != true),
    ))
}
