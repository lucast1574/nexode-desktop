//! Score exported todo plans. Paths are explicit; no maintainer-local directories.
use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, path::Path};
fn score(row: &Value) -> Value {
    let todos = row["todos"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    let calls = row["todo_calls"].as_i64().unwrap_or(0);
    let completed = todos
        .iter()
        .filter(|t| {
            t["status"]
                .as_str()
                .unwrap_or_default()
                .eq_ignore_ascii_case("completed")
        })
        .count();
    let pending = todos.iter().any(|t| {
        ["pending", "in_progress"].contains(
            &t["status"]
                .as_str()
                .unwrap_or_default()
                .to_lowercase()
                .as_str(),
        )
    });
    let contents: Vec<_> = todos
        .iter()
        .filter_map(|t| t["content"].as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    let words = [
        "map",
        "trace",
        "audit",
        "inspect",
        "verify",
        "enumerate",
        "identify",
        "classify",
        "test",
        "run",
        "search",
        "document",
        "review",
        "inventory",
        "analys",
        "analyz",
        "implement",
        "fix",
        "write",
        "report",
        "check",
        "validate",
    ];
    let specific = contents
        .iter()
        .filter(|text| {
            text.split(|c: char| !c.is_alphanumeric() && c != '_')
                .any(|word| words.contains(&word.to_lowercase().as_str()))
        })
        .count();
    let mut base: i32 = if contents.is_empty() || contents.iter().all(|s| s.chars().count() <= 12) {
        0
    } else {
        let average = contents.iter().map(|s| s.chars().count()).sum::<usize>() as f64
            / contents.len() as f64;
        if todos.len() >= 3 && average >= 35.0 && specific as f64 / contents.len() as f64 >= 0.5 {
            if completed == todos.len() && calls >= 3 {
                10
            } else if completed as f64 >= (todos.len() as f64 * 0.6).max(2.0) {
                9
            } else {
                8
            }
        } else if todos.len() >= 2 && average >= 20.0 {
            if completed as f64 >= todos.len() as f64 * 0.5 && calls >= 2 {
                7
            } else if pending {
                5
            } else {
                6
            }
        } else if calls <= 1 || pending {
            2
        } else {
            4
        }
    };
    let debug = row["is_debug"].as_bool().unwrap_or(false);
    if debug {
        base -= 2;
    }
    base = base.clamp(0, 10);
    let reason = if contents.is_empty() {
        "No meaningful todo items."
    } else if base <= 1 {
        "Boilerplate or junk todos with little planning value."
    } else if base <= 4 {
        "Sparse or vague plan, with limited completion evidence."
    } else if base <= 7 {
        "Reasonable plan, but generic or incompletely updated."
    } else if completed == todos.len() && calls >= 3 {
        "Specific decomposed plan, repeatedly updated and fully completed."
    } else if completed >= 2 {
        "Specific multi-step plan with substantial completion evidence."
    } else {
        "Specific multi-step plan, but completion remains partial."
    };
    let reason = if debug {
        format!(
            "Debug session penalty applied; {}{}",
            reason[..1].to_lowercase(),
            &reason[1..]
        )
    } else {
        reason.into()
    };
    json!({"file":row["file"],"score":base,"reason":reason.chars().take(99).collect::<String>()})
}
pub fn run(input: &Path, output: &Path) -> Result<()> {
    let rows: Value = serde_json::from_slice(&fs::read(input)?)?;
    let scores: Vec<_> = rows
        .as_array()
        .context("expected an array of exported plans")?
        .iter()
        .map(score)
        .collect();
    let mut histogram = BTreeMap::new();
    for row in &scores {
        *histogram
            .entry(row["score"].as_i64().unwrap_or(0))
            .or_insert(0) += 1;
    }
    fs::write(
        output,
        format!("{}\n", serde_json::to_string_pretty(&scores)?),
    )?;
    println!("{} {}", scores.len(), serde_json::to_string(&histogram)?);
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scoring_preserves_completion_and_debug_rules() {
        assert_eq!(score(&json!({"todos":[]}))["score"], 0);
        let todos = vec![
            json!({"content":"Verify all provider routes and document their failure contracts","status":"completed"});
            3
        ];
        let mut row = json!({"file":"session","todos":todos,"todo_calls":3});
        assert_eq!(score(&row)["score"], 10);
        row["is_debug"] = true.into();
        let out = score(&row);
        assert_eq!(out["score"], 8);
        assert!(
            out["reason"]
                .as_str()
                .unwrap()
                .starts_with("Debug session penalty applied;")
        );
        assert!(out["reason"].as_str().unwrap().chars().count() <= 99);
    }
}
