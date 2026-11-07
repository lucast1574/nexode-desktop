//! Safety ratchets. Existing baseline files are never rewritten by a check.
use crate::safety_scan;
use anyhow::{Context, Result, ensure};
use regex::Regex;
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, path::Path};
type Counts = BTreeMap<String, u64>;
#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Panic,
    Swallowed,
    Wildcard,
}
impl Kind {
    fn filename(self) -> &'static str {
        match self {
            Self::Panic => "panic_budget.json",
            Self::Swallowed => "swallowed_error_budget.json",
            Self::Wildcard => "wildcard_reexport_budget.json",
        }
    }
    fn patterns(self) -> Vec<(&'static str, Regex)> {
        let patterns = match self {
            Self::Panic => vec![(
                "panic",
                r"\.unwrap\(|\.expect\(|\b(?:panic!|todo!|unimplemented!)",
            )],
            Self::Swallowed => vec![
                ("let_underscore", r"\blet\s+_\s*="),
                ("dot_ok", r"\.ok\(\)"),
                ("unwrap_or_default", r"\.unwrap_or_default\(\)"),
            ],
            Self::Wildcard => vec![(
                "wildcard",
                r"^\s*pub\s+use\s+(?:::)?nexode_[a-z0-9_]+(?:::[A-Za-z0-9_]+)*::\*\s*;",
            )],
        };
        patterns
            .into_iter()
            .map(|(n, p)| (n, Regex::new(p).unwrap()))
            .collect()
    }
}
pub fn collect(root: &Path, kind: Kind) -> Result<Value> {
    let patterns = kind.patterns();
    let files = if kind == Kind::Wildcard {
        let mut f = Vec::new();
        for d in ["src", "crates"] {
            safety_scan::rust_files(&root.join(d), &mut f)?;
        }
        f
    } else {
        safety_scan::production_files(root)?
    };
    let mut totals: Counts = patterns.iter().map(|(n, _)| (n.to_string(), 0)).collect();
    let mut counts = BTreeMap::new();
    let mut total = 0;
    for path in files {
        let source = fs::read_to_string(&path)?;
        let text = if kind == Kind::Wildcard {
            source
        } else {
            safety_scan::production(&source)
        };
        let mut per_file = Counts::new();
        for (name, pattern) in &patterns {
            let count = text.lines().filter(|l| pattern.is_match(l)).count() as u64;
            per_file.insert(name.to_string(), count);
            *totals.get_mut(*name).unwrap() += count;
        }
        let count: u64 = per_file.values().sum();
        total += count;
        if count > 0 {
            counts.insert(
                path.strip_prefix(root)?
                    .to_string_lossy()
                    .replace('\\', "/"),
                if kind == Kind::Swallowed {
                    json!(per_file)
                } else {
                    json!(count)
                },
            );
        }
    }
    Ok(match kind {
        Kind::Wildcard => json!({"total":total,"files":counts}),
        Kind::Panic => json!({"version":1,"total":total,"tracked_files":counts}),
        Kind::Swallowed => {
            json!({"version":1,"total":total,"tracked_files":counts,"totals_by_pattern":totals})
        }
    })
}
fn count(value: &Value, nested: bool) -> Result<u64> {
    if nested {
        value
            .as_object()
            .context("expected pattern counts")?
            .values()
            .map(|v| v.as_u64().context("invalid pattern count"))
            .sum()
    } else {
        value.as_u64().context("invalid nonnegative count")
    }
}
fn regressions(old: &Value, current: &Value, kind: Kind) -> Result<Vec<String>> {
    let key = if kind == Kind::Wildcard {
        "files"
    } else {
        "tracked_files"
    };
    let tracked = old[key].as_object().context("invalid baseline files")?;
    let nested = kind == Kind::Swallowed;
    for value in tracked.values() {
        let n = count(value, nested)?;
        ensure!(nested || n > 0, "invalid zero baseline file count");
    }
    let mut errors = Vec::new();
    let old_total = old["total"].as_u64().context("invalid baseline total")?;
    let total = current["total"].as_u64().context("invalid current total")?;
    if total > old_total {
        errors.push(format!("total grew: {old_total} -> {total}"));
    }
    if nested {
        let old_patterns = old["totals_by_pattern"]
            .as_object()
            .context("invalid pattern totals")?;
        for (pattern, value) in current["totals_by_pattern"].as_object().unwrap() {
            let before = match old_patterns.get(pattern) {
                Some(v) => count(v, false)?,
                None => 0,
            };
            let now = count(value, false)?;
            if now > before {
                errors.push(format!("{pattern} grew: {before} -> {now}"));
            }
        }
    }
    for (path, value) in current[key].as_object().unwrap() {
        let now = count(value, nested)?;
        match tracked.get(path) {
            Some(before) => {
                let before = count(before, nested)?;
                if now > before {
                    errors.push(format!("{path} grew: {before} -> {now}"));
                }
            }
            None => errors.push(format!("new usage in {path}: {now}")),
        }
    }
    Ok(errors)
}
pub fn run(root: &Path, kind: Kind, update: bool, dump: bool) -> Result<()> {
    let path = root.join("scripts").join(kind.filename());
    let current = collect(root, kind)?;
    if dump {
        println!("{}", serde_json::to_string_pretty(&current)?);
        return Ok(());
    }
    if update {
        fs::write(
            path,
            format!("{}\n", serde_json::to_string_pretty(&current)?),
        )?;
        println!("Updated safety baseline");
        return Ok(());
    }
    let old = match fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && kind != Kind::Wildcard => {
            json!({"version":1,"total":0,"tracked_files":{},"totals_by_pattern":{}})
        }
        Err(e) => return Err(e.into()),
    };
    let errors = regressions(&old, &current, kind)?;
    for e in &errors {
        eprintln!("{e}");
    }
    ensure!(
        errors.is_empty(),
        "safety budget exceeded: {} regressions; baseline unchanged",
        errors.len()
    );
    println!(
        "Safety budget passed: total={} baseline={}",
        current["total"], old["total"]
    );
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn regressions_never_rewrite_baseline_and_update_is_explicit() {
        for kind in [Kind::Panic, Kind::Swallowed, Kind::Wildcard] {
            let tmp = tempfile::tempdir().unwrap();
            let root = tmp.path();
            fs::create_dir(root.join("src")).unwrap();
            fs::create_dir(root.join("scripts")).unwrap();
            let source = match kind {
                Kind::Panic => "fn f() { a.unwrap(); }",
                Kind::Swallowed => "fn f() { a.ok(); }",
                Kind::Wildcard => "pub use nexode_base::*;",
            };
            fs::write(root.join("src/old.rs"), source).unwrap();
            run(root, kind, true, false).unwrap();
            let p = root.join("scripts").join(kind.filename());
            let before = fs::read(&p).unwrap();
            fs::write(root.join("src/old.rs"), format!("{source}\n{source}")).unwrap();
            assert!(run(root, kind, false, false).is_err());
            assert_eq!(before, fs::read(&p).unwrap());
            fs::remove_file(root.join("src/old.rs")).unwrap();
            fs::write(root.join("src/new.rs"), source).unwrap();
            assert!(run(root, kind, false, false).is_err());
            assert_eq!(before, fs::read(&p).unwrap());
        }
    }
    #[test]
    fn swallowed_pattern_totals_cannot_grow_when_file_total_stays_equal() {
        let old =
            json!({"total":1,"tracked_files":{"a":{"dot_ok":1}},"totals_by_pattern":{"dot_ok":1}});
        let new = json!({"total":1,"tracked_files":{"a":{"let_underscore":1}},"totals_by_pattern":{"let_underscore":1}});
        assert_eq!(regressions(&old, &new, Kind::Swallowed).unwrap().len(), 1);
    }
}
