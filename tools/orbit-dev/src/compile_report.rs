//! Advisory crate isolation report with opt-in strict target-state checks.
use anyhow::{Context, Result, ensure};
use clap::Parser;
use regex::Regex;
use serde_json::{Map, Value, json};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    process::Command,
};
const WATCHED: [&str; 4] = ["nexode-base", "nexode-app-core", "nexode-tui", "nexode"];
#[derive(Parser)]
struct Args {
    #[arg(long)]
    json: bool,
    #[arg(long)]
    strict_target_state: bool,
    #[arg(long, default_value_t = 12)]
    top: usize,
}
fn stats(root: &Path, package: &Value, names: &BTreeSet<String>) -> Result<Value> {
    let name = package["name"].as_str().context("missing name")?;
    let manifest = PathBuf::from(
        package["manifest_path"]
            .as_str()
            .context("missing manifest")?,
    );
    let targets = package["targets"].as_array().context("missing targets")?;
    let src = targets
        .iter()
        .find(|t| {
            t["kind"]
                .as_array()
                .is_some_and(|k| k.iter().any(|v| v == "lib"))
        })
        .and_then(|t| t["src_path"].as_str())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            if name == "nexode" {
                root.join("src")
            } else {
                manifest.parent().unwrap().join("src")
            }
        });
    let mut files = Vec::new();
    if src.is_file() {
        crate::safety_scan::rust_files(src.parent().unwrap(), &mut files)?;
    } else {
        crate::safety_scan::rust_files(&src, &mut files)?;
    }
    let patterns = [
        ("cfg_test_count", r"#\s*\[\s*cfg\s*\(\s*test\s*\)\s*\]"),
        (
            "test_attr_count",
            r"#\s*\[\s*(?:tokio::)?test(?:\s*\([^\]]*\))?\s*\]",
        ),
        (
            "async_trait_count",
            r"#\s*\[\s*(?:async_trait::)?async_trait\s*\]",
        ),
        ("derive_count", r"#\s*\[\s*derive\s*\("),
    ]
    .map(|(k, p)| (k, Regex::new(p).unwrap()));
    let glob = Regex::new(r"^pub\s+use\s+[^;\n]+::\s*\*\s*;$")?;
    let mut counts = Map::new();
    for (k, _) in &patterns {
        counts.insert(k.to_string(), json!(0));
    }
    let mut loc = 0;
    let mut reexports = Vec::new();
    for path in &files {
        let bytes = fs::read(path)?;
        let text = String::from_utf8_lossy(&bytes);
        loc += text.lines().count();
        for (k, p) in &patterns {
            let count = counts[*k].as_u64().unwrap() + p.find_iter(&text).count() as u64;
            counts.insert(k.to_string(), json!(count));
        }
        for (i, line) in text.lines().enumerate() {
            if glob.is_match(line.trim()) {
                reexports.push(format!(
                    "{}:{}: {}",
                    path.strip_prefix(root)?.display(),
                    i + 1,
                    line.trim()
                ));
            }
        }
    }
    let mut value = json!({"name":name,"manifest_path":manifest.strip_prefix(root)?.to_string_lossy(),"src_path":if src.exists(){Some(src.strip_prefix(root)?.to_string_lossy().into_owned())}else{None},"rust_files":files.len(),"loc":loc,"glob_reexports":reexports});
    let obj = value.as_object_mut().unwrap();
    obj.extend(counts);
    for kind in ["normal", "dev", "build"] {
        for space in ["workspace", "external"] {
            obj.insert(format!("{kind}_{space}_deps"), json!([]));
        }
    }
    for dep in package["dependencies"].as_array().context("missing deps")? {
        let name = dep["name"].as_str().context("missing dep name")?;
        let kind = match dep["kind"].as_str() {
            Some("dev") => "dev",
            Some("build") => "build",
            _ => "normal",
        };
        let space = if names.contains(name) {
            "workspace"
        } else {
            "external"
        };
        obj.get_mut(&format!("{kind}_{space}_deps"))
            .unwrap()
            .as_array_mut()
            .unwrap()
            .push(json!(name));
    }
    for kind in ["normal", "dev", "build"] {
        for space in ["workspace", "external"] {
            obj.get_mut(&format!("{kind}_{space}_deps"))
                .unwrap()
                .as_array_mut()
                .unwrap()
                .sort_by(|a, b| a.as_str().cmp(&b.as_str()));
        }
    }
    Ok(value)
}
fn violations(stats: &[Value]) -> Vec<String> {
    let find = |name| stats.iter().find(|s| s["name"] == name);
    let mut errors = Vec::new();
    for (parent, dep) in [
        ("nexode-tui", "nexode-app-core"),
        ("nexode-app-core", "nexode-base"),
    ] {
        if let Some(s) = find(parent)
            && s["normal_workspace_deps"]
                .as_array()
                .unwrap()
                .iter()
                .any(|d| d == dep)
        {
            errors.push(format!(
                "target-state: {parent} still directly depends on {dep}"
            ));
        }
    }
    if let Some(base) = find("nexode-base") {
        for dep in base["normal_workspace_deps"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
        {
            if [
                "nexode-azure-auth",
                "nexode-provider-gemini",
                "nexode-provider-openai",
                "nexode-provider-openrouter",
                "nexode-notify-email",
                "nexode-build-support",
            ]
            .contains(&dep)
            {
                errors.push(format!(
                    "target-state: nexode-base still depends on leaf/runtime crate {dep}"
                ));
            }
        }
        for dep in base["normal_external_deps"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
        {
            if dep.starts_with("aws-") {
                errors.push(format!(
                    "target-state: nexode-base still depends directly on AWS crate {dep}"
                ));
            }
        }
    }
    for s in stats {
        if ["nexode", "nexode-app-core", "nexode-tui"]
            .iter()
            .any(|n| s["name"] == *n)
        {
            for glob in s["glob_reexports"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(Value::as_str)
            {
                errors.push(format!(
                    "target-state: broad glob re-export remains in {glob}"
                ));
            }
        }
    }
    errors
}
pub fn run(root: &Path, args: impl Iterator<Item = String>) -> Result<()> {
    let args = Args::parse_from(std::iter::once("compile-isolation".into()).chain(args));
    let root = root.canonicalize()?;
    let output = Command::new("cargo")
        .args(["metadata", "--locked", "--no-deps", "--format-version", "1"])
        .current_dir(&root)
        .output()?;
    ensure!(
        output.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let data: Value = serde_json::from_slice(&output.stdout)?;
    let members = data["workspace_members"]
        .as_array()
        .context("missing members")?;
    let packages = data["packages"].as_array().context("missing packages")?;
    let packages = members
        .iter()
        .map(|id| {
            packages
                .iter()
                .find(|p| p["id"] == *id)
                .context("unknown workspace member")
        })
        .collect::<Result<Vec<_>>>()?;
    let names = packages
        .iter()
        .map(|p| p["name"].as_str().unwrap().to_owned())
        .collect();
    let stats = packages
        .iter()
        .map(|p| stats(&root, p, &names))
        .collect::<Result<Vec<_>>>()?;
    let violations = violations(&stats);
    let mut largest: Vec<_> = stats.iter().collect();
    largest.sort_by_key(|s| std::cmp::Reverse(s["loc"].as_u64().unwrap()));
    let largest: Vec<_> = largest.into_iter().take(args.top).collect();
    let watched: Map<String, Value> = WATCHED
        .iter()
        .filter_map(|name| {
            stats
                .iter()
                .find(|s| s["name"] == *name)
                .map(|s| (name.to_string(), s.clone()))
        })
        .collect();
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(
                &json!({"watched_crates":watched,"largest_crates":largest,"target_state_violations":violations})
            )?
        );
    } else {
        println!("compile isolation static report\nlargest workspace crates by Rust LOC:");
        for s in largest {
            println!(
                "  - {}: {} LOC, {} files, #[test] {}, cfg(test) {}, async_trait {}, derive {}",
                s["name"].as_str().unwrap(),
                s["loc"],
                s["rust_files"],
                s["test_attr_count"],
                s["cfg_test_count"],
                s["async_trait_count"],
                s["derive_count"]
            );
        }
        println!("watched crates:");
        for name in WATCHED {
            if let Some(s) = watched.get(name) {
                println!(
                    "  - {name}: {} LOC, normal workspace deps={}, normal external deps={}",
                    s["loc"],
                    s["normal_workspace_deps"].as_array().unwrap().len(),
                    s["normal_external_deps"].as_array().unwrap().len()
                );
                for kind in ["dev", "build"] {
                    println!(
                        "    {kind} workspace={}, external={}",
                        s[format!("{kind}_workspace_deps")]
                            .as_array()
                            .unwrap()
                            .len(),
                        s[format!("{kind}_external_deps")].as_array().unwrap().len()
                    );
                }
                let globs = s["glob_reexports"].as_array().unwrap();
                for g in globs.iter().take(8) {
                    println!("      {}", g.as_str().unwrap());
                }
                if globs.len() > 8 {
                    println!("      ... {} more", globs.len() - 8);
                }
            }
        }
        println!("target-state violations/advisories:");
        if violations.is_empty() {
            println!("  none");
        }
        for v in &violations {
            println!("  - {v}");
        }
    }
    ensure!(
        !args.strict_target_state || violations.is_empty(),
        "strict target state has {} violations",
        violations.len()
    );
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dependency_kinds_and_globs_remain_distinct() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path();
        fs::create_dir(root.join("src")).unwrap();
        fs::write(root.join("src/lib.rs"),"#[cfg(test)]\n#[tokio::test]\n#[async_trait]\n#[derive(Debug)]\npub use nexode_base::*;\n// pub use ignored::*;\n").unwrap();
        let p = json!({"name":"nexode-tui","manifest_path":root.join("Cargo.toml"),"targets":[{"kind":["lib"],"src_path":root.join("src/lib.rs")}],"dependencies":[{"name":"nexode-app-core","kind":null},{"name":"nexode-base","kind":"dev"},{"name":"cc","kind":"build"}]});
        let s = stats(
            root,
            &p,
            &BTreeSet::from(["nexode-app-core".into(), "nexode-base".into()]),
        )
        .unwrap();
        assert_eq!(s["cfg_test_count"], 1);
        assert_eq!(s["test_attr_count"], 1);
        assert_eq!(s["derive_count"], 1);
        assert_eq!(s["async_trait_count"], 1);
        assert_eq!(s["normal_workspace_deps"], json!(["nexode-app-core"]));
        assert_eq!(s["dev_workspace_deps"], json!(["nexode-base"]));
        assert_eq!(s["build_external_deps"], json!(["cc"]));
        assert_eq!(violations(&[s]).len(), 2);
    }
}
