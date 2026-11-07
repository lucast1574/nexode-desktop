//! Release announcement formatting and tag-specific duplicate prevention.
use anyhow::{Context, Result, ensure};
use clap::Parser;
use regex::Regex;
use serde_json::{Value, json};
use std::{path::PathBuf, time::Duration};
const LIMIT: usize = 2000;
fn marker(tag: &str) -> String {
    format!("<!-- nexode-discord-announced:{tag} -->")
}
fn preview(tag: &str) -> bool {
    Regex::new(r"^v1\.0\.0-preview\.[1-9][0-9]*$")
        .unwrap()
        .is_match(tag)
}
fn notes(body: &str) -> String {
    let body=Regex::new(r"(?s)\n?<!-- nexode-platform-availability:start -->.*?<!-- nexode-platform-availability:end -->\n?").unwrap().replace_all(body,"\n");
    Regex::new(r"\n{3,}")
        .unwrap()
        .replace_all(&body, "\n\n")
        .trim()
        .into()
}
fn commits(notes: &str) -> Vec<String> {
    let line = Regex::new(r"^- \[[0-9a-f]{4,40}\]\([^)]*\) (.+)$").unwrap();
    let url = Regex::new(r"https?://[^\s<>()]+").unwrap();
    let mut in_section = false;
    let mut lines = Vec::new();
    for raw in notes.lines() {
        if raw.trim_end() == "## Commits" {
            in_section = true;
            continue;
        }
        if in_section && raw.starts_with("## ") {
            break;
        }
        if !in_section {
            continue;
        }
        if let Some(c) = line.captures(raw.trim()) {
            let subject = &c[1];
            let mut literal = String::new();
            let mut pos = 0;
            for m in url.find_iter(subject) {
                literal.push_str(&subject[pos..m.start()]);
                if m.start() > 0 && subject.as_bytes()[m.start() - 1] == b'<' {
                    literal.push_str(m.as_str());
                } else {
                    literal.push('<');
                    literal.push_str(m.as_str());
                    literal.push('>');
                }
                pos = m.end();
            }
            literal.push_str(&subject[pos..]);
            lines.push(format!("- {literal}"));
        }
    }
    lines
}
fn message(tag: &str, name: &str, body: &str, url: &str) -> String {
    let fallback = "https://github.com/lucast1574/nexode-orbit/releases";
    let url = if url.chars().count() <= 256 {
        url
    } else {
        fallback
    };
    if preview(tag) {
        let header = format!(
            "**nexode {}** (preview) {url}\n",
            tag.chars().take(120).collect::<String>()
        );
        let commits = commits(&notes(body));
        if commits.is_empty() {
            return header.trim_end().into();
        }
        let mut used = header.chars().count();
        let mut lines = Vec::new();
        for (index, line) in commits.iter().enumerate() {
            let more = format!("… {} more", commits.len() - index);
            let reserve = if index == commits.len() - 1 {
                0
            } else {
                more.chars().count() + 1
            };
            if used + line.chars().count() + 1 + reserve > LIMIT {
                lines.push(more);
                break;
            }
            lines.push(line.clone());
            used += line.chars().count() + 1;
        }
        return header + &lines.join("\n");
    }
    let title = if !name.is_empty() && name != tag {
        format!("## {tag} — {name}")
    } else {
        format!("## {tag}")
    };
    let notes = notes(body);
    let mut msg = if notes.is_empty() {
        title
    } else {
        format!("{title}\n{notes}")
    };
    if msg.chars().count() > LIMIT {
        let suffix = format!("\n… (full notes: <{url}>)");
        msg = msg
            .chars()
            .take(LIMIT.saturating_sub(suffix.chars().count()))
            .collect::<String>()
            + &suffix;
    }
    msg
}
fn webhook_url(raw: &str) -> Result<url::Url> {
    let mut url = url::Url::parse(raw).context("invalid webhook URL")?;
    if !url.query_pairs().any(|(key, _)| key == "wait") {
        url.query_pairs_mut().append_pair("wait", "true");
    }
    Ok(url)
}
trait Api {
    fn fetch(&mut self, repo: &str, tag: &str) -> Result<Value>;
    fn post(&mut self, content: &str) -> Result<Value>;
    fn mark(&mut self, repo: &str, id: &Value, body: &str) -> Result<()>;
}
struct Http {
    client: reqwest::blocking::Client,
    token: String,
    webhook: String,
}
impl Http {
    fn response(
        response: std::result::Result<reqwest::blocking::Response, reqwest::Error>,
    ) -> Result<Value> {
        let response = response.map_err(|e| e.without_url())?;
        ensure!(
            response.status().is_success(),
            "HTTP {} while announcing release",
            response.status()
        );
        Ok(response.json().map_err(|e| e.without_url())?)
    }
    fn github(&self, url: url::Url, body: Option<Value>) -> Result<Value> {
        let method = if body.is_some() {
            reqwest::Method::PATCH
        } else {
            reqwest::Method::GET
        };
        let mut r = self
            .client
            .request(method, url)
            .bearer_auth(&self.token)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28");
        if let Some(body) = body {
            r = r.json(&body);
        }
        Self::response(r.send())
    }
    fn api_url(repo: &str, parts: &[&str]) -> Result<url::Url> {
        let repo: Vec<_> = repo.split('/').collect();
        ensure!(
            repo.len() == 2
                && repo
                    .iter()
                    .all(|p| !p.is_empty() && !matches!(*p, "." | "..")),
            "expected owner/repository"
        );
        let mut url = url::Url::parse("https://api.github.com/")?;
        url.path_segments_mut()
            .map_err(|_| anyhow::anyhow!("API URL"))?
            .push("repos")
            .extend(repo)
            .extend(parts.iter().copied());
        Ok(url)
    }
}
impl Api for Http {
    fn fetch(&mut self, repo: &str, tag: &str) -> Result<Value> {
        self.github(Self::api_url(repo, &["releases", "tags", tag])?, None)
    }
    fn post(&mut self, content: &str) -> Result<Value> {
        let response = Self::response(
            self.client
                .post(webhook_url(&self.webhook)?)
                .json(&json!({"content":content,"allowed_mentions":{"parse":[]}}))
                .send(),
        )?;
        ensure!(
            response
                .get("id")
                .is_some_and(|v| v.as_str().is_some_and(|s| !s.is_empty()) || v.is_number()),
            "Discord returned no message id"
        );
        Ok(response)
    }
    fn mark(&mut self, repo: &str, id: &Value, body: &str) -> Result<()> {
        let id = id.as_u64().context("release ID missing")?.to_string();
        self.github(
            Self::api_url(repo, &["releases", &id])?,
            Some(json!({"body":body})),
        )?;
        Ok(())
    }
}
fn announce(api: &mut impl Api, repo: &str, tag: &str) -> Result<Option<String>> {
    let release = api.fetch(repo, tag)?;
    ensure!(
        release["draft"] != true,
        "refusing to announce draft release {tag}"
    );
    ensure!(
        !preview(tag) || release["prerelease"] == true,
        "refusing preview announcement for non-prerelease {tag}"
    );
    let body = release["body"].as_str().unwrap_or("");
    let marker = marker(tag);
    if body.contains(&marker) {
        println!("Discord announcement for {tag} is already recorded; skipping");
        return Ok(None);
    }
    let content = message(
        tag,
        release["name"].as_str().unwrap_or(tag),
        body,
        release["html_url"]
            .as_str()
            .context("release URL missing")?,
    );
    let posted = api.post(&content)?;
    let id = posted.get("id").context("Discord returned no message id")?;
    let id = id
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| id.to_string());
    ensure!(
        !id.is_empty() && id != "null",
        "Discord returned no message id"
    );
    println!("Posted {tag} to Discord as message {id}");
    // Even a refetch failure occurs after the public side effect. Do not invite
    // a workflow retry that would duplicate a successful announcement.
    let marked = (|| -> Result<()> {
        let latest = api.fetch(repo, tag)?;
        let body = latest["body"].as_str().unwrap_or("");
        if !body.contains(&marker) {
            api.mark(
                repo,
                &latest["id"],
                &format!("{}\n\n{marker}\n", body.trim_end()),
            )?;
        }
        Ok(())
    })();
    if let Err(e) = marked {
        eprintln!("::warning::Discord post succeeded, but the release marker failed: {e}");
    }
    Ok(Some(id))
}
#[derive(Parser)]
struct Args {
    #[arg(long)]
    tag: Option<String>,
    #[arg(long)]
    repository: Option<String>,
    /// Format an offline release JSON file without contacting GitHub or Discord.
    #[arg(long)]
    release_file: Option<PathBuf>,
}
pub fn run(args: impl Iterator<Item = String>) -> Result<()> {
    let args = Args::parse_from(std::iter::once("post-discord-release".into()).chain(args));
    let tag = args
        .tag
        .or_else(|| std::env::var("GITHUB_REF_NAME").ok())
        .filter(|s| !s.is_empty())
        .context("missing release tag")?;
    if let Some(path) = args.release_file {
        let v: Value = serde_json::from_slice(&std::fs::read(path)?)?;
        println!(
            "{}",
            message(
                &tag,
                v["name"].as_str().unwrap_or(&tag),
                v["body"].as_str().unwrap_or(""),
                v["html_url"].as_str().context("release URL")?
            )
        );
        return Ok(());
    }
    let repo = args
        .repository
        .or_else(|| std::env::var("GITHUB_REPOSITORY").ok())
        .filter(|s| !s.is_empty())
        .context("missing GitHub repository")?;
    let token = std::env::var("GH_TOKEN")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("GITHUB_TOKEN").ok())
        .filter(|s| !s.is_empty())
        .context("missing GH_TOKEN")?;
    let webhook =
        std::env::var("DISCORD_RELEASE_WEBHOOK").context("missing DISCORD_RELEASE_WEBHOOK")?;
    let mut api = Http {
        client: reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent("nexode-release-bot/2.0")
            .build()?,
        token,
        webhook,
    };
    announce(&mut api, &repo, &tag)?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    #[test]
    fn preview_has_only_commit_titles_and_one_unwrapped_link() {
        let body = "## What changed\n5 commits\n## Commits\n- [abc12345](https://example.test/a) Fix picker\n- [def67890](https://example.test/b) feat: see https://x.test/y\n## First-time preview setup\nDO NOT INCLUDE";
        assert_eq!(
            message(
                "v1.0.0-preview.12",
                "Preview",
                body,
                "https://example.test/preview"
            ),
            "**nexode v1.0.0-preview.12** (preview) https://example.test/preview\n- Fix picker\n- feat: see <https://x.test/y>"
        );
        for body in ["", "@everyone legacy notes", "## What changed\n\n"] {
            assert_eq!(
                message(
                    "v1.0.0-preview.12",
                    "Preview",
                    body,
                    "https://example.test/preview"
                ),
                "**nexode v1.0.0-preview.12** (preview) https://example.test/preview"
            );
        }
        let more = Regex::new(r"\n… \d+ more$").unwrap();
        for url in [
            "https://example.test/preview".to_owned(),
            format!("https://example.test/{}", "x".repeat(10000)),
        ] {
            let msg = message(
                "v1.0.0-preview.12",
                "Preview",
                &format!(
                    "## Commits\n{}",
                    "- [abcdef12](https://e.test/c) Fix picker\n".repeat(5000)
                ),
                &url,
            );
            assert!(msg.chars().count() <= LIMIT);
            assert!(more.is_match(&msg));
        }
    }
    #[test]
    fn stable_notes_remove_platform_block_and_bound_unicode() {
        let body = "## Highlights\n\nFast startup.\n\n<!-- nexode-platform-availability:start -->\nLinux available\n<!-- nexode-platform-availability:end -->\n";
        assert_eq!(
            message("v0.52.0", "v0.52.0", body, "https://e.test/v"),
            "## v0.52.0\n## Highlights\n\nFast startup."
        );
        let msg = message(
            "v0.52.0",
            "A large release",
            &"é".repeat(3000),
            "https://e.test/v",
        );
        assert_eq!(msg.chars().count(), LIMIT);
        assert!(msg.ends_with("… (full notes: <https://e.test/v>)"));
        assert_ne!(marker("v1"), marker("v2"));
    }
    struct Fake {
        releases: VecDeque<Value>,
        posts: usize,
        marked: Option<String>,
        fail_mark: bool,
    }
    impl Api for Fake {
        fn fetch(&mut self, _: &str, _: &str) -> Result<Value> {
            self.releases.pop_front().context("refetch unavailable")
        }
        fn post(&mut self, _: &str) -> Result<Value> {
            self.posts += 1;
            Ok(json!({"id":"message-54"}))
        }
        fn mark(&mut self, _: &str, _: &Value, body: &str) -> Result<()> {
            ensure!(!self.fail_mark, "marker failure");
            self.marked = Some(body.into());
            Ok(())
        }
    }
    #[test]
    fn announcement_gates_duplicates_refetch_and_partial_success() {
        let initial = json!({"id":54,"draft":false,"prerelease":true,"body":"initial notes","html_url":"https://e.test/v"});
        for (draft, pre, body) in [
            (true, true, ""),
            (false, false, ""),
            (
                false,
                true,
                "<!-- nexode-discord-announced:v1.0.0-preview.12 -->",
            ),
        ] {
            let mut release = initial.clone();
            release["draft"] = json!(draft);
            release["prerelease"] = json!(pre);
            release["body"] = json!(body);
            let mut api = Fake {
                releases: VecDeque::from([release]),
                posts: 0,
                marked: None,
                fail_mark: false,
            };
            let result = announce(&mut api, "o/r", "v1.0.0-preview.12");
            if body.is_empty() {
                assert!(result.is_err());
            } else {
                assert_eq!(result.unwrap(), None);
            }
            assert_eq!(api.posts, 0);
        }
        for fail_mark in [false, true] {
            let mut latest = initial.clone();
            latest["body"] = json!("notes edited during post");
            let mut api = Fake {
                releases: VecDeque::from([initial.clone(), latest]),
                posts: 0,
                marked: None,
                fail_mark,
            };
            assert_eq!(
                announce(&mut api, "o/r", "v0.54.0").unwrap().as_deref(),
                Some("message-54")
            );
            assert_eq!(api.posts, 1);
            if !fail_mark {
                assert!(
                    api.marked
                        .unwrap()
                        .starts_with("notes edited during post\n\n")
                );
            }
        }
        let mut api = Fake {
            releases: VecDeque::from([initial]),
            posts: 0,
            marked: None,
            fail_mark: false,
        };
        assert!(announce(&mut api, "o/r", "v0.54.0").is_ok());
        assert_eq!(api.posts, 1);
    }
    #[test]
    fn local_http_wait_mentions_rejection_and_missing_id() {
        for (status, reply, ok) in [
            (200, json!({"id":"message-123"}), true),
            (400, json!({"message":"rejected"}), false),
            (200, json!({}), false),
        ] {
            let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
            let url = format!("http://{}/webhook?thread_id=1", server.server_addr());
            let worker = std::thread::spawn(move || {
                let mut req = server.recv().unwrap();
                assert!(req.url().contains("thread_id=1"));
                assert!(req.url().contains("wait=true"));
                let body: Value = serde_json::from_reader(req.as_reader()).unwrap();
                assert_eq!(
                    body,
                    json!({"content":"Release @everyone","allowed_mentions":{"parse":[]}})
                );
                req.respond(
                    tiny_http::Response::from_string(reply.to_string()).with_status_code(status),
                )
                .unwrap();
            });
            let mut api = Http {
                client: reqwest::blocking::Client::builder()
                    .timeout(Duration::from_secs(3))
                    .build()
                    .unwrap(),
                token: "unused".into(),
                webhook: url,
            };
            assert_eq!(api.post("Release @everyone").is_ok(), ok);
            worker.join().unwrap();
        }
        assert_eq!(
            webhook_url("https://example.test/hook?wait=false")
                .unwrap()
                .query(),
            Some("wait=false")
        );
    }
}
