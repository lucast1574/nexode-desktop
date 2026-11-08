use super::*;
use clap::Parser;
use std::{path::PathBuf, time::Duration};
#[derive(Parser)]
struct Args {
    #[arg(long)]
    cases: Option<PathBuf>,
    #[arg(long = "case")]
    ids: Vec<String>,
    #[arg(long)]
    catalog_file: Option<PathBuf>,
    #[arg(long)]
    allow_catalog_mismatch: bool,
    #[arg(long, default_value_t = 1)]
    trials: usize,
    #[arg(long, default_value_t = 5)]
    max_attempts: usize,
    #[arg(long, default_value_t = 90.)]
    timeout: f64,
    #[arg(long, default_value_t = 4)]
    catalog_retries: usize,
    #[arg(long, default_value_t = 0.5)]
    retry_delay: f64,
    #[arg(long)]
    nexode: Option<PathBuf>,
    #[arg(long)]
    model: Option<String>,
    #[arg(long)]
    provider: Option<String>,
    #[arg(long)]
    discovery_only: bool,
    #[arg(long)]
    output: Option<PathBuf>,
    #[arg(long)]
    dry_run: bool,
}
fn catalog_file(path: &Path, categories: &[String]) -> Result<Value> {
    let v: Value = serde_json::from_slice(&std::fs::read(path)?)?;
    let v = v.get("categories").unwrap_or(&v);
    ensure!(v.is_object(), "catalog must be category mapping");
    let mut result = json!({});
    for c in categories {
        let raw = v.get(c).cloned().unwrap_or(json!([]));
        let raw = if raw.is_object() {
            raw.get("tools").cloned().unwrap_or(json!([]))
        } else {
            raw
        };
        ensure!(raw.is_array(), "catalog category {c} is not a list");
        result[c] = raw;
    }
    Ok(result)
}
pub(super) fn coverage(cases: &[Value], catalog: &Value) -> Result<Value> {
    let mut live = BTreeSet::new();
    for (category, tools) in catalog.as_object().context("catalog mapping")? {
        for t in array(tools) {
            let name = lower(t, "name");
            ensure!(!name.is_empty(), "catalog entry has no name");
            live.insert(format!("{category}/{name}"));
        }
    }
    let covered: BTreeSet<_> = cases
        .iter()
        .filter(|c| c["expectation"] == "listing")
        .map(|c| {
            format!(
                "{}/{}",
                string(c, "expected_category"),
                string(c, "expected_tool")
            )
        })
        .collect();
    Ok(
        json!({"live_targets":live,"case_targets":covered,"missing_cases":live.difference(&covered).collect::<Vec<_>>(),"stale_cases":covered.difference(&live).collect::<Vec<_>>()}),
    )
}
fn fetch(s: &runtime::Server, categories: &[String], a: &Args) -> Result<Value> {
    let ws = s.root.path().join("catalog");
    std::fs::create_dir(&ws)?;
    let created = s.debug(&format!("create_session:{}", ws.display()), None, None, 60.)?;
    let id = created["session_id"]
        .as_str()
        .context("catalog session id")?;
    let result = (|| {
        let mut result = json!({});
        for category in categories {
            let input = json!({"category":category,"query":format!("public {} capabilities available for external agent workflows",category.replace('-'," ")),"reason":"Enumerate live public listings to validate benchmark scenarios without selecting or configuring any provider."});
            let mut last = None;
            for attempt in 1..=a.catalog_retries {
                let got = s
                    .debug(
                        "tool",
                        Some(&format!("integration_tools {input}")),
                        Some(id),
                        20.,
                    )
                    .and_then(|v| {
                        let call = parse_output(&string(&v, "output"), 0.);
                        ensure!(
                            call["category"] == *category
                                && ["listing", "empty"]
                                    .contains(&string(&call, "outcome").as_str()),
                            "unexpected catalog response: {v}"
                        );
                        Ok(call)
                    });
                match got {
                    Ok(call) => {
                        result[category] = json!(
                            array(&call["tools"])
                                .iter()
                                .map(|n| json!({"name":n}))
                                .collect::<Vec<_>>()
                        );
                        last = None;
                        break;
                    }
                    Err(e) => last = Some(e),
                }
                if attempt < a.catalog_retries {
                    std::thread::sleep(Duration::from_secs_f64(a.retry_delay * attempt as f64));
                }
            }
            if let Some(e) = last {
                return Err(e);
            }
        }
        Ok(result)
    })();
    let cleanup = s.debug(&format!("destroy_session:{id}"), None, None, 15.);
    result.and_then(|v| {
        cleanup?;
        Ok(v)
    })
}
pub(super) fn decision(case: &Value, call: &Value) -> &'static str {
    if case["expectation"] == "no-discovery" || call["outcome"] == "selection" {
        "failure"
    } else if call["outcome"] == "listing"
        && call["category"] == case["expected_category"]
        && array(&call["tools"]).contains(&case["expected_tool"])
    {
        "success"
    } else {
        "continue"
    }
}
fn attempt(
    s: &runtime::Server,
    a: &Args,
    case: &Value,
    n: usize,
    model: &str,
    workspace: &Path,
) -> Result<Value> {
    let mut command = s.model_command(
        model,
        a.provider.as_deref(),
        workspace,
        &string(case, "prompt"),
        a.discovery_only,
    );
    let mut calls = vec![];
    let mut success = false;
    let mut failed = false;
    let mut hit = Value::Null;
    let mut v = runtime::stream(&mut command, a.timeout, |event, elapsed| {
        if event["type"] != "tool_done"
            || !["integration_tools", "discover_tools"].contains(&string(&event, "name").as_str())
        {
            return false;
        }
        let call = parse_output(&string(&event, "output"), elapsed);
        let d = decision(case, &call);
        if d == "success" {
            success = true;
            hit = call["elapsed_seconds"].clone();
        }
        failed = d == "failure";
        calls.push(call);
        success || failed
    })?;
    if case["expectation"] == "no-discovery" && !failed && v["timed_out"] == false {
        success = v["exit_code"] == 0;
        if success {
            hit = v["elapsed_seconds"].clone();
        }
    }
    v["attempt"] = json!(n);
    v["success"] = json!(success);
    v["hit_seconds"] = hit;
    v["discovery_calls"] = json!(calls);
    v["runtime_error_count"] = json!(
        regex(
            r"(?i)\b(error|failed|failure|timed out|timeout|did not start|exited before startup)\b"
        )
        .find_iter(&string(&v, "stderr_tail"))
        .count()
    );
    Ok(v)
}
pub(super) fn summarize(case: &Value, trials: Vec<Value>) -> Value {
    let successful: Vec<_> = trials.iter().filter(|t| t["success"] == true).collect();
    let mut wrong = std::collections::BTreeMap::<String, usize>::new();
    let (mut unexpected, mut selections, mut reached, mut confounded) = (0, 0, 0, 0);
    for t in &trials {
        if case["expectation"] == "listing"
            && array(&t["attempts"][0]["discovery_calls"]).iter().any(|c| {
                c["category"] == case["expected_category"]
                    && array(&c["tools"]).contains(&case["expected_tool"])
            })
        {
            reached += 1
        }
        if t["success"] != true
            && array(&t["attempts"])
                .iter()
                .any(|a| a["runtime_error_count"].as_u64().unwrap_or(0) > 0)
        {
            confounded += 1
        }
        for a in array(&t["attempts"]) {
            for c in array(&a["discovery_calls"]) {
                if case["expectation"] == "no-discovery" {
                    unexpected += 1
                } else {
                    if c["outcome"] == "selection" {
                        selections += 1
                    }
                    if !c["category"].is_null() && c["category"] != case["expected_category"] {
                        *wrong.entry(string(c, "category")).or_default() += 1
                    }
                }
            }
        }
    }
    let first = trials.iter().filter(|t| t["attempts_to_hit"] == 1).count();
    let n = trials.len() as f64;
    json!({"case":case,"trial_count":trials.len(),"successful_trials":successful.len(),"success_rate":successful.len() as f64/n,"first_attempt_successful_trials":first,"first_attempt_success_rate":first as f64/n,"first_attempt_target_reach_rate":if case["expectation"]=="listing"{json!(reached as f64/n)}else{Value::Null},"mean_attempts_to_hit":mean(successful.iter().filter_map(|t|t["attempts_to_hit"].as_f64()).collect(),3),"median_hit_seconds":median(successful.iter().filter_map(|t|t["hit_seconds"].as_f64()).collect()),"runtime_confounded_trials":confounded,"wrong_category_calls":wrong,"unexpected_discovery_calls":unexpected,"direct_selection_calls":selections,"trials":trials})
}
pub fn run(args: impl Iterator<Item = String>) -> Result<i32> {
    let mut a = Args::try_parse_from(std::iter::once("benchmark-discovery".into()).chain(args))?;
    ensure!(
        a.trials > 0
            && a.max_attempts > 0
            && a.catalog_retries > 0
            && a.timeout.is_finite()
            && a.timeout > 0.
            && a.retry_delay.is_finite()
            && a.retry_delay >= 0.,
        "invalid trials, timeout or retry values"
    );
    a.provider = a
        .provider
        .or_else(|| std::env::var("NEXODE_DISCOVERY_BENCHMARK_PROVIDER").ok());
    let started = chrono::Utc::now();
    let root = crate::root()?;
    let categories = categories()?;
    let path = a
        .cases
        .clone()
        .unwrap_or(root.join("scripts/discovery_benchmark_cases.json"));
    let all = cases(&path, false, &categories)?;
    let chosen = filter(all.clone(), &a.ids, &[], false)?;
    let model = a
        .model
        .clone()
        .or_else(|| std::env::var("NEXODE_DISCOVERY_BENCHMARK_MODEL").ok())
        .unwrap_or("gpt-5.6-sol".into());
    let binary = a
        .nexode
        .clone()
        .or_else(|| std::env::var_os("NEXODE_BIN").map(PathBuf::from))
        .unwrap_or("nexode".into());
    let server = if a.dry_run && a.catalog_file.is_some() {
        None
    } else {
        Some(runtime::Server::start(&binary, a.provider.as_deref())?)
    };
    let catalog = if let Some(p) = &a.catalog_file {
        catalog_file(p, &categories)?
    } else {
        fetch(server.as_ref().unwrap(), &categories, &a)?
    };
    let coverage = coverage(&all, &catalog)?;
    let mismatch = !array(&coverage["missing_cases"]).is_empty()
        || !array(&coverage["stale_cases"]).is_empty();
    ensure!(
        !mismatch || a.allow_catalog_mismatch,
        "catalog coverage mismatch: {coverage}"
    );
    let mut results = vec![];
    if !a.dry_run {
        let s = server.as_ref().unwrap();
        for case in &chosen {
            let mut trials = vec![];
            for n in 1..=a.trials {
                let mut attempts = vec![];
                for k in 1..=a.max_attempts {
                    let ws = tempfile::Builder::new()
                        .prefix("trial-")
                        .tempdir_in(s.root.path())?;
                    let v = attempt(s, &a, case, k, &model, ws.path())?;
                    let stop = v["success"] == true || case["expectation"] == "no-discovery";
                    attempts.push(v);
                    if stop {
                        break;
                    }
                    std::thread::sleep(Duration::from_secs_f64(a.retry_delay));
                }
                let hit = attempts.iter().find(|v| v["success"] == true);
                let outcome = if hit.is_some() {
                    if case["expectation"] == "listing" {
                        "expected-listing"
                    } else {
                        "no-discovery"
                    }
                } else if case["expectation"] == "no-discovery"
                    && attempts
                        .iter()
                        .any(|a| !array(&a["discovery_calls"]).is_empty())
                {
                    "unexpected-discovery"
                } else if attempts
                    .iter()
                    .any(|a| a["runtime_error_count"].as_u64().unwrap_or(0) > 0)
                {
                    "runtime-confounded-miss"
                } else {
                    "clean-miss"
                };
                trials.push(json!({"trial":n,"success":hit.is_some(),"attempts_to_hit":hit.map(|a|&a["attempt"]),"hit_seconds":hit.map(|a|&a["hit_seconds"]),"outcome":outcome,"attempts":attempts}));
                println!(
                    "NEXODE_PROGRESS {}",
                    json!({"current":n,"total":a.trials,"unit":"trials","message":format!("Completed {}",string(case,"id"))})
                );
            }
            results.push(summarize(case, trials));
        }
    }
    let passed = !mismatch
        && results
            .iter()
            .all(|r| r["successful_trials"] == r["trial_count"]);
    let report = json!({"benchmark":"discovery-trigger","version":2,"started_at":started.to_rfc3339(),"finished_at":chrono::Utc::now().to_rfc3339(),"benchmark_marker":marker(),"config":{"catalog_source":a.catalog_file.as_ref().map(|p|p.display().to_string()).unwrap_or("integration_tools".into()),"model":model,"provider":a.provider,"tool_mode":if a.discovery_only{"discovery-only"}else{"full"},"trials":a.trials,"max_attempts":a.max_attempts,"timeout_seconds":a.timeout,"cases_file":path},"coverage":coverage,"catalog":catalog,"results":results,"passed":passed});
    let output = a
        .output
        .unwrap_or(root.join("target/discovery-benchmark/latest.json"));
    write_report(&output, &report)?;
    println!(
        "Discovery benchmark passed={passed}; report {}",
        output.display()
    );
    Ok(i32::from(!passed))
}
