//! Local changelog rendering; reads Git but never creates or publishes a release.
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::{collections::BTreeMap, fs, path::Path, process::Command};
fn git(root: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git").current_dir(root).args(args).output()?;
    ensure!(
        out.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
    Ok(String::from_utf8(out.stdout)?.trim().to_owned())
}
fn version(value: &str) -> Option<[u64; 3]> {
    let parts: Vec<_> = value.split('.').collect();
    if parts.len() != 3
        || parts
            .iter()
            .any(|p| p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    Some([
        parts[0].parse().ok()?,
        parts[1].parse().ok()?,
        parts[2].parse().ok()?,
    ])
}
fn previous_index(index: &Value, current: &str) -> Option<String> {
    let current = version(current)?;
    index["entries"]
        .as_array()?
        .iter()
        .filter_map(|e| {
            let s = e["version"].as_str()?;
            let v = version(s)?;
            (v < current).then_some((v, s))
        })
        .max_by_key(|(v, _)| *v)
        .map(|(_, s)| format!("v{s}"))
}
fn section(out: &mut String, heading: &str, items: &[String]) {
    if items.is_empty() {
        return;
    }
    out.push_str(&format!("### {heading}\n\n"));
    for item in items {
        out.push_str(&format!("- {item}\n"));
    }
    out.push('\n');
}
fn changelog(entry: &Value) -> Result<String> {
    let mut out = String::new();
    if let Some(title) = entry["title"].as_str().filter(|s| !s.is_empty()) {
        out.push_str(&format!("**{title}**\n\n"));
    }
    for (key, heading) in [
        ("highlights", "Highlights"),
        ("improvements", "Improvements"),
        ("fixes", "Fixes"),
    ] {
        if entry[key].is_null() {
            continue;
        }
        let items = entry[key]
            .as_array()
            .context("changelog section must be an array")?
            .iter()
            .map(|v| {
                v.as_str()
                    .map(str::to_owned)
                    .context("changelog item must be text")
            })
            .collect::<Result<Vec<_>>>()?;
        section(&mut out, heading, &items);
    }
    Ok(out.trim_end().into())
}
fn subjects(text: &str) -> Result<String> {
    let pattern = regex::Regex::new(r"(?i)^(feat|fix|perf|docs)(\([^)]*\))?!?:\s*(.+)$")?;
    let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for line in text.lines().map(str::trim).filter(|s| !s.is_empty()) {
        let (key, value) = if let Some(m) = pattern.captures(line) {
            (m[1].to_lowercase(), m[3].trim().to_owned())
        } else {
            ("other".into(), line.into())
        };
        groups.entry(key).or_default().push(value);
    }
    let mut out = String::new();
    for (key, heading) in [
        ("feat", "Features"),
        ("fix", "Fixes"),
        ("perf", "Performance"),
        ("docs", "Documentation"),
        ("other", "Other changes"),
    ] {
        if let Some(items) = groups.get(key) {
            section(&mut out, heading, items);
        }
    }
    Ok(out.trim_end().into())
}
pub fn run(args: impl Iterator<Item = String>) -> Result<()> {
    let args: Vec<_> = args.collect();
    ensure!(
        (1..=2).contains(&args.len()),
        "usage: generate-release-notes <tag> [previous-tag]"
    );
    let tag = &args[0];
    ensure!(
        !tag.starts_with('-') && !tag.contains(['/', '\\']),
        "invalid release tag"
    );
    let root = git(Path::new("."), &["rev-parse", "--show-toplevel"])?;
    let root = Path::new(&root);
    let num = tag.strip_prefix('v').unwrap_or(tag);
    let mut prev = args.get(1).cloned().unwrap_or_default();
    if prev.is_empty() {
        let index = root.join("changelog/index.json");
        if index.exists() {
            prev = previous_index(&serde_json::from_slice(&fs::read(index)?)?, num)
                .unwrap_or_default();
        }
        if !prev.is_empty()
            && git(
                root,
                &["rev-parse", "-q", "--verify", &format!("refs/tags/{prev}")],
            )
            .is_err()
        {
            prev.clear();
        }
        if prev.is_empty()
            && git(
                root,
                &["rev-parse", "-q", "--verify", &format!("refs/tags/{tag}")],
            )
            .is_ok()
        {
            prev = git(
                root,
                &["describe", "--tags", "--abbrev=0", &format!("{tag}^")],
            )
            .unwrap_or_default();
        }
        if prev.is_empty() {
            let tags = git(root, &["tag", "-l", "v*", "--sort=-v:refname"])?;
            let mut lines = tags.lines();
            while let Some(line) = lines.next() {
                if line == tag {
                    prev = lines.next().unwrap_or("").into();
                    break;
                }
            }
        }
    }
    ensure!(!prev.starts_with('-'), "invalid previous tag");
    let file = root.join(format!("changelog/v{num}.json"));
    let body = if file.is_file() {
        changelog(&serde_json::from_slice(&fs::read(file)?)?)?
    } else {
        let end = if git(
            root,
            &["rev-parse", "-q", "--verify", &format!("{tag}^{{commit}}")],
        )
        .is_ok()
        {
            tag.as_str()
        } else {
            "HEAD"
        };
        let range = if prev.is_empty() {
            end.into()
        } else {
            format!("{prev}..{end}")
        };
        subjects(&git(
            root,
            &["log", "--no-merges", "--pretty=format:%s", &range],
        )?)?
    };
    let repo = std::env::var("GITHUB_REPOSITORY")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| {
            let url = git(root, &["remote", "get-url", "origin"]).ok()?;
            let re = regex::Regex::new(r"github\.com[:/]+([^/]+/[^/ ]+)").ok()?;
            Some(re.captures(&url)?[1].trim_end_matches(".git").to_owned())
        })
        .unwrap_or_else(|| "nexode-agent/unconfigured".into());
    if !body.is_empty() {
        println!("{body}\n");
    }
    if prev.is_empty() {
        println!("**Full changelog**: https://github.com/{repo}/commits/{tag}");
    } else {
        println!("**Full changelog**: https://github.com/{repo}/compare/{prev}...{tag}");
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn stable_versions_use_numeric_order_and_skip_future_and_prereleases() {
        let index = json!({"entries":[{"version":"0.9.0"},{"version":"0.10.0"},{"version":"0.11.0-preview.1"},{"version":"1.0.0"}]});
        assert_eq!(previous_index(&index, "0.11.0").as_deref(), Some("v0.10.0"));
        assert!(previous_index(&index, "0.11.0-rc.1").is_none());
    }
    #[test]
    fn curated_notes_and_conventional_groups_keep_content_and_order() {
        assert_eq!(
            changelog(&json!({"title":"Title","highlights":["One"],"fixes":["Two"]})).unwrap(),
            "**Title**\n\n### Highlights\n\n- One\n\n### Fixes\n\n- Two"
        );
        let out =
            subjects("fix(scope)!: bug\nFeat: new\nperf: faster\ndocs: manual\nmisc").unwrap();
        assert!(out.starts_with("### Features\n\n- new"));
        assert!(out.contains("### Fixes\n\n- bug"));
        assert!(out.ends_with("- misc"));
    }
}
