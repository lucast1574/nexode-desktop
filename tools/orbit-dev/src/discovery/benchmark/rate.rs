use super::*;
use clap::Parser;
use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
    time::Duration,
};
const INVALID: &str = r"(?i)(token refresh failed|re-authenticate|/login|no api key|missing api key|unauthorized|401|429|400 bad request|request_too_expensive|too expensive|insufficient|billing|rate limit|quota|provider error|connection refused|dns error|failed to connect|model not found|unknown model)";
#[derive(Parser)]
struct Args {
    #[arg(long)]
    cases: Option<PathBuf>,
    #[arg(long = "case")]
    ids: Vec<String>,
    #[arg(long = "tag")]
    tags: Vec<String>,
    #[arg(long, default_value_t = 1)]
    trials: usize,
    #[arg(long, default_value_t = 120.)]
    timeout: f64,
    #[arg(long)]
    nexode: Option<PathBuf>,
    #[arg(long)]
    model: Option<String>,
    #[arg(long)]
    provider: Option<String>,
    #[arg(long)]
    output: Option<PathBuf>,
    #[arg(long, default_value_t = 0.8)]
    min_recall: f64,
    #[arg(long, default_value_t = 0.9)]
    min_precision: f64,
    #[arg(long, default_value_t = 1.)]
    min_selection_accuracy: f64,
    #[arg(long, default_value_t = 0.5)]
    retry_delay: f64,
    #[arg(long, default_value_t = 3)]
    invalid_retries: usize,
    #[arg(long, default_value_t = 20.)]
    invalid_backoff: f64,
    #[arg(long)]
    list: bool,
}
pub(super) fn identity(binary: &Path) -> Result<Value> {
    let resolved = if binary.components().count() > 1 || binary.is_absolute() {
        binary.canonicalize()?
    } else {
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
            .map(|p| p.join(binary))
            .find(|p| p.is_file())
            .context("binary not found in PATH")?
            .canonicalize()?
    };
    let o = crate::process::capture(
        std::process::Command::new(&resolved).arg("--version"),
        Duration::from_secs(15),
    )?;
    ensure!(o.status.success(), "binary --version failed");
    let version = String::from_utf8(o.stdout)?.trim().to_string();
    let commit = regex(r"(?i)\(([0-9a-f]{7,40})[, )-]")
        .captures(&version)
        .map(|c| c[1].to_string());
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut f = std::fs::File::open(&resolved)?;
    let mut digest = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        digest.update(&buf[..n]);
    }
    Ok(
        json!({"argument":binary,"path":resolved,"version":version,"commit":commit,"sha256":format!("{:x}",digest.finalize()),"size_bytes":f.metadata()?.len()}),
    )
}
pub(super) fn bypasses(tool: &str, text: &str, elapsed: f64) -> Vec<Value> {
    if !["bash", "webfetch", "mcp"].contains(&tool) {
        return vec![];
    }
    let patterns = [
        (
            "package-install",
            r"(?i)\b(?:npm|pnpm|yarn|bun)\s+(?:add|install)\s+([A-Za-z@][\w@/.-]*)",
        ),
        (
            "package-install",
            r"(?i)\b(?:pip|pip3|uv)\s+(?:install|add)\s+([A-Za-z@][\w@/.-]*)",
        ),
        ("package-install", r"(?i)\bcargo\s+add\s+\w"),
        ("package-install", r"(?i)\bgo\s+get\s+\w+\.\w"),
        (
            "vendor-cli",
            r#"(?im)(?:^|[\n;&|"'(]|&&|\|\|)\s*(?:sudo\s+|npx\s+|env\s+\w+=\S+\s+)*(?:vercel|stripe|supabase|neonctl|railway|flyctl|heroku|wrangler|doctl|netlify|planetscale|pscale|sentry-cli|datadog-ci|clerk|auth0|twilio|sendgrid|resend)\s+[a-z]"#,
        ),
        (
            "vendor-endpoint",
            r"(?i)https?://(?:[\w.-]+\.)?(?:api|dashboard|console)\.[\w.-]+",
        ),
        (
            "signup-page",
            r"(?i)https?://[\w.-]+/(?:signup|sign-up|register|pricing|api-keys?)\b",
        ),
        ("mcp-connect", r#"(?i)"command"|connect"#),
    ];
    patterns.iter().filter_map(|(kind,p)|{
        if (tool=="mcp")!=(*kind=="mcp-connect"){return None}
        regex(p).find(text).map(|m|{let start=text[..m.start()].char_indices().rev().nth(39).map(|(i,_)|i).unwrap_or(0);let end=text[m.end()..].char_indices().nth(60).map(|(i,_)|m.end()+i).unwrap_or(text.len());json!({"tool":tool,"kind":kind,"evidence":text[start..end].trim().chars().take(200).collect::<String>(),"elapsed_seconds":round(elapsed,3)})})
    }).collect()
}
pub(super) fn tool_input(input: &str) -> (Value, Value, Value) {
    let v: Value = serde_json::from_str(input).unwrap_or(Value::Null);
    (
        optional(lower(&v, "action")),
        optional(lower(&v, "tool")),
        optional(string(&v, "reason")),
    )
}
pub(super) fn correct(
    case: &Value,
    call: &Value,
    action: &Value,
    tool: &Value,
    reason: &Value,
) -> bool {
    case["expect"] == "select"
        && *action == "select"
        && *tool == case["expected_tool"]
        && reason.as_str().is_some_and(|s| s.chars().count() >= 40)
        && call["outcome"] == "selection"
        && array(&call["tools"]) == [case["expected_tool"].clone()]
        && call["category"] == case["expected_category"]
        && call["listed"] == case["expected_listed"]
}
pub(super) fn empty_trial(n: usize) -> Value {
    json!({"trial":n,"outcome":"pending","browsed":false,"browse_categories":[],"selected_via_discovery":[],"off_catalog_selects":[],"first_call_seconds":null,"category_correct":null,"selection_correct":null,"bypasses":[],"discovery_calls":[],"other_tool_calls":[],"exit_code":null,"timed_out":false,"elapsed_seconds":0.,"stderr_tail":"","invalid_reason":null})
}
fn push(v: &mut Value, key: &str, item: Value) {
    v[key].as_array_mut().unwrap().push(item);
}
pub(super) fn event(
    v: &mut Value,
    case: &Value,
    e: &Value,
    elapsed: f64,
    pending: &mut HashMap<String, String>,
    current: &mut String,
) -> bool {
    match string(e, "type").as_str() {
        "tool_start" => {
            *current = string(e, "id");
            pending.insert(current.clone(), String::new());
        }
        "tool_input" => {
            let id = e["id"].as_str().unwrap_or(current);
            pending
                .entry(id.to_string())
                .or_default()
                .push_str(e["delta"].as_str().unwrap_or(""));
        }
        "tool_done" => {
            let name = string(e, "name");
            let input = pending.remove(&string(e, "id")).unwrap_or_default();
            if ["integration_tools", "discover_tools"].contains(&name.as_str()) {
                let mut call = parse_output(&string(e, "output"), elapsed);
                let (action, tool, reason) = tool_input(&input);
                call["input"] = json!(input.chars().take(2000).collect::<String>());
                call["input_action"] = action.clone();
                call["input_tool"] = tool.clone();
                call["input_reason"] = reason.clone();
                if v["first_call_seconds"].is_null() {
                    v["first_call_seconds"] = json!(round(elapsed, 3))
                }
                if ["listing", "empty"].contains(&string(&call, "outcome").as_str()) {
                    v["browsed"] = json!(true);
                    if !call["category"].is_null() {
                        push(v, "browse_categories", call["category"].clone());
                    }
                }
                let selection = action == "select" || call["outcome"] == "selection";
                if action == "select" && !tool.is_null() {
                    push(v, "selected_via_discovery", tool.clone());
                }
                if call["outcome"] == "selection" && call["listed"] == false {
                    for t in array(&call["tools"]) {
                        push(v, "off_catalog_selects", t.clone());
                    }
                }
                if case["expect"] == "select" && selection {
                    v["selection_correct"] = json!(correct(case, &call, &action, &tool, &reason));
                }
                push(v, "discovery_calls", call);
                return case["expect"] == "no-call" || selection;
            } else {
                push(v, "other_tool_calls", json!(name));
                for b in bypasses(&name, &input, elapsed) {
                    push(v, "bypasses", b);
                }
            }
        }
        _ => {}
    }
    false
}
pub(super) fn finish(v: &mut Value, case: &Value) {
    if !case["expected_category"].is_null() && !array(&v["browse_categories"]).is_empty() {
        v["category_correct"] =
            json!(array(&v["browse_categories"]).contains(&case["expected_category"]));
    }
    if array(&v["discovery_calls"]).is_empty() && array(&v["other_tool_calls"]).is_empty() {
        if v["exit_code"] != 0
            && !v["exit_code"].is_null()
            && let Some(m) = regex(INVALID).find(&string(v, "stderr_tail"))
        {
            v["invalid_reason"] = json!(m.as_str().to_lowercase())
        }
        if v["invalid_reason"].is_null()
            && v["timed_out"] == true
            && v["elapsed_seconds"].as_f64().unwrap_or(0.) < 5.
        {
            v["invalid_reason"] = json!("no-model-activity")
        }
    }
    let outcome = if !v["invalid_reason"].is_null() {
        "invalid"
    } else if case["expect"] == "no-call" {
        if array(&v["discovery_calls"]).is_empty() {
            "clean"
        } else {
            "false-positive"
        }
    } else if case["expect"] == "select" {
        if v["selection_correct"] == true {
            "selected"
        } else if v["selection_correct"] == false {
            "incorrect-selection"
        } else if !array(&v["bypasses"]).is_empty() {
            "bypassed"
        } else {
            "no-selection"
        }
    } else if v["browsed"] == true {
        "browsed"
    } else if !array(&v["selected_via_discovery"]).is_empty() {
        "select-without-browse"
    } else if !array(&v["off_catalog_selects"]).is_empty() {
        "off-catalog-select"
    } else if !array(&v["bypasses"]).is_empty() {
        "bypassed"
    } else {
        "no-call"
    };
    v["outcome"] = json!(outcome);
}
fn trial(s: &runtime::Server, a: &Args, case: &Value, n: usize, model: &str) -> Result<Value> {
    let workspace = tempfile::Builder::new()
        .prefix("trial-")
        .tempdir_in(s.root.path())?;
    std::fs::write(
        workspace.path().join("README.md"),
        "# scratch project\n\nA small project used for one benchmark task.\n",
    )?;
    let mut c = s.model_command(
        model,
        a.provider.as_deref(),
        workspace.path(),
        &string(case, "prompt"),
        false,
    );
    let mut v = empty_trial(n);
    let mut pending = HashMap::new();
    let mut current = String::new();
    let meta = runtime::stream(&mut c, a.timeout, |e, t| {
        event(&mut v, case, &e, t, &mut pending, &mut current)
    })?;
    for (k, x) in meta.as_object().unwrap() {
        v[k] = x.clone();
    }
    finish(&mut v, case);
    Ok(v)
}
pub(super) fn summary(case: &Value, trials: Vec<Value>) -> Value {
    let scored: Vec<_> = trials
        .iter()
        .filter(|t| t["invalid_reason"].is_null())
        .collect();
    let n = scored.len();
    let rate = |predicate: fn(&Value) -> bool| -> Value {
        if n == 0 {
            Value::Null
        } else {
            json!(scored.iter().filter(|t| predicate(t)).count() as f64 / n as f64)
        }
    };
    let wanted = match string(case, "expect").as_str() {
        "call" => "browsed",
        "no-call" => "clean",
        _ => "selected",
    };
    let mut outcomes = BTreeMap::<String, usize>::new();
    let mut kinds = BTreeSet::new();
    for t in &trials {
        *outcomes.entry(string(t, "outcome")).or_default() += 1;
        for b in array(&t["bypasses"]) {
            kinds.insert(string(b, "kind"));
        }
    }
    let category: Vec<_> = scored
        .iter()
        .filter_map(|t| {
            t["category_correct"]
                .as_bool()
                .map(|v| if v { 1. } else { 0. })
        })
        .collect();
    json!({"case":case,"trial_count":trials.len(),"scored_trial_count":n,"invalid_trial_count":trials.len()-n,"passed":n>0&&scored.iter().all(|t|t["outcome"]==wanted),"call_rate":rate(|t|!array(&t["discovery_calls"]).is_empty()),"browse_rate":rate(|t|t["browsed"]==true),"bypass_rate":rate(|t|!array(&t["bypasses"]).is_empty()&&array(&t["discovery_calls"]).is_empty()),"select_rate":rate(|t|!array(&t["selected_via_discovery"]).is_empty()),"selection_accuracy":if case["expect"]=="select"{rate(|t|t["selection_correct"]==true)}else{Value::Null},"category_accuracy":mean(category,9),"median_first_call_seconds":median(scored.iter().filter_map(|t|t["first_call_seconds"].as_f64()).collect()),"outcomes":outcomes,"bypass_kinds":kinds,"trials":trials})
}
pub(super) fn aggregate(results: &[Value]) -> Value {
    let calls: Vec<_> = results
        .iter()
        .filter(|r| r["case"]["expect"] == "call")
        .collect();
    let controls: Vec<_> = results
        .iter()
        .filter(|r| r["case"]["expect"] == "no-call")
        .collect();
    let selects: Vec<_> = results
        .iter()
        .filter(|r| r["case"]["expect"] == "select")
        .collect();
    let avg =
        |rows: &[&Value], key: &str| mean(rows.iter().filter_map(|r| r[key].as_f64()).collect(), 4);
    let mut worst: Vec<_> = calls
        .iter()
        .filter(|r| r["browse_rate"].is_number())
        .copied()
        .collect();
    worst.sort_by(|a, b| {
        a["browse_rate"]
            .as_f64()
            .unwrap()
            .total_cmp(&b["browse_rate"].as_f64().unwrap())
    });
    json!({"call_case_count":calls.len(),"control_case_count":controls.len(),"select_case_count":selects.len(),"invalid_trial_count":results.iter().map(|r|r["invalid_trial_count"].as_u64().unwrap()).sum::<u64>(),"scored_trial_count":results.iter().map(|r|r["scored_trial_count"].as_u64().unwrap()).sum::<u64>(),"recall_browse_rate":avg(&calls,"browse_rate"),"recall_any_call_rate":avg(&calls,"call_rate"),"bypass_rate":avg(&calls,"bypass_rate"),"select_rate":avg(&calls.iter().chain(&selects).copied().collect::<Vec<_>>(),"select_rate"),"selection_accuracy":avg(&selects,"selection_accuracy"),"category_accuracy":avg(&calls,"category_accuracy"),"control_clean_rate":mean(controls.iter().filter_map(|r|r["call_rate"].as_f64().map(|n|1.-n)).collect(),4),"worst_call_cases":worst.iter().take(5).map(|r|&r["case"]["id"]).collect::<Vec<_>>(),"failing_controls":controls.iter().filter(|r|r["call_rate"].as_f64().unwrap_or(0.)>0.).map(|r|&r["case"]["id"]).collect::<Vec<_>>()})
}
pub(super) fn gates(s: &Value, recall: f64, precision: f64, selection: f64) -> bool {
    s["scored_trial_count"].as_u64().unwrap_or(0) > 0
        && [
            ("recall_browse_rate", recall),
            ("control_clean_rate", precision),
            ("selection_accuracy", selection),
        ]
        .iter()
        .all(|(k, min)| s[k].as_f64().is_none_or(|v| v >= *min))
}
pub fn run(args: impl Iterator<Item = String>) -> Result<i32> {
    let mut a =
        Args::try_parse_from(std::iter::once("benchmark-discovery-rate".into()).chain(args))?;
    ensure!(
        a.trials > 0 && a.timeout.is_finite() && a.timeout > 0.,
        "invalid trials or timeout"
    );
    for n in [a.retry_delay, a.invalid_backoff] {
        ensure!(n.is_finite() && n >= 0., "invalid delay");
    }
    for n in [a.min_recall, a.min_precision, a.min_selection_accuracy] {
        ensure!((0.0..=1.0).contains(&n), "gate must be between 0 and 1");
    }
    let started = chrono::Utc::now();
    let root = crate::root()?;
    let path = a
        .cases
        .clone()
        .unwrap_or(root.join("scripts/discovery_rate_cases.json"));
    let chosen = filter(cases(&path, true, &categories()?)?, &a.ids, &a.tags, true)?;
    if a.list {
        for c in &chosen {
            println!(
                "{} {} {}",
                string(c, "expect"),
                string(c, "id"),
                string(c, "prompt")
            );
        }
        println!("{} cases", chosen.len());
        return Ok(0);
    }
    let executable = identity(
        &a.nexode
            .clone()
            .or_else(|| std::env::var_os("NEXODE_BIN").map(PathBuf::from))
            .unwrap_or("nexode".into()),
    )?;
    let model = a
        .model
        .clone()
        .or_else(|| std::env::var("NEXODE_DISCOVERY_BENCHMARK_MODEL").ok())
        .unwrap_or("gpt-5.6-sol".into());
    a.provider = a
        .provider
        .or_else(|| std::env::var("NEXODE_DISCOVERY_BENCHMARK_PROVIDER").ok());
    let s = runtime::Server::start(
        Path::new(executable["path"].as_str().unwrap()),
        a.provider.as_deref(),
    )?;
    let mut results = vec![];
    for case in &chosen {
        let mut trials = vec![];
        for n in 1..=a.trials {
            let mut t = trial(&s, &a, case, n, &model)?;
            for retry in 1..=a.invalid_retries {
                if t["invalid_reason"].is_null() {
                    break;
                }
                std::thread::sleep(Duration::from_secs_f64(a.invalid_backoff * retry as f64));
                t = trial(&s, &a, case, n, &model)?;
            }
            println!(
                "NEXODE_PROGRESS {}",
                json!({"current":n,"total":a.trials,"unit":"trials","message":format!("{}: {}",string(case,"id"),string(&t,"outcome"))})
            );
            trials.push(t);
            std::thread::sleep(Duration::from_secs_f64(a.retry_delay));
        }
        results.push(summary(case, trials));
    }
    drop(s);
    let summary = aggregate(&results);
    let passed = gates(
        &summary,
        a.min_recall,
        a.min_precision,
        a.min_selection_accuracy,
    );
    let report = json!({"benchmark":"discovery-call-rate","version":2,"started_at":started.to_rfc3339(),"finished_at":chrono::Utc::now().to_rfc3339(),"benchmark_marker":marker(),"config":{"executable":executable,"model":model,"provider":a.provider,"trials":a.trials,"timeout_seconds":a.timeout,"cases_file":path,"min_recall":a.min_recall,"min_precision":a.min_precision,"min_selection_accuracy":a.min_selection_accuracy},"summary":summary,"results":results,"passed":passed});
    let output = a
        .output
        .unwrap_or(root.join("target/discovery-rate/latest.json"));
    write_report(&output, &report)?;
    println!("{}", serde_json::to_string_pretty(&report["summary"])?);
    println!("Report: {}", output.display());
    Ok(i32::from(!passed))
}
pub fn compare(mut args: impl Iterator<Item = String>) -> Result<i32> {
    let before = args.next().context("before report required")?;
    let after = args.next().context("after report required")?;
    ensure!(
        args.next().is_none(),
        "usage: compare-discovery-rate BEFORE.json AFTER.json"
    );
    let read = |p| -> Result<Value> { Ok(serde_json::from_slice(&std::fs::read(p)?)?) };
    let a = read(before)?;
    let b = read(after)?;
    println!("{:<20} {:>10} {:>10}", "metric", "before", "after");
    for (key, label) in [
        ("recall_browse_rate", "browse recall"),
        ("recall_any_call_rate", "any call"),
        ("bypass_rate", "bypassed"),
        ("select_rate", "reached select"),
        ("control_clean_rate", "controls clean"),
        ("scored_trial_count", "scored trials"),
        ("invalid_trial_count", "invalid trials"),
    ] {
        let fmt = |v: &Value| {
            if let Some(n) = v.as_f64() {
                if key.ends_with("count") {
                    format!("{n:.0}")
                } else {
                    format!("{:.0}%", n * 100.)
                }
            } else {
                "n/a".into()
            }
        };
        println!(
            "{label:<20} {:>10} {:>10}",
            fmt(&a["summary"][key]),
            fmt(&b["summary"][key])
        );
    }
    Ok(0)
}
