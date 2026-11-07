use anyhow::{Context, Result, ensure};
use regex::Regex;
use std::{collections::BTreeSet, fs, path::Path, process::Command};
fn unresolved(source: &Path, text: &str) -> Result<Vec<String>> {
    let declaration =
        Regex::new(r"^\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+([A-Za-z_][A-Za-z0-9_]*)\s*;")?;
    let attribute = Regex::new(r#"#\s*\[\s*path\s*=\s*"([^"]+)"\s*\]"#)?;
    let parent = source.parent().context("module parent")?;
    let mut pending = None;
    let mut missing = Vec::new();
    for line in text.lines() {
        if let Some(found) = attribute.captures(line) {
            pending = Some(found[1].to_owned());
            continue;
        }
        if let Some(found) = declaration.captures(line) {
            let name = &found[1];
            let exists = if let Some(path) = pending.take() {
                parent.join(path).exists()
            } else {
                let mut candidates = vec![
                    parent.join(format!("{name}.rs")),
                    parent.join(name).join("mod.rs"),
                ];
                if !["mod.rs", "lib.rs", "main.rs"]
                    .iter()
                    .any(|n| source.file_name().is_some_and(|f| f == *n))
                    && let Some(stem) = source.file_stem()
                {
                    candidates.push(parent.join(stem).join(format!("{name}.rs")));
                    candidates.push(parent.join(stem).join(name).join("mod.rs"));
                }
                candidates.iter().any(|p| p.exists())
            };
            if !exists {
                missing.push(name.to_owned());
            }
        } else {
            let line = line.trim();
            if !line.is_empty() && !["#", "//", "/*", "*"].iter().any(|p| line.starts_with(p)) {
                pending = None;
            }
        }
    }
    Ok(missing)
}
pub fn modules(root: &Path) -> Result<()> {
    let output = Command::new("git")
        .args(["ls-files", "-z", "*.rs"])
        .current_dir(root)
        .output()?;
    ensure!(output.status.success(), "git ls-files failed");
    let mut errors = Vec::new();
    for path in std::str::from_utf8(&output.stdout)?
        .split('\0')
        .filter(|p| !p.is_empty())
    {
        let source = root.join(path);
        let Ok(text) = fs::read_to_string(&source) else {
            continue;
        };
        for name in unresolved(&source, &text)? {
            errors.push(format!("{path}: mod {name}; has no file"));
        }
    }
    for error in &errors {
        eprintln!("{error}");
    }
    ensure!(
        errors.is_empty(),
        "{} unresolved module declarations",
        errors.len()
    );
    println!("Module declarations OK");
    Ok(())
}
fn type_crate(name: &str) -> bool {
    name.starts_with("nexode-") && name.ends_with("-types")
}
fn boundary_errors(metadata: &serde_json::Value) -> Result<Vec<String>> {
    let packages = metadata["packages"]
        .as_array()
        .context("missing metadata packages")?;
    let workspace: BTreeSet<_> = metadata["workspace_members"]
        .as_array()
        .context("missing workspace members")?
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    let names: BTreeSet<_> = packages
        .iter()
        .filter(|p| p["id"].as_str().is_some_and(|id| workspace.contains(id)))
        .filter_map(|p| p["name"].as_str())
        .collect();
    let forbidden = [
        "nexode",
        "nexode-agent-runtime",
        "nexode-azure-auth",
        "nexode-core",
        "nexode-embedding",
        "nexode-mobile-core",
        "nexode-mobile-sim",
        "nexode-notify-email",
        "nexode-pdf",
        "nexode-plan",
        "nexode-provider-core",
        "nexode-provider-gemini",
        "nexode-provider-metadata",
        "nexode-provider-openrouter",
        "nexode-protocol",
        "nexode-terminal-launch",
        "nexode-tui-core",
        "nexode-tui-markdown",
        "nexode-tui-mermaid",
        "nexode-tui-render",
        "nexode-tui-workspace",
    ];
    let mut errors = Vec::new();
    for pkg in packages {
        if !pkg["id"].as_str().is_some_and(|id| workspace.contains(id)) {
            continue;
        }
        let name = pkg["name"].as_str().context("missing package name")?;
        if !type_crate(name) {
            continue;
        }
        for dep in pkg["dependencies"]
            .as_array()
            .context("missing dependencies")?
        {
            let dependency = dep["name"].as_str().context("missing dependency name")?;
            if !names.contains(dependency)
                || dependency == "nexode-message-types"
                || type_crate(dependency)
            {
                continue;
            }
            if forbidden.contains(&dependency) {
                errors.push(format!(
                    "{name} must not depend on runtime/internal crate {dependency}"
                ));
            } else {
                eprintln!(
                    "warning: {name} depends on internal non-type crate {dependency}; review boundary policy"
                );
            }
        }
    }
    Ok(errors)
}
pub fn boundaries(root: &Path) -> Result<()> {
    let output = Command::new("cargo")
        .args(["metadata", "--locked", "--no-deps", "--format-version", "1"])
        .current_dir(root)
        .output()?;
    ensure!(
        output.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let errors = boundary_errors(&serde_json::from_slice(&output.stdout)?)?;
    for error in &errors {
        eprintln!("{error}");
    }
    ensure!(
        errors.is_empty(),
        "{} dependency boundary errors",
        errors.len()
    );
    println!("Dependency boundaries passed");
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resolves_nested_and_aliased_modules_and_does_not_leak_attributes() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        fs::create_dir(root.join("owner")).unwrap();
        fs::write(root.join("owner/nested.rs"), "").unwrap();
        fs::write(root.join("alias.rs"), "").unwrap();
        let text = "mod nested;\n#[path = \"alias.rs\"]\nmod found;\n#[path = \"alias.rs\"]\nfn unrelated() {}\nmod missing;";
        assert_eq!(
            unresolved(&root.join("owner.rs"), text).unwrap(),
            vec!["missing"]
        );
        assert_eq!(
            unresolved(&root.join("lib.rs"), "mod nested;").unwrap(),
            vec!["nested"]
        );
    }
    #[test]
    fn type_contracts_reject_runtime_dependencies() {
        let metadata = serde_json::json!({"workspace_members":["types","core","message"],"packages":[
            {"id":"types","name":"nexode-demo-types","dependencies":[{"name":"nexode-core"},{"name":"nexode-message-types"},{"name":"serde"}]},
            {"id":"core","name":"nexode-core","dependencies":[]},
            {"id":"message","name":"nexode-message-types","dependencies":[]}
        ]});
        let errors = boundary_errors(&metadata).unwrap();
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("nexode-core"));
    }
}
