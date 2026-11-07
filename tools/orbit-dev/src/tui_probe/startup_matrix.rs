//! Isolated startup matrix. Never edits the caller's header hint or transcripts.
use super::{debug_fixture::Fixture, live_duplicate, screen::Client};
use anyhow::{Context, Result, ensure};
use clap::Parser;
use regex::Regex;
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::{Duration, Instant},
};
#[derive(Parser)]
struct Args {
    binary: PathBuf,
    #[arg(default_value = "native")]
    label: String,
    #[arg(long, default_value = ".")]
    repo: PathBuf,
    #[arg(long)]
    output: Option<PathBuf>,
}
fn hint(path: &Path, case: &str) -> Result<()> {
    let mut v: Value =
        serde_json::from_slice(&fs::read(path).context("prime client did not write header hint")?)?;
    let object = v.as_object_mut().context("hint must be an object")?;
    match case {
        "cold-mcp-hint" => {
            object.insert("mcp_servers".into(), json!([]));
        }
        "no-session-facts" => {
            object.remove("session");
        }
        "mismatched-hint" => {
            object.insert("session".into(),json!({"provider_name":"OpenAI","provider_model":"gpt-5.5","context_limit":400000,"reasoning_effort":"high","resolved_credential":"oauth"}));
        }
        _ => anyhow::bail!("unknown hint mutation"),
    }
    fs::write(path, serde_json::to_vec(&v)?)?;
    Ok(())
}
fn overscroll(text: &str) -> Option<String> {
    let re = Regex::new(r"[0-9.]+[kM]/[0-9.]+[kM]").unwrap();
    text.lines()
        .rfind(|s| re.is_match(s))
        .map(|s| s.trim().to_owned())
}
fn inspect(command: &mut Command, cols: u16, cwd: &Path, out: &Path) -> Result<Value> {
    fs::create_dir_all(out)?;
    command.current_dir(cwd).env("TERM", "xterm-256color");
    let client = Client::spawn(command, 40, cols)?;
    let start = Instant::now();
    let mut frames = vec![];
    let mut previous = String::new();
    while start.elapsed() < Duration::from_secs(30) {
        let snapshot = client.snapshot()?;
        ensure!(!snapshot.eof, "startup client exited early");
        let text = snapshot.rows.join("\n");
        if text != previous {
            fs::write(
                out.join(format!(
                    "{:03}_{}ms.txt",
                    frames.len(),
                    start.elapsed().as_millis()
                )),
                &text,
            )?;
            frames.push(text.clone());
            previous = text;
        }
        if start.elapsed() >= Duration::from_secs(4)
            && overscroll(&previous).is_some()
            && !previous.to_lowercase().contains("connecting")
        {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }
    thread::sleep(Duration::from_secs(3));
    let idle = client.snapshot()?.rows.join("\n");
    client.send(b"hi")?;
    thread::sleep(Duration::from_millis(400));
    let typed = client.snapshot()?.rows.join("\n");
    client.send(b"\x7f\x7f/model")?;
    thread::sleep(Duration::from_millis(1500));
    let picker = client.snapshot()?.rows.join("\n");
    for (name, text) in [
        ("zz_idle", &idle),
        ("zz_typed", &typed),
        ("zz_model_picker", &picker),
    ] {
        fs::write(out.join(name), text)?;
    }
    let first = frames.iter().find_map(|s| overscroll(s));
    let final_line = overscroll(&idle);
    let had_overview = frames.iter().any(|s| s.contains("Overview"));
    let branch = Command::new("git")
        .args(["-C"])
        .arg(cwd)
        .args(["branch", "--show-current"])
        .output()?;
    let branch = if branch.status.success() {
        String::from_utf8(branch.stdout)?.trim().to_owned()
    } else {
        String::new()
    };
    let branch_ok = first.as_ref().is_some_and(|s| {
        if branch.is_empty() {
            !s.contains(" main ") && !s.contains(" master ")
        } else {
            s.contains(&format!(" {branch} "))
        }
    });
    let model_rows = picker
        .lines()
        .filter(|s| {
            ["Opus", "Sonnet", "GPT", "Haiku", "Fable"]
                .iter()
                .any(|x| s.contains(x))
        })
        .count();
    let more = Regex::new(r"\+(\d+) more")?
        .captures_iter(&picker)
        .last()
        .and_then(|c| c[1].parse::<usize>().ok())
        .unwrap_or(0);
    let checks = json!({
        "R1":first.is_some()&&first==final_line,
        "R2":first.as_ref().is_some_and(|s|!s.contains("/200k")&&!s.contains("Oauth:")),
        "R3":if had_overview{json!(frames.last().is_some_and(|s|s.contains("Overview")))}else{Value::Null},
        "R4":if had_overview{json!(idle.contains("Overview")&&typed.contains("Overview"))}else{Value::Null},
        "R5":branch_ok,"R6":model_rows>=3&&more>=10});
    let passed = checks.as_object().unwrap().values().all(|v| v != false);
    client.send(b"\x1b\x15/quit\r")?;
    Ok(
        json!({"passed":passed,"checks":checks,"first_header":first,"final_header":final_line,"frames":frames.len(),"model_rows":model_rows,"more_models":more,"artifacts":out}),
    )
}
struct SpawnRoot(tempfile::TempDir);
impl Drop for SpawnRoot {
    fn drop(&mut self) {
        if let Err(e) = crate::detached_acceptance::cleanup_workspace(self.0.path()) {
            eprintln!("startup cleanup: {e:#}");
        }
    }
}
fn timing(home: &Path) -> Result<Value> {
    let re = Regex::new(r"send_history prep: .*tool_names=(\d+)ms.*models=(\d+)ms")?;
    let mut models = vec![];
    let mut tools = vec![];
    let logs = home.join("logs");
    if logs.exists() {
        for e in fs::read_dir(logs)? {
            let p = e?.path();
            if p.is_file() {
                let text = fs::read_to_string(p).unwrap_or_default();
                for c in re.captures_iter(&text) {
                    models.push(c[2].parse::<u64>()?);
                    tools.push(c[1].parse::<u64>()?);
                }
            }
        }
    }
    let max = models.into_iter().max();
    Ok(
        json!({"passed":max.is_some_and(|n|n<=40),"models_max_ms":max,"tool_names_max_ms":tools.into_iter().max(),"budget_ms":40}),
    )
}
pub(super) fn run(args: impl Iterator<Item = String>) -> Result<i32> {
    let a = Args::try_parse_from(std::iter::once("startup-acceptance-matrix".into()).chain(args))?;
    let binary = a.binary.canonicalize()?;
    let repo = a.repo.canonicalize()?;
    let output = a.output.unwrap_or_else(|| {
        std::env::temp_dir().join(format!("orbit-startup-{}", std::process::id()))
    });
    ensure!(
        !output.exists(),
        "output already exists; choose an empty artifact directory"
    );
    fs::create_dir_all(&output)?;
    let fixture = Fixture::new(&binary)?;
    let hint_file = fixture.home().join("remote_header_hint.json");
    let mut reports = vec![];
    for (case, cols) in [
        ("prime", 140),
        ("warm-140", 140),
        ("warm-140-again", 140),
        ("narrow-80", 80),
        ("nogit-dir", 140),
        ("cold-mcp-hint", 140),
        ("no-session-facts", 140),
        ("mismatched-hint", 140),
        ("after-mismatch", 140),
        ("resumed-session", 140),
    ] {
        let result = (|| -> Result<Value> {
            if matches!(
                case,
                "cold-mcp-hint" | "no-session-facts" | "mismatched-hint"
            ) {
                hint(&hint_file, case)?;
            }
            let cwd = if case == "nogit-dir" {
                &fixture.workspace
            } else {
                &repo
            };
            let mut command = fixture.client_command(&binary);
            command
                .env("ANTHROPIC_API_KEY", "sk-ant-local-ui-fixture")
                .env("OPENAI_API_KEY", "sk-local-ui-fixture");
            if case == "resumed-session" {
                let (id, _) = live_duplicate::seed(&fixture.home(), &repo, 1)?;
                command.args(["--resume", &id]);
            }
            inspect(&mut command, cols, cwd, &output.join(case))
        })();
        reports.push(match result {
            Ok(mut r) => {
                r["case"] = json!(case);
                r
            }
            Err(e) => json!({"case":case,"passed":false,"error":format!("{e:#}")}),
        });
    }
    let spawn = SpawnRoot(
        tempfile::Builder::new()
            .prefix("orbit-startup-spawn-")
            .tempdir()?,
    );
    let home = spawn.0.path().join("home");
    let runtime = spawn.0.path().join("run");
    fs::create_dir(&home)?;
    fs::create_dir(&runtime)?;
    let mut command = Command::new(&binary);
    command
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", &home)
        .env("NEXODE_HOME", &home)
        .env("NEXODE_RUNTIME_DIR", &runtime)
        .env("NEXODE_TEST_WORKSPACE", spawn.0.path())
        .env("NEXODE_TEMP_SERVER", "1")
        .env("NEXODE_SERVER_OWNER_PID", std::process::id().to_string())
        .env("NEXODE_NO_TELEMETRY", "1")
        .env("ANTHROPIC_API_KEY", "sk-ant-local-ui-fixture")
        .env("ANTHROPIC_BASE_URL", "http://127.0.0.1:9")
        .env("OPENAI_BASE_URL", "http://127.0.0.1:9/v1")
        .args(["--no-update", "--no-selfdev", "--socket"])
        .arg(runtime.join("nexode.sock"));
    reports.push(
        match inspect(
            &mut command,
            140,
            &repo,
            &output.join("client-spawned-server"),
        ) {
            Ok(mut r) => {
                r["case"] = json!("client-spawned-server");
                r
            }
            Err(e) => {
                json!({"case":"client-spawned-server","passed":false,"error":format!("{e:#}")})
            }
        },
    );
    let server_timing = timing(&fixture.home())?;
    let logs = fixture.home().join("logs");
    if logs.is_dir() {
        fs::create_dir(output.join("server-logs"))?;
        for e in fs::read_dir(logs)? {
            let e = e?;
            if e.path().is_file() {
                fs::copy(e.path(), output.join("server-logs").join(e.file_name()))?;
            }
        }
    }
    let failed = reports.iter().any(|r| r["passed"] == false) || server_timing["passed"] == false;
    let report = json!({"label":a.label,"scenarios":reports,"R7":server_timing,"failed":failed});
    fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(i32::from(failed))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hint_edits_preserve_unrelated_fields() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("hint.json");
        fs::write(
            &p,
            r#"{"keep":42,"session":{"old":true},"mcp_servers":["one"]}"#,
        )
        .unwrap();
        for case in ["cold-mcp-hint", "no-session-facts", "mismatched-hint"] {
            hint(&p, case).unwrap();
            let v: Value = serde_json::from_slice(&fs::read(&p).unwrap()).unwrap();
            assert_eq!(v["keep"], 42);
        }
        assert_eq!(
            overscroll("header\n 10k/200k gauge \ncomposer"),
            Some("10k/200k gauge".into())
        );
        fs::create_dir(t.path().join("logs")).unwrap();
        fs::write(
            t.path().join("logs/runtime.log"),
            "[TIMING] send_history prep: tool_names=2ms, models=31ms, routes=0ms",
        )
        .unwrap();
        let report = timing(t.path()).unwrap();
        assert_eq!(report["models_max_ms"], 31);
        assert_eq!(report["tool_names_max_ms"], 2);
        assert_eq!(report["passed"], true);
    }
}
