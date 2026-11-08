use super::*;
use sha2::{Digest, Sha256};
use std::{collections::HashSet, io::Read};
#[derive(Parser)]
struct Campaign {
    #[arg(long)]
    campaign_dir: PathBuf,
    #[arg(long)]
    task: Vec<String>,
    #[arg(long)]
    tasks_file: Option<PathBuf>,
    #[arg(long, default_value = "terminal-bench@2.0")]
    dataset: String,
    #[arg(long)]
    path: Option<PathBuf>,
    #[arg(long, default_value = "openai/gpt-5.4")]
    model: String,
    #[arg(short = 'k', long, default_value_t = 1)]
    n_attempts: usize,
    #[arg(long, default_value_t = 1.)]
    timeout_multiplier: f64,
    #[arg(long)]
    continue_on_failure: bool,
    #[arg(long, default_value_t = 1)]
    max_parallel_tasks: usize,
    #[arg(long)]
    binary: Option<PathBuf>,
    #[arg(long, default_value = "docker")]
    engine: String,
    #[arg(long)]
    dry_run: bool,
}
const PINNED: &[&str] = &[
    "runner_script",
    "model",
    "reasoning_effort",
    "service_tier",
    "binary_path",
    "binary_sha256",
    "openai_auth_path",
    "dataset",
    "path",
    "attempts_per_task",
    "n_concurrent",
    "timeout_multiplier",
];
fn hash(path: &Path) -> Result<String> {
    let mut f = fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut b = [0; 65536];
    loop {
        let n = f.read(&mut b)?;
        if n == 0 {
            break;
        }
        hash.update(&b[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
fn compatible(old: &Value, new: &Value) -> Result<()> {
    let different = PINNED
        .iter()
        .filter(|k| old[**k] != new[**k])
        .copied()
        .collect::<Vec<_>>();
    ensure!(
        different.is_empty(),
        "campaign settings drift: {}",
        different.join(", ")
    );
    Ok(())
}
fn trials(job: &Path) -> Result<Vec<Value>> {
    let mut dirs = fs::read_dir(job)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir() && p.file_name().unwrap().to_string_lossy().contains("__"))
        .collect::<Vec<_>>();
    dirs.sort();
    dirs.into_iter().filter(|p|p.join("result.json").is_file()).map(|p|{let file=p.join("result.json");let v:Value=serde_json::from_slice(&fs::read(&file)?)?;Ok(json!({"task_name":v["task_name"],"trial_name":v["trial_name"],"reward":v["verifier_result"]["rewards"]["reward"],"exception_type":v["exception_info"]["exception_type"],"exception_message":v["exception_info"]["exception_message"],"agent_return_code":v["agent_result"]["metadata"]["return_code"],"started_at":v["started_at"],"finished_at":v["finished_at"],"result_path":file}))}).collect()
}
fn summary(job: &Path, task: &str, required: usize, code: i32) -> Result<Value> {
    let ts = trials(job)?;
    ensure!(
        ts.iter().all(|v| v["task_name"] == task),
        "existing job contains results for another task"
    );
    let numbers = ts
        .iter()
        .filter_map(|v| v["reward"].as_f64())
        .filter(|n| n.is_finite())
        .collect::<Vec<_>>();
    let status = if numbers.len() >= required {
        "completed"
    } else if numbers.is_empty() {
        "completed_without_numeric_reward"
    } else {
        "completed_with_partial_numeric_reward"
    };
    let job_result = job.join("result.json");
    let v: Value = serde_json::from_slice(&fs::read(&job_result)?)?;
    Ok(
        json!({"task_name":task,"job_name":job.file_name().unwrap(),"jobs_dir":job.parent(),"status":status,"process_return_code":code,"job_result_path":job_result,"n_total_trials":v["n_total_trials"],"job_started_at":v["started_at"],"job_finished_at":v["finished_at"],"trial_names":ts.iter().map(|v|&v["trial_name"]).collect::<Vec<_>>(),"rewards":ts.iter().map(|v|&v["reward"]).collect::<Vec<_>>(),"mean_reward":if numbers.is_empty(){None}else{Some(numbers.iter().sum::<f64>()/numbers.len() as f64)},"trial_results":ts}),
    )
}
fn record(dir: &Path, manifest: &mut Value, value: Value) -> Result<()> {
    let records = manifest["tasks_run"]
        .as_array_mut()
        .context("tasks_run must be array")?;
    if let Some(old) = records
        .iter_mut()
        .find(|v| v["task_name"] == value["task_name"] && v["job_name"] == value["job_name"])
    {
        *old = value
    } else {
        records.push(value)
    }
    write_json(&dir.join("campaign.json"), manifest)?;
    fs::write(
        dir.join("results.jsonl"),
        manifest["tasks_run"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| format!("{v}\n"))
            .collect::<String>(),
    )?;
    Ok(())
}
fn complete(v: &Value, n: usize) -> bool {
    v["status"] == "completed"
        && v["mean_reward"].as_f64().is_some()
        && v["trial_results"].as_array().is_some_and(|ts| {
            ts.iter()
                .filter(|v| v["reward"].as_f64().is_some_and(f64::is_finite))
                .count()
                >= n
        })
}
pub fn run(args: impl Iterator<Item = String>) -> Result<i32> {
    let c =
        Campaign::try_parse_from(std::iter::once("terminal-bench-campaign".into()).chain(args))?;
    ensure!(
        c.n_attempts > 0 && c.max_parallel_tasks > 0 && c.max_parallel_tasks <= 128,
        "invalid attempts/parallelism"
    );
    let mut names = c.task;
    if let Some(p) = c.tasks_file {
        names.extend(
            fs::read_to_string(p)?
                .lines()
                .map(str::trim)
                .filter(|s| !s.is_empty() && !s.starts_with('#'))
                .map(str::to_owned),
        );
    }
    let mut seen = HashSet::new();
    names.retain(|s| seen.insert(s.clone()));
    ensure!(!names.is_empty(), "no tasks provided");
    for name in &names {
        safe_name(name)?;
    }
    let base = Args {
        path: c.path.clone(),
        dataset: if c.path.is_none() {
            Some(c.dataset.clone())
        } else {
            None
        },
        tasks: vec![],
        model: Some(c.model.clone()),
        claude: false,
        n_attempts: c.n_attempts,
        n_concurrent: 1,
        n_tasks: Some(1),
        jobs_dir: PathBuf::new(),
        job_name: None,
        timeout_multiplier: c.timeout_multiplier,
        binary: c.binary,
        engine: c.engine,
        yes: true,
        dry_run: c.dry_run,
    };
    let source = base.source()?;
    if c.dry_run {
        for name in names {
            let mut a = base.clone();
            a.tasks = vec![name];
            execute(a)?;
        }
        return Ok(0);
    }
    fs::create_dir_all(expand(&c.campaign_dir))?;
    let dir = expand(c.campaign_dir).canonicalize()?;
    base.ensure_binary()?;
    let binary = base.binary().canonicalize()?;
    let auth = match c.model.split_once('/').map(|(p, _)| p).unwrap_or("openai") {
        "openai" => Some(
            expand(env(
                "NEXODE_HARBOR_OPENAI_AUTH",
                "~/.nexode/openai-auth.json",
            ))
            .canonicalize()?,
        ),
        "claude" => {
            Some(expand(env("NEXODE_HARBOR_CLAUDE_AUTH", "~/.nexode/auth.json")).canonicalize()?)
        }
        "anthropic-api" | "openrouter" => None,
        _ => anyhow::bail!("unsupported campaign provider"),
    };
    let root = crate::root()?;
    let git = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(&root)
        .output()?;
    ensure!(git.status.success(), "cannot determine git revision");
    let settings = json!({"schema_version":1,"created_at":chrono::Utc::now().to_rfc3339(),"repo_root":root,"git_head":String::from_utf8_lossy(&git.stdout).trim(),"runner_script":"orbit-dev terminal-bench (native-docker-v1)","model":c.model,"reasoning_effort":env("NEXODE_OPENAI_REASONING_EFFORT","high"),"service_tier":env("NEXODE_OPENAI_SERVICE_TIER","priority"),"binary_path":binary,"binary_sha256":hash(&binary)?,"openai_auth_path":auth,"dataset":c.dataset,"path":source,"attempts_per_task":c.n_attempts,"n_concurrent":1,"timeout_multiplier":c.timeout_multiplier,"tasks_run":[]});
    let manifest_path = dir.join("campaign.json");
    let mut manifest = if manifest_path.exists() {
        let v: Value = serde_json::from_slice(&fs::read(&manifest_path)?)?;
        compatible(&v, &settings)?;
        v
    } else {
        write_json(&manifest_path, &settings)?;
        settings
    };
    let mut pending = vec![];
    for name in names {
        if manifest["tasks_run"]
            .as_array()
            .context("manifest tasks_run")?
            .iter()
            .any(|v| v["task_name"] == name && complete(v, c.n_attempts))
        {
            continue;
        }
        let jobs = dir.join("harbor-jobs").join(&name);
        fs::create_dir_all(&jobs)?;
        let mut existing = fs::read_dir(&jobs)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.is_dir())
            .collect::<Vec<_>>();
        existing.sort();
        existing.reverse();
        let mut adopted = false;
        for p in &existing {
            if p.join("result.json").is_file() {
                let v = summary(p, &name, c.n_attempts, 0)?;
                if complete(&v, c.n_attempts) {
                    record(&dir, &mut manifest, v)?;
                    adopted = true;
                    break;
                }
            }
        }
        if adopted {
            continue;
        }
        let mut index = existing.len() + 1;
        while jobs.join(format!("run-{index:03}")).exists() {
            index += 1
        }
        let mut a = base.clone();
        a.tasks = vec![name];
        a.jobs_dir = jobs;
        a.job_name = Some(format!("run-{index:03}"));
        pending.push(a);
    }
    let mut failed = false;
    for batch in pending.chunks(c.max_parallel_tasks) {
        let values = std::thread::scope(|s| {
            batch
                .iter()
                .map(|a| s.spawn(move || execute(a.clone())))
                .collect::<Vec<_>>()
                .into_iter()
                .map(|h| {
                    h.join()
                        .unwrap_or_else(|_| Err(anyhow::anyhow!("campaign worker panicked")))
                })
                .collect::<Vec<_>>()
        });
        for (a, result) in batch.iter().zip(values) {
            let code = result.as_ref().copied().unwrap_or(1);
            let name = &a.tasks[0];
            let job = a.jobs_dir.join(a.job_name.as_ref().unwrap());
            let v=summary(&job,name,c.n_attempts,code).unwrap_or_else(|e|json!({"task_name":name,"job_name":a.job_name,"jobs_dir":a.jobs_dir,"status":"failed_to_produce_result","return_code":code,"error":result.err().unwrap_or(e).to_string()}));
            failed |= !complete(&v, c.n_attempts);
            record(&dir, &mut manifest, v)?;
        }
        if failed && !c.continue_on_failure {
            break;
        }
    }
    Ok(i32::from(failed && !c.continue_on_failure))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn partial_rewards_never_complete_and_drift_rejected() {
        assert!(!complete(
            &json!({"status":"completed","mean_reward":1,"trial_results":[{"reward":true},{"reward":1}]}),
            2
        ));
        assert!(complete(
            &json!({"status":"completed","mean_reward":0.5,"trial_results":[{"reward":0},{"reward":1}]}),
            2
        ));
        assert!(compatible(&json!({"model":"a"}), &json!({"model":"b"})).is_err());
        assert!(safe_name("../escape").is_err());
    }
}
