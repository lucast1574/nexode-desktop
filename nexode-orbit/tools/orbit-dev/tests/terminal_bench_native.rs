#![cfg(target_os = "linux")]
use serde_json::Value;
use std::os::unix::fs::PermissionsExt;
use std::{fs, path::Path, process::Command};
fn tool() -> Command {
    Command::new(env!("CARGO_BIN_EXE_orbit-dev"))
}
fn fixture(root: &Path, image: &str) -> std::path::PathBuf {
    let task = root.join("task-one");
    fs::create_dir_all(task.join("environment")).unwrap();
    fs::create_dir_all(task.join("tests")).unwrap();
    fs::write(task.join("task.toml"),format!("version = '1.0'\n[agent]\nuser = '65534'\ntimeout_sec = 15.0\n[verifier]\ntimeout_sec = 10.0\n[environment]\ndocker_image = '{image}'\ncpus = 1\nmemory_mb = 256\nnetwork_mode = 'none'\n")).unwrap();
    fs::write(
        task.join("instruction.md"),
        "Preserve literal text: $(touch /tmp/injected) ' \"\nline two",
    )
    .unwrap();
    fs::write(task.join("tests/test.sh"),"#!/bin/sh\nset -eu\ntest -f /tmp/native-benchmark-answer\ntest ! -e /tmp/injected\ntest -z \"${ANTHROPIC_API_KEY:-}\"\nprintf '{\"reward\":1}' > /logs/verifier/reward.json\n").unwrap();
    task
}
#[test]
fn dry_run_is_native_and_rejects_unknown_task_semantics() {
    let r = tempfile::tempdir().unwrap();
    let task = fixture(r.path(), "unused");
    let o = tool()
        .args(["terminal-bench", "--dry-run", "--path"])
        .arg(&task)
        .env("PATH", "/nonexistent")
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["inference"], false);
    fs::write(task.join("task.toml"), "steps = [{name = 'one'}]").unwrap();
    assert!(
        !tool()
            .args(["terminal-bench", "--dry-run", "--path"])
            .arg(&task)
            .status()
            .unwrap()
            .success()
    );
}
#[test]
fn real_docker_agent_verifier_and_campaign_resume() {
    let Ok(image) = std::env::var("ORBIT_TEST_DOCKER_IMAGE") else {
        eprintln!(
            "SKIP real Docker test: set ORBIT_TEST_DOCKER_IMAGE to a local Linux image with sh/bash"
        );
        return;
    };
    let r = tempfile::tempdir().unwrap();
    let task = fixture(r.path(), &image);
    let binary = r.path().join("fake-nexode");
    fs::write(&binary,r#"#!/bin/sh
set -eu
case " $* " in *" --version "*|*" version "*) printf '{"version":"fixture"}\n'; exit 0;; esac
test "$NEXODE_PROVIDER" = anthropic-api
test "$ANTHROPIC_API_KEY" = synthetic-benchmark-key
for arg in "$@"; do last=$arg; done
printf '%s' "$last" > /tmp/native-benchmark-answer
printf '{"type":"done","text":"fixture done","usage":{"input_tokens":7,"output_tokens":3,"cache_read_input_tokens":2,"cache_creation_input_tokens":1}}\n'
"#).unwrap();
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).unwrap();
    let jobs = r.path().join("jobs");
    let out = tool()
        .args(["terminal-bench", "--path"])
        .arg(&task)
        .args(["--binary"])
        .arg(&binary)
        .args(["--model", "anthropic-api/fixture", "--jobs-dir"])
        .arg(&jobs)
        .args(["--job-name", "test", "-k", "2", "--n-concurrent", "2"])
        .env("ANTHROPIC_API_KEY", "synthetic-benchmark-key")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let job: Value =
        serde_json::from_slice(&fs::read(jobs.join("test/result.json")).unwrap()).unwrap();
    assert_eq!(job["n_total_trials"], 2);
    for i in 1..=2 {
        let bytes = fs::read(jobs.join(format!("test/task-one__{i:03}/result.json"))).unwrap();
        let v: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["verifier_result"]["rewards"]["reward"], 1);
        assert_eq!(v["agent_result"]["n_cache_tokens"], 3);
        assert!(
            !String::from_utf8(bytes)
                .unwrap()
                .contains("synthetic-benchmark-key")
        );
    }
    // Campaign adoption uses existing complete native trial results; no inference.
    let campaign = r.path().join("campaign");
    let adopted = campaign.join("harbor-jobs/task-one/run-001");
    fs::create_dir_all(adopted.parent().unwrap()).unwrap();
    fs::rename(jobs.join("test"), &adopted).unwrap();
    let auth = r.path().join("oauth.json");
    fs::write(&auth, "{}").unwrap();
    let cmd = || {
        let mut c = tool();
        c.args(["terminal-bench-campaign", "--campaign-dir"])
            .arg(&campaign)
            .args(["--path"])
            .arg(&task)
            .args([
                "--task",
                "task-one",
                "--model",
                "anthropic-api/fixture",
                "-k",
                "2",
                "--binary",
            ])
            .arg(&binary)
            .env("NEXODE_HARBOR_OPENAI_AUTH", &auth)
            .env("ANTHROPIC_API_KEY", "synthetic-benchmark-key");
        c
    };
    assert!(cmd().status().unwrap().success());
    assert!(cmd().status().unwrap().success());
    let m: Value =
        serde_json::from_slice(&fs::read(campaign.join("campaign.json")).unwrap()).unwrap();
    assert_eq!(m["tasks_run"].as_array().unwrap().len(), 1);
    assert_eq!(m["tasks_run"][0]["mean_reward"], 1.);
    assert_eq!(fs::read_dir(adopted.parent().unwrap()).unwrap().count(), 1);
    fs::write(&binary, "different binary").unwrap();
    assert!(!cmd().status().unwrap().success());
    // A stalled agent cannot fabricate a successful reward; its container is removed.
    fs::write(
        &binary,
        "#!/bin/sh\ncase \" $* \" in *\" version \"*) echo '{}'; exit 0;; esac\nsleep 30\n",
    )
    .unwrap();
    let config = fs::read_to_string(task.join("task.toml"))
        .unwrap()
        .replace("timeout_sec = 15.0", "timeout_sec = 0.1");
    fs::write(task.join("task.toml"), config).unwrap();
    let out = tool()
        .args(["terminal-bench", "--path"])
        .arg(&task)
        .arg("--binary")
        .arg(&binary)
        .args(["--model", "anthropic-api/fixture", "--jobs-dir"])
        .arg(&jobs)
        .args(["--job-name", "timeout"])
        .env("ANTHROPIC_API_KEY", "synthetic-benchmark-key")
        .output()
        .unwrap();
    assert!(!out.status.success());
    let result: Value =
        serde_json::from_slice(&fs::read(jobs.join("timeout/task-one__001/result.json")).unwrap())
            .unwrap();
    assert!(
        result["exception_info"]["exception_message"]
            .as_str()
            .unwrap()
            .contains("timed out")
    );
    assert!(result["verifier_result"]["rewards"].is_null());
    let id = fs::read_to_string(jobs.join("timeout/task-one__001/start.log")).unwrap();
    assert!(
        !Command::new("docker")
            .args(["inspect", id.trim()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap()
            .success()
    );
}
