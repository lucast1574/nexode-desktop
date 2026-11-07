//! Native live Discovery benchmarks; all policy scoring is testable offline.
mod listing;
mod rate;
mod runtime;
#[cfg(test)]
mod tests;
use anyhow::{Context, Result, ensure};
pub use listing::run;
pub use rate::{compare, run as rate};
use regex::Regex;
use serde_json::{Value, json};
use std::{collections::BTreeSet, path::Path};
fn string(v: &Value, key: &str) -> String {
    v[key].as_str().unwrap_or("").trim().to_string()
}
fn lower(v: &Value, key: &str) -> String {
    string(v, key).to_lowercase()
}
fn optional(s: String) -> Value {
    if s.is_empty() { Value::Null } else { json!(s) }
}
fn array(v: &Value) -> &[Value] {
    v.as_array().map(Vec::as_slice).unwrap_or(&[])
}
fn round(n: f64, d: i32) -> f64 {
    let m = 10f64.powi(d);
    (n * m).round() / m
}
fn median(mut n: Vec<f64>) -> Value {
    if n.is_empty() {
        return Value::Null;
    }
    n.sort_by(f64::total_cmp);
    let k = n.len() / 2;
    json!(round(
        if n.len().is_multiple_of(2) {
            (n[k - 1] + n[k]) / 2.
        } else {
            n[k]
        },
        3
    ))
}
fn mean(n: Vec<f64>, digits: i32) -> Value {
    if n.is_empty() {
        Value::Null
    } else {
        json!(round(n.iter().sum::<f64>() / n.len() as f64, digits))
    }
}
fn regex(pattern: &str) -> Regex {
    Regex::new(pattern).expect("constant regex")
}
fn categories() -> Result<Vec<String>> {
    let text = std::fs::read_to_string(crate::root()?.join("crates/nexode-base/src/sponsors.rs"))?;
    let re = regex(r"(?s)pub const DISCOVERY_CATEGORIES: &\[&str\] = &\[(.*?)\];");
    let body = re
        .captures(&text)
        .context("DISCOVERY_CATEGORIES not found")?[1]
        .to_string();
    Ok(regex(r#""([a-z0-9-]+)""#)
        .captures_iter(&body)
        .map(|c| c[1].to_string())
        .collect())
}
fn cases(path: &Path, rate: bool, categories: &[String]) -> Result<Vec<Value>> {
    let value: Value = serde_json::from_slice(&std::fs::read(path)?)?;
    ensure!(
        value["version"] == 1 || (!rate && value["version"] == 2),
        "unsupported cases version"
    );
    let rows = value["cases"]
        .as_array()
        .context("cases must be an array")?;
    let (mut ids, mut prompts) = (BTreeSet::new(), BTreeSet::new());
    let mut result = vec![];
    for raw in rows {
        let id = string(raw, "id");
        let prompt = string(raw, "prompt");
        let category = lower(raw, "expected_category");
        let tool = lower(raw, "expected_tool");
        ensure!(!id.is_empty() && !prompt.is_empty(), "empty id or prompt");
        ensure!(ids.insert(id.clone()), "duplicate case id {id}");
        let normalized = prompt
            .to_lowercase()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        ensure!(prompts.insert(normalized.clone()), "duplicate prompt {id}");
        let mut leaks = vec!["discover_tools", "integration_tools", "tool discovery"];
        if rate {
            leaks.extend(["discovery tool", "catalog"])
        }
        ensure!(
            !leaks.iter().any(|s| normalized.contains(s)),
            "case {id} leaks Discovery"
        );
        let c = if rate {
            let expect = lower(raw, "expect");
            ensure!(
                ["call", "no-call", "select"].contains(&expect.as_str()),
                "invalid expect"
            );
            if expect == "select" {
                ensure!(
                    !category.is_empty() && !tool.is_empty() && raw["expected_listed"].is_boolean(),
                    "select requires category, tool and boolean expected_listed"
                )
            } else {
                ensure!(
                    tool.is_empty() && raw["expected_listed"].is_null(),
                    "only select cases may declare selection fields"
                )
            }
            ensure!(
                expect != "no-call" || category.is_empty(),
                "control must not declare category"
            );
            ensure!(
                category.is_empty() || categories.contains(&category),
                "unknown category {category}"
            );
            ensure!(
                !categories
                    .iter()
                    .any(|c| c != "other" && normalized.contains(&c.replace('-', " "))),
                "case {id} leaks category slug"
            );
            let tags: Vec<_> = array(&raw["tags"])
                .iter()
                .map(|v| v.as_str().unwrap_or("").trim().to_lowercase())
                .collect();
            json!({"id":id,"expect":expect,"prompt":prompt,"expected_category":optional(category),"expected_tool":optional(tool),"expected_listed":raw["expected_listed"],"tags":tags})
        } else {
            let expect = raw["expectation"]
                .as_str()
                .unwrap_or("listing")
                .trim()
                .to_lowercase();
            ensure!(
                ["listing", "no-discovery"].contains(&expect.as_str()),
                "invalid expectation"
            );
            ensure!(
                if expect == "listing" {
                    !category.is_empty() && !tool.is_empty()
                } else {
                    category.is_empty() && tool.is_empty()
                },
                "listing requires target; control must not declare target"
            );
            ensure!(
                tool.is_empty() || !normalized.contains(&tool),
                "case {id} leaks expected tool"
            );
            json!({"id":id,"expectation":expect,"prompt":prompt,"expected_category":optional(category),"expected_tool":optional(tool)})
        };
        result.push(c);
    }
    ensure!(!result.is_empty(), "no cases");
    Ok(result)
}
fn filter(cases: Vec<Value>, ids: &[String], tags: &[String], rate: bool) -> Result<Vec<Value>> {
    let wanted: BTreeSet<_> = ids.iter().map(|s| s.to_lowercase()).collect();
    let selected: Vec<_> = cases
        .into_iter()
        .filter(|c| {
            wanted.is_empty()
                || wanted.contains(&lower(c, "id"))
                || (!rate && wanted.contains(&lower(c, "expected_tool")))
        })
        .collect();
    for w in wanted {
        ensure!(
            selected
                .iter()
                .any(|c| lower(c, "id") == w || (!rate && lower(c, "expected_tool") == w)),
            "unknown --case {w}"
        )
    }
    let selected: Vec<_> = selected
        .into_iter()
        .filter(|c| {
            tags.is_empty()
                || tags
                    .iter()
                    .any(|t| array(&c["tags"]).contains(&json!(t.to_lowercase())))
        })
        .collect();
    ensure!(!selected.is_empty(), "case filters selected nothing");
    Ok(selected)
}
fn parse_output(output: &str, elapsed: f64) -> Value {
    let patterns = [
        (
            r"(?:Discoverable tools|Available integrations) in '([^']+)'",
            "listing",
        ),
        (
            r"No (?:discoverable tools|integrations) in category '([^']+)'",
            "empty",
        ),
        (r"(?:Selected|Set up) '([^']+)' from '([^']+)'", "selection"),
        (
            r"Selected off-catalog product '([^']+)' for '([^']+)'",
            "selection",
        ),
    ];
    let mut v = json!({"elapsed_seconds":round(elapsed,3),"category":null,"tools":[],"outcome":if output.starts_with("Error:"){"error"}else{"other"},"output":output.chars().take(4000).collect::<String>(),"listed":null});
    for (i, (pattern, outcome)) in patterns.iter().enumerate() {
        if let Some(c) = regex(pattern).captures(output) {
            v["outcome"] = json!(outcome);
            v["category"] = json!(&c[if i < 2 { 1 } else { 2 }]);
            if i == 0 {
                v["tools"] = json!(
                    regex(r"(?m)^- ([^:\n]+):")
                        .captures_iter(output)
                        .map(|c| c[1].trim().to_lowercase())
                        .collect::<Vec<_>>()
                );
            }
            if i >= 2 {
                v["tools"] = json!([c[1].trim().to_lowercase()]);
                v["listed"] = json!(i == 2);
            }
            break;
        }
    }
    v
}
fn write_report(path: &Path, value: &Value) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    use std::io::Write;
    writeln!(temp, "{}", serde_json::to_string_pretty(value)?)?;
    temp.persist(path)?;
    Ok(())
}
fn marker() -> Value {
    json!({"environment":"NEXODE_DISCOVERY_BENCHMARK=1","request_header":"x-nexode-discovery-benchmark: 1","telemetry_field":"benchmark_run=true"})
}
