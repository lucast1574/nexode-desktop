use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
fn rows(v: &Value) -> &[Value] {
    v.as_array().map(Vec::as_slice).unwrap_or(&[])
}
fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap_or("")
}
pub(super) fn timing(v: &Value, path: &str, out: &mut serde_json::Map<String, Value>) {
    if let Some(o) = v.as_object() {
        for (k, child) in o {
            let p = if path.is_empty() {
                k.clone()
            } else {
                format!("{path}.{k}")
            };
            if ["timing", "elapsed", "duration", "latency"]
                .iter()
                .any(|n| k.to_lowercase().contains(n))
            {
                out.insert(p, child.clone());
            } else {
                timing(child, &p, out)
            }
        }
    } else if let Some(a) = v.as_array() {
        for (i, child) in a.iter().enumerate() {
            timing(child, &format!("{path}[{i}]"), out)
        }
    }
}
pub(super) fn extract(text: &str) -> Value {
    let mut calls = serde_json::Map::<String, Value>::new();
    let mut current = String::new();
    let mut final_text = String::new();
    let mut errors = vec![];
    let mut tokens = vec![];
    let mut usage = Value::Null;
    let mut model_count = Value::Null;
    let mut cost = Value::Null;
    for line in text.lines() {
        let Ok(e) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        match s(&e, "type") {
            "tokens" => tokens.push(e),
            "done" => {
                if let Some(t) = e["text"].as_str() {
                    final_text = t.into()
                }
                usage = e["usage"].clone();
                model_count = e["model_call_count"].clone();
                cost = e["actual_cost_usd"].clone();
            }
            "tool_start" => {
                current = s(&e, "id").into();
                calls.insert(
                    current.clone(),
                    json!({"id":current,"name":e["name"],"input_text":"","executed":false}),
                );
            }
            "tool_input" => {
                let id = e["id"].as_str().unwrap_or(&current);
                if let Some(c) = calls.get_mut(id) {
                    let mut t = s(c, "input_text").to_string();
                    t.push_str(s(&e, "delta"));
                    c["input_text"] = json!(t);
                }
            }
            "tool_exec" | "tool_done" => {
                let id = s(&e, "id");
                let c = calls
                    .entry(id.to_string())
                    .or_insert(json!({"id":id,"name":e["name"],"input_text":""}));
                c["executed"] = json!(true);
                if e["type"] == "tool_done" {
                    c["error"] = e["error"].clone();
                    let output: Value =
                        serde_json::from_str(s(&e, "output")).unwrap_or(Value::Null);
                    c["decision_provider"] = output["decision_provider"].clone();
                    if output.is_object() {
                        c["handoff_status"] = output["status"].clone();
                        let mut timings = serde_json::Map::new();
                        timing(&output, "", &mut timings);
                        c["timing_instrumentation"] = timings.into();
                        c["handoff_executed_steps"] = json!(
                            rows(&output["action_trace"])
                                .iter()
                                .filter(|v| v["status"] == "executed")
                                .count()
                        );
                    }
                }
            }
            "text_delta" => final_text.push_str(s(&e, "text")),
            "text_replace" => final_text = s(&e, "text").into(),
            "error" => errors.push(e),
            _ => {}
        }
    }
    let count = calls.values().filter(|c| c["executed"] == true).count();
    let mut browser = vec![];
    let mut other = vec![];
    for (_, mut c) in calls {
        let input = c
            .as_object_mut()
            .unwrap()
            .remove("input_text")
            .unwrap_or(Value::Null);
        let input: Value =
            serde_json::from_str(input.as_str().unwrap_or("")).unwrap_or(Value::Null);
        if s(&c, "name").rsplit('.').next() == Some("browser") {
            c["action"] = input["action"].clone();
            browser.push(c)
        } else if c["executed"] == true {
            other.push(c["name"].clone())
        }
    }
    json!({"browser_calls":browser,"other_tools":other,"final_text":final_text,"errors":errors,"parent_tool_count":count,"model_call_count":model_count,"usage_tokens":usage,"usage_tokens_scope":"last reported snapshot, not task total","actual_cost_usd":cost,"token_events":tokens})
}
pub(super) fn handoff(trace: &Value) -> Value {
    let calls: Vec<_> = rows(&trace["browser_calls"])
        .iter()
        .filter(|c| c["executed"] == true && c["action"] == "handoff")
        .collect();
    json!({"handoff_effective":calls.iter().any(|c|(c["error"].is_null()||c["error"]==false)&&(c["handoff_status"]=="done"||c["handoff_executed_steps"].as_u64().unwrap_or(0)>0)),"handoff_done_calls":calls.iter().filter(|c|c["handoff_status"]=="done"&&(c["error"].is_null()||c["error"]==false)).count(),"handoff_executed_steps":calls.iter().map(|c|c["handoff_executed_steps"].as_u64().unwrap_or(0)).sum::<u64>(),"handoff_statuses":calls.iter().map(|c|&c["handoff_status"]).collect::<Vec<_>>()})
}
fn median(mut values: Vec<f64>) -> Value {
    if values.is_empty() {
        return Value::Null;
    }
    values.sort_by(f64::total_cmp);
    let n = values.len();
    json!(if n.is_multiple_of(2) {
        (values[n / 2 - 1] + values[n / 2]) / 2.
    } else {
        values[n / 2]
    })
}
pub(super) fn summary(results: &[Value]) -> Value {
    let comparison = if results.iter().any(|r| r["mode"] == "jev") {
        "jev"
    } else {
        "normal"
    };
    let mut pairs = BTreeMap::<(String, String, u64), BTreeMap<String, &Value>>::new();
    for r in results {
        let phase = r["phase"].as_str().unwrap_or("discovery");
        let task = r["task"].as_str().unwrap_or("navigation");
        pairs
            .entry((phase.into(), task.into(), r["pair"].as_u64().unwrap_or(0)))
            .or_default()
            .insert(s(r, "mode").into(), r);
    }
    let complete: Vec<_> = pairs
        .values()
        .filter(|p| p.contains_key(comparison) && p.contains_key("direct"))
        .collect();
    let eligible: Vec<_> = complete
        .iter()
        .filter(|p| {
            p[comparison]["valid_success"] == true
                && p["direct"]["valid_success"] == true
                && p[comparison]["handoff_used"] == true
                && p[comparison]["handoff_effective"] == true
                && p["direct"]["handoff_used"] != true
                && p[comparison]["elapsed_seconds"].as_f64().unwrap_or(0.) > 0.
        })
        .collect();
    let ratio = |p: &&BTreeMap<String, &Value>| {
        p["direct"]["elapsed_seconds"].as_f64().unwrap_or(0.)
            / p[comparison]["elapsed_seconds"].as_f64().unwrap()
    };
    let ratios: Vec<_> = eligible.iter().map(|p| ratio(p)).collect();
    let all: Vec<_> = complete
        .iter()
        .filter(|p| p[comparison]["elapsed_seconds"].as_f64().unwrap_or(0.) > 0.)
        .map(ratio)
        .collect();
    let med = median(ratios.clone());
    let allmed = median(all.clone());
    let mut arms = json!({});
    let mut successes = json!({});
    for mode in [comparison, "direct"] {
        let rs: Vec<_> = results.iter().filter(|r| r["mode"] == mode).collect();
        let times: Vec<_> = rs
            .iter()
            .filter_map(|r| r["elapsed_seconds"].as_f64())
            .collect();
        let success = rs.iter().filter(|r| r["valid_success"] == true).count();
        successes[mode] = json!(success);
        arms[mode] = json!({"attempts":rs.len(),"successes":success,"success_rate":if rs.is_empty(){Value::Null}else{json!(success as f64/rs.len() as f64)},"failures":rs.len()-success,"timeouts":rs.iter().filter(|r|r["timed_out"]==true).count(),"protocol_violations":rs.iter().filter(|r|r["protocol_compliant"]==false).count(),"handoff_violations":if mode=="direct"{rs.iter().filter(|r|r["handoff_used"]==true).count()}else{0},"all_attempt_seconds":times,"median_all_attempt_seconds":median(times.clone())});
    }
    let normal: Vec<_> = results.iter().filter(|r| r["mode"] == "normal").collect();
    let target = comparison == "jev"
        && complete.len() >= 3
        && eligible.len() >= 3
        && med.as_f64().is_some_and(|n| n >= 2.)
        && allmed.as_f64().is_some_and(|n| n >= 2.)
        && successes[comparison].as_u64() >= successes["direct"].as_u64();
    let mut summary = json!({"type":"summary","comparison_arm":comparison,"trials":results.len(),"arms":arms,"eligible_speed_pairs":eligible.len(),"valid_successes":successes,"normal_handoff_rate":normal.iter().filter(|r|r["handoff_used"]==true).count() as f64/normal.len().max(1) as f64,"paired_direct_over_handoff_ratios":ratios,"median_direct_over_handoff_ratio":med,"all_attempt_paired_ratios":all,"median_all_attempt_paired_ratio":allmed,"target_met":target,"note":"Success-only ratios exclude failures, which remain in all-attempt timing and counts. Timeout timings are censored. No inferred model calls, token totals, or prices."});
    let groups: BTreeSet<_> = pairs
        .keys()
        .map(|(p, t, _)| (p.clone(), t.clone()))
        .collect();
    if groups.len() > 1 {
        let mut sub = json!({});
        for (phase, task) in &groups {
            let rs: Vec<_> = results
                .iter()
                .filter(|r| {
                    r["phase"].as_str().unwrap_or("discovery") == phase
                        && r["task"].as_str().unwrap_or("navigation") == task
                })
                .cloned()
                .collect();
            sub[format!("{phase}/{task}")] = self::summary(&rs);
        }
        summary["target_met"] = json!(
            sub.as_object()
                .unwrap()
                .values()
                .all(|v| v["target_met"] == true)
        );
        summary["by_phase_task"] = sub;
    }
    let full: BTreeSet<_> = ["discovery", "heldout"]
        .iter()
        .flat_map(|p| {
            ["navigation", "search", "form"]
                .iter()
                .map(move |t| (p.to_string(), t.to_string()))
        })
        .collect();
    summary["full_suite_target_met"] = json!(summary["target_met"] == true && groups == full);
    summary
}
