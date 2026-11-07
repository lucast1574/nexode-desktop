use super::{
    engine::*,
    runtime::*,
    task::{Task, rewards},
    *,
};
use std::collections::BTreeMap;
const HOME: &str = "/tmp/nexode-home";
const RUNTIME: &str = "/tmp/nexode-runtime";
pub(super) fn trial(a: &Args, t: &Task, attempt: usize, job: &Path) -> Result<Value> {
    let trial_name = format!("{}__{attempt:03}", t.name);
    let dir = job.join(&trial_name);
    fs::create_dir(&dir)?;
    let started = chrono::Utc::now().to_rfc3339();
    let outcome = perform(a, t, &dir);
    let (reward, agent, exception) = match outcome {
        Ok((r, m)) => (r, m, Value::Null),
        Err(e) => (
            Value::Null,
            Value::Null,
            json!({"exception_type":"NativeTrialError","exception_message":e.to_string()}),
        ),
    };
    let result = json!({"task_name":t.name,"trial_name":trial_name,"started_at":started,"finished_at":chrono::Utc::now().to_rfc3339(),"verifier_result":{"rewards":reward},"agent_result":agent,"exception_info":exception});
    write_json(&dir.join("result.json"), &result)?;
    Ok(result)
}
fn perform(a: &Args, t: &Task, dir: &Path) -> Result<(Value, Value)> {
    let token = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let name = format!("orbit-tb-{}-{token}", std::process::id());
    let image = t
        .config
        .pointer("/environment/docker_image")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let image_name = image.clone().unwrap_or_else(|| format!("{name}:latest"));
    let c = Container {
        engine: a.engine.clone(),
        name,
        image: if image.is_none() {
            Some(image_name.clone())
        } else {
            None
        },
    };
    if image.is_none() {
        let secs = t
            .config
            .pointer("/environment/build_timeout_sec")
            .and_then(Value::as_u64)
            .unwrap_or(600);
        ok(
            &c.engine,
            vec![
                "build".into(),
                "-t".into(),
                image_name.clone(),
                t.path.join("environment").to_string_lossy().into(),
            ],
            secs,
            &dir.join("build.log"),
        )?;
    }
    let secrets = tempfile::tempdir()?;
    let base_env = envfile(
        secrets.path(),
        "container.env",
        &task_env(t, "environment")?,
    )?;
    let mut run = words(&[
        "run",
        "-d",
        "--name",
        &c.name,
        "--label",
        "app=orbit-terminal-bench",
        "--env-file",
    ]);
    run.push(base_env.to_string_lossy().into());
    for (key, flag, suffix) in [
        ("cpus", "--cpus", ""),
        ("memory_mb", "--memory", "m"),
        ("gpus", "--gpus", ""),
    ] {
        if let Some(v) = t.config.pointer(&format!("/environment/{key}"))
            && v.as_f64().is_some_and(|n| n > 0.)
        {
            run.extend([flag.into(), format!("{v}{suffix}")]);
        }
    }
    if t.config.pointer("/environment/allow_internet") == Some(&json!(false))
        || t.config
            .pointer("/environment/network_mode")
            .and_then(Value::as_str)
            == Some("none")
    {
        run.extend(words(&["--network", "none"]));
    }
    if let Some(w) = t
        .config
        .pointer("/environment/workdir")
        .and_then(Value::as_str)
    {
        run.extend(words(&["--workdir", w]));
    }
    run.extend(words(&[
        "--entrypoint",
        "/bin/sh",
        &image_name,
        "-c",
        "while :; do sleep 3600; done",
    ]));
    ok(&c.engine, run, 120, &dir.join("start.log"))?;
    shell(
        &c,
        "mkdir -p /tmp/nexode-home /tmp/nexode-runtime/lib /tmp/nexode-input /tmp/nexode-output /usr/local/bin /logs/agent /logs/verifier",
        &dir.join("setup.log"),
    )?;
    cp(
        &c,
        &a.binary(),
        "/usr/local/bin/nexode",
        &dir.join("binary-copy.log"),
    )?;
    let cert = if std::env::var_os("NEXODE_HARBOR_CA_BUNDLE").is_some() {
        resolve("NEXODE_HARBOR_CA_BUNDLE", "")?
    } else {
        [
            "/etc/ssl/certs/ca-certificates.crt",
            "/etc/ca-certificates/extracted/tls-ca-bundle.pem",
        ]
        .iter()
        .map(PathBuf::from)
        .find(|p| p.is_file())
        .context("CA bundle unavailable")?
    };
    cp(
        &c,
        &cert,
        "/tmp/nexode-home/ca-certificates.crt",
        &dir.join("ca-copy.log"),
    )?;
    for (stem, var) in [
        ("libssl", "NEXODE_HARBOR_LIBSSL"),
        ("libcrypto", "NEXODE_HARBOR_LIBCRYPTO"),
    ] {
        let mut candidates = vec![];
        if let Ok(p) = std::env::var(var) {
            candidates.push(PathBuf::from(p));
        }
        if let Some(p) = a.binary().parent()
            && let Ok(entries) = fs::read_dir(p)
        {
            let mut siblings = entries
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| {
                    p.file_name()
                        .is_some_and(|n| n.to_string_lossy().starts_with(&format!("{stem}.so")))
                })
                .collect::<Vec<_>>();
            siblings.sort();
            candidates.extend(siblings);
        }
        for base in [
            "/usr/lib/x86_64-linux-gnu",
            "/lib/x86_64-linux-gnu",
            "/usr/lib",
        ] {
            candidates.push(PathBuf::from(format!("{base}/{stem}.so.3")));
        }
        if let Some(p) = candidates.into_iter().find(|p| p.is_file()) {
            cp(
                &c,
                &p,
                &format!(
                    "/tmp/nexode-runtime/lib/{}",
                    p.file_name().unwrap().to_string_lossy()
                ),
                &dir.join(format!("{stem}-copy.log")),
            )?;
        }
    }
    let model = a.model();
    let (provider, model) = model
        .split_once('/')
        .unwrap_or((if a.claude { "anthropic-api" } else { "openai" }, &model));
    let mut vars = BTreeMap::from_iter(
        [
            ("HOME", HOME),
            ("NEXODE_HOME", HOME),
            ("NEXODE_RUNTIME_DIR", RUNTIME),
            ("NEXODE_NO_TELEMETRY", "1"),
            ("NEXODE_PROVIDER", provider),
            ("NEXODE_MODEL", model),
            ("SSL_CERT_FILE", "/tmp/nexode-home/ca-certificates.crt"),
            ("OPENSSL_CERT_FILE", "/tmp/nexode-home/ca-certificates.crt"),
            ("LD_LIBRARY_PATH", "/tmp/nexode-runtime/lib"),
        ]
        .into_iter()
        .map(|(k, v)| (k.into(), v.into())),
    );
    for (k, d) in [
        ("NEXODE_OPENAI_REASONING_EFFORT", "high"),
        ("NEXODE_OPENAI_SERVICE_TIER", "priority"),
        ("NEXODE_ANTHROPIC_REASONING_EFFORT", "high"),
    ] {
        vars.insert(k.into(), env(k, d));
    }
    match provider {
        "openai" => cp(
            &c,
            &resolve("NEXODE_HARBOR_OPENAI_AUTH", "~/.nexode/openai-auth.json")?,
            "/tmp/nexode-home/openai-auth.json",
            &dir.join("auth-copy.log"),
        )?,
        "claude" => cp(
            &c,
            &resolve("NEXODE_HARBOR_CLAUDE_AUTH", "~/.nexode/auth.json")?,
            "/tmp/nexode-home/auth.json",
            &dir.join("auth-copy.log"),
        )?,
        "anthropic-api" => {
            vars.insert(
                "ANTHROPIC_API_KEY".into(),
                key(
                    "ANTHROPIC_API_KEY",
                    "NEXODE_HARBOR_ANTHROPIC_KEY",
                    "NEXODE_HARBOR_ANTHROPIC_ENV",
                    "~/.config/nexode/anthropic.env",
                )?,
            );
        }
        "openrouter" => {
            vars.insert(
                "OPENROUTER_API_KEY".into(),
                key(
                    "OPENROUTER_API_KEY",
                    "NEXODE_HARBOR_OPENROUTER_KEY",
                    "NEXODE_HARBOR_OPENROUTER_ENV",
                    "~/.config/nexode/openrouter.env",
                )?,
            );
        }
        _ => anyhow::bail!("unsupported benchmark provider {provider}"),
    }
    let instruction = instruction(t)?;
    fs::write(dir.join("instruction.txt"), instruction)?;
    cp(
        &c,
        &dir.join("instruction.txt"),
        "/tmp/nexode-input/instruction.txt",
        &dir.join("instruction-copy.log"),
    )?;
    shell(
        &c,
        "chmod 755 /usr/local/bin/nexode; chmod 700 /tmp/nexode-home; for p in /tmp/nexode-home/*auth.json; do [ ! -f \"$p\" ] || chmod 600 \"$p\"; done",
        &dir.join("permissions.log"),
    )?;
    let auth_env = envfile(secrets.path(), "agent.env", &vars)?;
    let identity_log = dir.join("agent-user.txt");
    let mut identity = words(&["exec"]);
    if let Some(user) = t.config.pointer("/agent/user") {
        let u = user
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| user.to_string());
        identity.extend(words(&["--user", &u]));
    }
    identity.extend(words(&[&c.name, "id", "-u"]));
    ok(&c.engine, identity, 30, &identity_log)?;
    let uid = fs::read_to_string(&identity_log)?
        .trim()
        .parse::<u32>()
        .context("agent UID")?;
    shell(
        &c,
        &format!(
            "chown -R {uid} /tmp/nexode-home /tmp/nexode-runtime /tmp/nexode-input /tmp/nexode-output /logs/agent"
        ),
        &dir.join("ownership.log"),
    )?;
    let mut exec = words(&["exec", "--env-file"]);
    exec.push(auth_env.to_string_lossy().into());
    if let Some(user) = t.config.pointer("/agent/user") {
        let u = user
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| user.to_string());
        exec.extend(words(&["--user", &u]));
    }
    exec.push(c.name.clone());
    let mut version = exec.clone();
    version.extend(words(&[
        "/usr/local/bin/nexode",
        "--quiet",
        "--no-update",
        "--no-selfdev",
        "version",
        "--json",
    ]));
    ok(&c.engine, version, 60, &dir.join("setup_version.json"))?;
    if let Ok(w) = std::env::var("NEXODE_TASK_WORKDIR") {
        vars.insert("NEXODE_TASK_WORKDIR".into(), w);
        fs::remove_file(&auth_env)?;
        envfile(secrets.path(), "agent.env", &vars)?;
    }
    exec.extend(words(&["sh","-c",r#"workdir="${NEXODE_TASK_WORKDIR:-}"; if [ -z "$workdir" ]; then if [ -d /app ]; then workdir=/app; else workdir="$(pwd)"; fi; fi; instruction="$(cat /tmp/nexode-input/instruction.txt)"; /usr/local/bin/nexode --quiet --no-update --no-selfdev --provider "$NEXODE_PROVIDER" --model "$NEXODE_MODEL" -C "$workdir" run --ndjson "$instruction" > /tmp/nexode-output/events.ndjson 2> /tmp/nexode-output/stderr.txt"#]));
    let code = command(
        &c.engine,
        &exec,
        t.seconds("agent", 900., a.timeout_multiplier)?,
        &dir.join("exec_stdout.txt"),
    )?;
    fs::create_dir(dir.join("nexode-output"))?;
    ok(
        &c.engine,
        words(&[
            "cp",
            &format!("{}:/tmp/nexode-output/.", c.name),
            &dir.join("nexode-output").to_string_lossy(),
        ]),
        60,
        &dir.join("download.log"),
    )?;
    ensure!(
        code != 124,
        "agent timed out; container stopped before verification"
    );
    let final_payload = payload(&dir.join("nexode-output"))?;
    let usage = &final_payload["usage"];
    let metadata = json!({"metadata":{"return_code":code,"provider":provider,"model":model,"nexode_binary":a.binary(),"nexode_result":final_payload},"n_input_tokens":usage["input_tokens"],"n_output_tokens":usage["output_tokens"],"n_cache_tokens":usage["cache_read_input_tokens"].as_u64().unwrap_or(0)+usage["cache_creation_input_tokens"].as_u64().unwrap_or(0)});
    cp(
        &c,
        &t.path.join("tests"),
        "/tests",
        &dir.join("tests-copy.log"),
    )?;
    shell(
        &c,
        "rm -f /logs/verifier/reward.json /logs/verifier/reward.txt; chmod +x /tests/test.sh",
        &dir.join("verifier-prepare.log"),
    )?;
    let verifier_env = envfile(secrets.path(), "verifier.env", &task_env(t, "verifier")?)?;
    let mut verify = words(&["exec", "--env-file", &verifier_env.to_string_lossy()]);
    if let Some(user) = t.config.pointer("/verifier/user") {
        let u = user
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| user.to_string());
        verify.extend(words(&["--user", &u]));
    }
    verify.extend(words(&[&c.name, "bash", "/tests/test.sh"]));
    let verify_code = command(
        &c.engine,
        &verify,
        t.seconds("verifier", 600., a.timeout_multiplier)?,
        &dir.join("test-stdout.txt"),
    )?;
    ensure!(verify_code != 124, "verifier timed out");
    fs::create_dir(dir.join("verifier"))?;
    ok(
        &c.engine,
        words(&[
            "cp",
            &format!("{}:/logs/verifier/.", c.name),
            &dir.join("verifier").to_string_lossy(),
        ]),
        60,
        &dir.join("verifier-download.log"),
    )?;
    let reward = rewards(&dir.join("verifier"))?;
    Ok((reward, metadata))
}
