use anyhow::{Context, Result, ensure};
use regex::Regex;
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};
fn unlocked(source: &str) -> Vec<String> {
    let begins = Regex::new(r"^\s*(?:async\s+)?fn\s+([A-Za-z0-9_]+)").unwrap();
    let mutation = Regex::new(r#"(?:set_var|set_path|remove_var)\s*\(\s*"NEXODE_HOME""#).unwrap();
    let lock = Regex::new(r"lock_test_env\s*\(\)").unwrap();
    let mut functions: Vec<(String, String)> = Vec::new();
    for line in source.lines() {
        if let Some(c) = begins.captures(line) {
            functions.push((c[1].to_owned(), line.to_owned()));
        } else if let Some((_, body)) = functions.last_mut() {
            body.push('\n');
            body.push_str(line);
        }
    }
    let helpers: Vec<_> = functions
        .iter()
        .filter(|(_, s)| lock.is_match(s))
        .map(|(n, _)| Regex::new(&format!(r"\b{}\s*\(", regex::escape(n))).unwrap())
        .collect();
    functions
        .iter()
        .filter(|(_, s)| {
            mutation.is_match(s) && !lock.is_match(s) && !helpers.iter().any(|h| h.is_match(s))
        })
        .map(|(n, _)| n.clone())
        .collect()
}
pub fn env_tests(path: &Path) -> Result<()> {
    ensure!(
        path.is_dir(),
        "source directory does not exist: {}",
        path.display()
    );
    let mut files = Vec::new();
    crate::safety_scan::rust_files(path, &mut files)?;
    let mut count = 0;
    for path in files {
        let source = fs::read_to_string(&path)?;
        if source.contains("NEXODE_HOME") {
            for f in unlocked(&source) {
                println!("{}: {f}", path.display());
                count += 1;
            }
        }
    }
    println!("\n{count} function(s) mutate NEXODE_HOME without the shared test-env lock");
    Ok(())
}
fn baseline(path: &Path) -> Result<BTreeMap<String, f64>> {
    let mut result = BTreeMap::new();
    for line in fs::read_to_string(path)?
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.starts_with('#'))
    {
        let fields: Vec<_> = line.split('\t').collect();
        if fields.len() >= 4 {
            let rate = fields[3].parse::<f64>()?;
            ensure!(rate.is_finite(), "invalid baseline rate");
            result.insert(fields[0].into(), rate);
        }
    }
    ensure!(!result.is_empty(), "baseline has no task results");
    Ok(result)
}
fn collect(dir: &Path, results: &mut BTreeMap<String, Vec<f64>>) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_dir() {
            collect(&entry.path(), results)?;
        } else if kind.is_file() && entry.file_name() == "result.json" {
            let data: Value = match fs::read(entry.path())
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok())
            {
                Some(d) => d,
                None => continue,
            };
            let Some(evals) = data.pointer("/stats/evals").and_then(Value::as_object) else {
                continue;
            };
            for ev in evals.values() {
                let Some(rewards) = ev
                    .pointer("/reward_stats/reward")
                    .and_then(Value::as_object)
                else {
                    continue;
                };
                for (reward, trials) in rewards {
                    let Ok(reward) = reward.parse::<f64>() else {
                        continue;
                    };
                    if let Some(trials) = trials.as_array() {
                        for trial in trials.iter().filter_map(Value::as_str) {
                            let task = trial.split("__").next().context("empty trial")?;
                            results.entry(task.into()).or_default().push(reward);
                        }
                    }
                }
            }
        }
    }
    Ok(())
}
pub fn compare(jobs: &Path, baseline_path: &Path) -> Result<()> {
    let base = baseline(baseline_path)?;
    let mut results = BTreeMap::new();
    collect(jobs, &mut results)?;
    let tasks: BTreeSet<_> = base.keys().chain(results.keys()).collect();
    let mut regressions = Vec::new();
    let mut resolved = 0;
    let mut trials = 0;
    println!(
        "{:38} {:>7} {:>7} {:>6} flag",
        "task", "base%", "nexode%", "pass"
    );
    println!("{}", "-".repeat(75));
    for task in tasks {
        let values = results.get(task).map(Vec::as_slice).unwrap_or(&[]);
        let n = values.len();
        let passed = values.iter().filter(|v| **v >= 1.0).count();
        let rate = if n > 0 {
            Some(100.0 * passed as f64 / n as f64)
        } else {
            None
        };
        let old = base.get(task);
        resolved += passed;
        trials += n;
        let flag = match (rate, old) {
            (Some(r), Some(b)) if r < *b => {
                regressions.push((task, *b, r));
                "REGRESSION"
            }
            (Some(r), Some(b)) if r > *b => "gain",
            _ => "",
        };
        println!(
            "{task:38} {:>7} {:>7} {:>6} {flag}",
            old.map(|n| format!("{n:.1}")).unwrap_or_else(|| "-".into()),
            rate.map(|n| format!("{n:.1}"))
                .unwrap_or_else(|| "n/a".into()),
            if n > 0 {
                format!("{passed}/{n}")
            } else {
                "-".into()
            }
        );
    }
    println!("{}", "-".repeat(75));
    if trials > 0 {
        println!(
            "nexode micro-avg: {resolved}/{trials} = {:.1}%",
            100.0 * resolved as f64 / trials as f64
        );
    }
    println!(
        "baseline macro-avg: {:.1}% (CC+Opus4.8)",
        base.values().sum::<f64>() / base.len() as f64
    );
    if regressions.is_empty() {
        println!("\nNo per-task regressions detected (for tasks with results).");
    } else {
        println!("\n{} REGRESSIONS vs baseline:", regressions.len());
        for (task, b, r) in regressions {
            println!("  {task}: {b:.1}% -> {r:.1}%");
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lock_helpers_and_unlocked_functions() {
        let source = "fn helper() { lock_test_env(); }\nasync fn protected() { helper(); set_var(\"NEXODE_HOME\", \"a\"); }\nfn unsafe_test() { remove_var(\"NEXODE_HOME\"); }";
        assert_eq!(unlocked(source), vec!["unsafe_test"]);
    }
    #[test]
    fn nested_harbor_results_keep_trials_and_ignore_malformed_reports() {
        let t = tempfile::tempdir().unwrap();
        fs::create_dir(t.path().join("nested")).unwrap();
        fs::write(t.path().join("result.json"), "invalid").unwrap();
        fs::write(t.path().join("nested/result.json"),serde_json::json!({"stats":{"evals":{"suite":{"reward_stats":{"reward":{"1":["task__a","task__b"],"0":["other__x"],"bad":["ignored__z"]}}}}}}).to_string()).unwrap();
        let mut result = BTreeMap::new();
        collect(t.path(), &mut result).unwrap();
        assert_eq!(result["task"], vec![1.0, 1.0]);
        assert_eq!(result["other"], vec![0.0]);
        assert!(!result.contains_key("ignored"));
    }
}
