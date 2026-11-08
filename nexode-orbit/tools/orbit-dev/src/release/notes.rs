use super::{Github, array_pages};
use anyhow::{Context, Result, ensure};
use regex::Regex;
use std::collections::BTreeSet;
pub fn number(tag: &str) -> Option<u64> {
    let suffix = tag.strip_prefix("v1.0.0-preview.")?;
    if suffix.starts_with('0') || !suffix.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    suffix.parse().ok()
}
fn literal(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if "\\`*_{}[]()<>#".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}
pub fn changes(repo: &str, sha: &str, tag: &str, api: &mut impl Github) -> Result<String> {
    let releases = array_pages(api.api(&format!("repos/{repo}/releases?per_page=100"), true)?)?;
    let n = number(tag).context("invalid preview number")?;
    let releases: Vec<_> = releases
        .iter()
        .filter(|r| r["draft"] != true && r["tag_name"] != tag)
        .collect();
    let baseline = releases
        .iter()
        .filter(|r| number(r["tag_name"].as_str().unwrap_or("")).is_some_and(|p| p < n))
        .max_by_key(|r| number(r["tag_name"].as_str().unwrap_or("")))
        .or_else(|| {
            releases
                .iter()
                .filter(|r| r["prerelease"] != true)
                .max_by_key(|r| r["published_at"].as_str().unwrap_or(""))
        })
        .context("No published release baseline")?;
    let base = baseline["tag_name"]
        .as_str()
        .context("missing baseline tag")?;
    let encoded: String = base
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect();
    let comparison = format!("{encoded}...{sha}");
    let value = api.api(
        &format!("repos/{repo}/compare/{comparison}?per_page=100"),
        true,
    )?;
    let pages = value
        .as_array()
        .filter(|p| !p.is_empty())
        .context("missing comparison pages")?;
    let total = pages[0]["total_commits"]
        .as_u64()
        .context("missing total commits")? as usize;
    let mut seen = BTreeSet::new();
    let mut commits = vec![];
    for page in pages {
        for commit in page["commits"].as_array().context("missing commits")? {
            let hash = commit["sha"].as_str().context("missing commit SHA")?;
            ensure!(
                hash.len() == 40
                    && hash
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
                "Invalid commit SHA in comparison"
            );
            if seen.insert(hash.to_string()) {
                commits.push((
                    hash.to_string(),
                    commit["commit"]["message"]
                        .as_str()
                        .context("missing commit message")?
                        .lines()
                        .next()
                        .unwrap_or("")
                        .to_string(),
                ));
            }
        }
    }
    ensure!(commits.len() == total, "Incomplete preview comparison");
    let feat = Regex::new(r"(?i)^(?:feat(?:\([^)]*\))?!?:|add\b|implement\b)")?;
    let fix = Regex::new(r"(?i)^(?:fix(?:\([^)]*\))?!?:|fix\b|repair\b|resolve\b)")?;
    let features = commits.iter().filter(|(_, s)| feat.is_match(s)).count();
    let fixes = commits.iter().filter(|(_, s)| fix.is_match(s)).count();
    let url = format!("https://github.com/{repo}/compare/{comparison}");
    let mut out = format!(
        "## What changed\n\n{total} commits since **{}**: {features} feature/addition commits, {fixes} fixes, and {} other changes (classified from commit titles).\n\n[Full comparison]({url})\n\n",
        literal(base),
        total - features - fixes
    );
    let highlights: Vec<_> = [feat, fix]
        .iter()
        .flat_map(|pattern| {
            commits
                .iter()
                .rev()
                .filter(|(_, s)| pattern.is_match(s))
                .take(2)
                .map(|(_, s)| literal(s))
        })
        .collect();
    if !highlights.is_empty() {
        out.push_str("**Highlights from commit titles**\n\n");
        for title in highlights {
            out.push_str(&format!("- {title}\n"));
        }
        out.push('\n');
    }
    out.push_str("## Commits\n\n");
    for (index, (hash, subject)) in commits.iter().rev().enumerate() {
        let line = format!(
            "- [{}](https://github.com/{repo}/commit/{hash}) {}\n",
            &hash[..8],
            literal(subject)
        );
        if out.chars().count() + line.chars().count() > 55000 {
            out.push_str(&format!(
                "\n{} more commits in the [full comparison]({url}).\n",
                total - index
            ));
            break;
        }
        out.push_str(&line);
    }
    if commits.is_empty() {
        out.push_str("No source changes since the previous published release.\n");
    }
    out.push('\n');
    Ok(out)
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    struct Fake {
        replies: std::collections::VecDeque<Value>,
        calls: Vec<String>,
    }
    impl Github for Fake {
        fn command(&mut self, _: &[String]) -> Result<String> {
            unreachable!()
        }
        fn api(&mut self, path: &str, _: bool) -> Result<Value> {
            self.calls.push(path.into());
            Ok(self.replies.pop_front().unwrap())
        }
    }
    fn notes(releases: Value, pages: Value) -> Result<(String, Vec<String>)> {
        let mut api = Fake {
            replies: [releases, pages].into(),
            calls: vec![],
        };
        let text = changes("o/r", &"a".repeat(40), "v1.0.0-preview.12", &mut api)?;
        Ok((text, api.calls))
    }
    #[test]
    fn baseline_pagination_escape_and_counts() {
        let releases = json!([[{"tag_name":"v0.99.1","published_at":"2026-10-22"},{"tag_name":"v0.98.0","published_at":"2026-10-01"}]]);
        let commit = json!({"sha":"b".repeat(40),"commit":{"message":"Fix <script> [link](bad)\nNot a subject"}});
        let (out,calls)=notes(releases.clone(),json!([{"total_commits":2,"commits":[commit]},{"commits":[commit,{"sha":"c".repeat(40),"commit":{"message":"feat: Add tests"}}]}])).unwrap();
        assert!(calls[1].contains("v0.99.1..."));
        assert!(out.contains("2 commits since **v0.99.1**: 1 feature/addition commits, 1 fixes"));
        assert_eq!(out.matches("bbbbbbbb]").count(), 1);
        assert!(!out.contains("<script>"));
        assert!(!out.contains("Not a subject"));
        assert!(notes(json!([[]]), json!([])).is_err());
        assert!(notes(releases, json!([{"total_commits":1,"commits":[]}])).is_err());
        let releases = json!([[{"tag_name":"v1.0.0-preview.9","prerelease":true},{"tag_name":"v1.0.0-preview.10","prerelease":true},{"tag_name":"v1.0.0-preview.11","draft":true},{"tag_name":"v1.0.0-preview.13"}]]);
        let (out, calls) = notes(releases, json!([{"total_commits":0,"commits":[]}])).unwrap();
        assert!(calls[1].contains("preview.10..."));
        assert!(out.contains("No source changes"));
    }
    #[test]
    fn large_list_bounded_newest_first_and_invalid_sha() {
        let commits:Vec<_>=(0..200).map(|i|json!({"sha":format!("{i:040x}"),"commit":{"message":format!("Fix {}","x".repeat(500))}})).collect();
        let (out, _) = notes(
            json!([[{"tag_name":"v0.99.1"}]]),
            json!([{"total_commits":200,"commits":commits}]),
        )
        .unwrap();
        assert!(out.len() < 56000);
        assert!(out.contains("more commits in the [full comparison]"));
        assert!(out.find("000000c7)") < out.find("000000c6)"));
        assert!(
            notes(
                json!([[{"tag_name":"v0.99.1"}]]),
                json!([{"total_commits":1,"commits":[{"sha":"evil","commit":{"message":"x"}}]}])
            )
            .is_err()
        );
    }
}
