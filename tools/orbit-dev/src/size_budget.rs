//! Ratcheting production/test file-size budgets, preserving the JSON baselines.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, path::Path};
fn is_test(path: &Path) -> bool {
    path.components().any(|part| {
        let s = part.as_os_str().to_string_lossy();
        s == "tests" || s.ends_with("_tests") || s.ends_with("_test") || s.starts_with("tests_")
    }) || path.file_name().is_some_and(|n| {
        let n = n.to_string_lossy();
        n == "tests.rs"
            || n.ends_with("_tests.rs")
            || n.ends_with("_test.rs")
            || n.starts_with("tests_")
    })
}
fn collect(
    root: &Path,
    dir: &Path,
    tests: bool,
    threshold: u64,
    files: &mut BTreeMap<String, u64>,
) -> Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let path = entry.path();
        if kind.is_symlink() {
            continue;
        }
        if kind.is_dir() {
            collect(root, &path, tests, threshold, files)?;
        } else if path.extension().is_some_and(|s| s == "rs") {
            let rel = path.strip_prefix(root)?;
            if is_test(rel) != tests {
                continue;
            }
            let count = fs::read_to_string(&path)?.lines().count() as u64;
            if count > threshold {
                files.insert(rel.to_string_lossy().replace('\\', "/"), count);
            }
        }
    }
    Ok(())
}
fn regressions(current: &BTreeMap<String, u64>, tracked: &BTreeMap<String, u64>) -> Vec<String> {
    current
        .iter()
        .filter_map(|(path, count)| match tracked.get(path) {
            Some(old) if count <= old => None,
            Some(old) => Some(format!(
                "oversized file grew: {path} ({old} -> {count} LOC)"
            )),
            None => Some(format!("new oversized file: {path} ({count} LOC)")),
        })
        .collect()
}
pub fn run(root: &Path, tests: bool, update: bool) -> Result<()> {
    let path = root.join(if tests {
        "scripts/test_size_budget.json"
    } else {
        "scripts/code_size_budget.json"
    });
    let baseline: Value = match fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes)?,
        Err(e) if tests && e.kind() == std::io::ErrorKind::NotFound => {
            json!({"version":1,"threshold_loc":1200,"tracked_files":{}})
        }
        Err(e) => return Err(e.into()),
    };
    let threshold = baseline["threshold_loc"]
        .as_u64()
        .filter(|n| *n > 0)
        .context("invalid size threshold")?;
    let tracked = baseline["tracked_files"]
        .as_object()
        .context("invalid size baseline")?
        .iter()
        .map(|(p, n)| {
            Ok((
                p.clone(),
                n.as_u64()
                    .filter(|n| *n > 0)
                    .context("invalid baseline line count")?,
            ))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    let mut current = BTreeMap::new();
    for name in if tests {
        vec!["src", "crates", "tests"]
    } else {
        vec!["src", "crates"]
    } {
        collect(root, &root.join(name), tests, threshold, &mut current)?;
    }
    if update {
        fs::write(
            path,
            format!(
                "{}\n",
                serde_json::to_string_pretty(
                    &json!({"version":1,"threshold_loc":threshold,"tracked_files":current})
                )?
            ),
        )?;
        println!(
            "Updated {} size baseline: {} -> {} files",
            if tests { "test" } else { "production" },
            tracked.len(),
            current.len()
        );
        return Ok(());
    }
    let errors = regressions(&current, &tracked);
    for error in &errors {
        eprintln!("{error}");
    }
    ensure!(
        errors.is_empty(),
        "size budget exceeded: {} regressions; update only after intentional cleanup",
        errors.len()
    );
    println!(
        "Size budget passed: {} oversized files, threshold {threshold}",
        current.len()
    );
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn classifies_test_files_and_ratchets_without_accepting_growth() {
        for path in [
            "tests/a.rs",
            "crates/x/src/tests.rs",
            "crates/x/src/network_tests/a.rs",
            "src/tests_extra/a.rs",
        ] {
            assert!(is_test(Path::new(path)));
        }
        assert!(!is_test(Path::new("crates/x/src/lib.rs")));
        let tracked = BTreeMap::from([("src/a.rs".into(), 5)]);
        assert!(regressions(&BTreeMap::from([("src/a.rs".into(), 4)]), &tracked).is_empty());
        assert_eq!(
            regressions(
                &BTreeMap::from([("src/a.rs".into(), 6), ("src/new.rs".into(), 3)]),
                &tracked
            )
            .len(),
            2
        );
    }
    #[test]
    fn update_is_explicit_and_tests_have_separate_baseline() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        fs::create_dir_all(root.join("scripts")).unwrap();
        fs::create_dir(root.join("src")).unwrap();
        fs::write(
            root.join("scripts/code_size_budget.json"),
            json!({"threshold_loc":2,"tracked_files":{}}).to_string(),
        )
        .unwrap();
        fs::write(root.join("src/lib.rs"), "one\ntwo\nthree\n").unwrap();
        let before = fs::read(root.join("scripts/code_size_budget.json")).unwrap();
        assert!(run(root, false, false).is_err());
        assert_eq!(
            fs::read(root.join("scripts/code_size_budget.json")).unwrap(),
            before
        );
        run(root, false, true).unwrap();
        run(root, false, false).unwrap();
        run(root, true, false).unwrap();
        fs::write(root.join("src/lib.rs"), "one\ntwo\nthree\nfour\n").unwrap();
        assert!(run(root, false, false).is_err());
    }
}
