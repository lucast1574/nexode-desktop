use super::{Gh, Github, array_pages, ci, cmd, notes, objects};
use anyhow::{Context, Result, ensure};
use clap::Parser;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};
// Intentionally unconfigured: porting the tooling does not enable publication.
const REPOSITORY: &str = "nexode-agent/unconfigured";
fn validate(repo: &str, sha: &str, tag: &str) -> Result<()> {
    ensure!(
        repo == REPOSITORY,
        "Preview publication restricted to canonical repository"
    );
    ci::validate(repo, sha, 0.0, 0.0)?;
    ensure!(sha == sha.to_ascii_lowercase(), "expected lowercase SHA");
    ensure!(notes::number(tag).is_some(), "Expected v1.0.0-preview.N");
    Ok(())
}
fn head(api: &mut impl Github, repo: &str, sha: &str) -> Result<()> {
    ensure!(
        api.api(&format!("repos/{repo}/git/ref/heads/master"), false)?["object"]["sha"] == sha,
        "master advanced, refusing stale preview"
    );
    Ok(())
}
fn window(api: &mut impl Github, repo: &str) -> Result<()> {
    let refs = array_pages(api.api(&format!("repos/{repo}/git/matching-refs/tags/v1.0.0"), true)?)?;
    ensure!(
        !refs.iter().any(|v| v["ref"] == "refs/tags/v1.0.0"),
        "v1.0.0 stable exists, preview publishing has ended"
    );
    Ok(())
}
fn assets() -> BTreeSet<String> {
    ["linux", "macos", "windows"]
        .into_iter()
        .flat_map(|p| ["x86_64", "aarch64"].map(|a| format!("nexode-{p}-{a}.tar.gz")))
        .collect()
}
fn inventory(path: &Path) -> Result<BTreeMap<String, PathBuf>> {
    let mut files = BTreeMap::new();
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        if entry.file_type()?.is_file() {
            files.insert(
                entry
                    .file_name()
                    .into_string()
                    .map_err(|_| anyhow::anyhow!("invalid asset name"))?,
                entry.path(),
            );
        }
    }
    ensure!(
        files.keys().cloned().collect::<BTreeSet<_>>() == assets(),
        "Expected exact six-platform preview asset set"
    );
    for p in files.values() {
        ensure!(fs::metadata(p)?.len() > 0, "Empty preview asset");
    }
    Ok(files)
}
fn publish(
    api: &mut impl Github,
    repo: &str,
    sha: &str,
    tag: &str,
    directory: &Path,
) -> Result<()> {
    validate(repo, sha, tag)?;
    let mut files = inventory(directory)?;
    ci::authorize(repo, sha, 0.0, 30.0, api, true)?;
    window(api, repo)?;
    head(api, repo, sha)?;
    let changes = notes::changes(repo, sha, tag, api)?;
    let mut checksums = String::new();
    for (name, path) in &files {
        let mut file = fs::File::open(path)?;
        let mut hasher = Sha256::new();
        std::io::copy(&mut file, &mut hasher)?;
        checksums.push_str(&format!("{:x}  {name}\n", hasher.finalize()));
    }
    let checksum = directory.join("SHA256SUMS");
    fs::write(&checksum, checksums)?;
    let refs = array_pages(api.api(&format!("repos/{repo}/git/matching-refs/tags/{tag}"), true)?)?;
    ensure!(
        !refs.iter().any(|v| v["ref"] == format!("refs/tags/{tag}")),
        "Preview tag already exists, start a new workflow run"
    );
    cmd(
        api,
        &[
            "api",
            &format!("repos/{repo}/git/refs"),
            "-X",
            "POST",
            "-f",
            &format!("ref=refs/tags/{tag}"),
            "-f",
            &format!("sha={sha}"),
        ],
    )?;
    let text = format!(
        "Opt-in v1 preview built from master commit {sha}.\n\nExperimental binaries, not a stable release. Windows preview binaries are unsigned. Use the stable channel for production. Preview numbers may have gaps.\n\n{changes}{}",
        include_str!("preview_setup.txt")
    );
    let mut args: Vec<String> = [
        "release",
        "create",
        tag,
        "--repo",
        repo,
        "--verify-tag",
        "--draft",
        "--prerelease",
        "--latest=false",
        "--title",
        tag,
        "--notes",
        &text,
    ]
    .into_iter()
    .map(str::to_string)
    .collect();
    args.extend(files.values().map(|p| p.to_string_lossy().into_owned()));
    args.push(checksum.to_string_lossy().into());
    api.command(&args)?;
    let release: Value = serde_json::from_str(&cmd(
        api,
        &[
            "release",
            "view",
            tag,
            "--repo",
            repo,
            "--json",
            "isDraft,isPrerelease,assets",
        ],
    )?)?;
    files.insert("SHA256SUMS".into(), checksum);
    let remote = release["assets"]
        .as_array()
        .context("missing draft assets")?;
    ensure!(
        release["isDraft"] == true
            && release["isPrerelease"] == true
            && remote.len() == files.len(),
        "Draft assets incomplete"
    );
    let mut seen = BTreeSet::new();
    for asset in remote {
        let name = asset["name"].as_str().context("asset name")?;
        let path = files.get(name).context("unexpected draft asset")?;
        ensure!(
            seen.insert(name)
                && asset["state"] == "uploaded"
                && asset["size"].as_u64() == Some(fs::metadata(path)?.len()),
            "Draft assets incomplete"
        );
    }
    ensure!(
        api.api(&format!("repos/{repo}/git/ref/tags/{tag}"), false)?["object"]["sha"] == sha,
        "Preview tag changed during staging"
    );
    window(api, repo)?;
    head(api, repo, sha)?; // Last check immediately before visibility change.
    cmd(
        api,
        &[
            "release",
            "edit",
            tag,
            "--repo",
            repo,
            "--draft=false",
            "--prerelease",
            "--latest=false",
        ],
    )?;
    Ok(())
}
fn reconcile(api: &mut impl Github, repo: &str) -> Result<bool> {
    ensure!(
        repo == REPOSITORY,
        "Preview recovery restricted to canonical repository"
    );
    let current = api.api(&format!("repos/{repo}/git/ref/heads/master"), false)?;
    let sha = current["object"]["sha"]
        .as_str()
        .context("missing master SHA")?;
    ci::validate(repo, sha, 0.0, 0.0)?;
    let refs = array_pages(api.api(
        &format!("repos/{repo}/git/matching-refs/tags/v1.0.0?per_page=100"),
        true,
    )?)?;
    let skip = |reason: &str| {
        println!("SKIP: {reason}");
        Ok(false)
    };
    if refs.iter().any(|r| r["ref"] == "refs/tags/v1.0.0") {
        return skip("v1.0.0 stable exists, preview window has ended");
    }
    let runs = objects(
        api,
        &format!("repos/{repo}/actions/workflows/preview.yml/runs?head_sha={sha}&per_page=100"),
        "workflow_runs",
    )?;
    let runs: Vec<_> = runs
        .iter()
        .filter(|r| r["head_sha"] == sha && r["head_branch"] == "master")
        .collect();
    if runs
        .iter()
        .any(|r| ci::PENDING.contains(&r["status"].as_str().unwrap_or("")))
    {
        return skip("current master already has an active preview");
    }
    let tags: BTreeSet<_> = refs
        .iter()
        .filter(|r| r["object"]["sha"] == sha)
        .filter_map(|r| r["ref"].as_str()?.strip_prefix("refs/tags/"))
        .collect();
    if !tags.is_empty() {
        let releases = array_pages(api.api(&format!("repos/{repo}/releases?per_page=100"), true)?)?;
        if releases.iter().any(|r| {
            r["draft"] == false
                && r["prerelease"] == true
                && r["tag_name"]
                    .as_str()
                    .is_some_and(|t| notes::number(t).is_some() && tags.contains(t))
        }) {
            return skip("current master already has a published preview");
        }
    }
    if runs.len() >= 3 {
        return skip("automatic preview attempt limit (3) reached");
    }
    match ci::check_once(repo, sha, api, true) {
        Ok(None) => {}
        Ok(Some(reason)) => return skip(&format!("source CI not ready: {reason}")),
        Err(e) => return skip(&format!("source CI not authorized: {e}")),
    }
    // API errors are errors, a genuine changed head is a harmless skip.
    if api.api(&format!("repos/{repo}/git/ref/heads/master"), false)?["object"]["sha"] != sha {
        return skip("master advanced during reconciliation");
    }
    cmd(
        api,
        &[
            "api",
            &format!("repos/{repo}/actions/workflows/preview.yml/dispatches"),
            "--method",
            "POST",
            "-f",
            "ref=master",
        ],
    )?;
    println!("DISPATCHED: preview attempt {}/3 for {sha}", runs.len() + 1);
    Ok(true)
}
#[derive(Parser)]
struct Args {
    #[arg(value_parser=["check-head","publish"])]
    action: String,
    #[arg(long)]
    repo: Option<String>,
    #[arg(long)]
    sha: String,
    #[arg(long)]
    tag: String,
    #[arg(long, default_value = "dist")]
    assets: PathBuf,
}
pub fn run(args: impl Iterator<Item = String>) -> Result<()> {
    let a = Args::parse_from(std::iter::once("preview-release".into()).chain(args));
    let repo = a
        .repo
        .or_else(|| std::env::var("GITHUB_REPOSITORY").ok())
        .unwrap_or_default();
    validate(&repo, &a.sha, &a.tag)?;
    if a.action == "check-head" {
        window(&mut Gh, &repo)?;
        head(&mut Gh, &repo, &a.sha)
    } else {
        publish(&mut Gh, &repo, &a.sha, &a.tag, &a.assets)
    }
}
#[derive(Parser)]
struct RecoveryArgs {
    #[arg(long,default_value=REPOSITORY)]
    repo: String,
}
pub fn reconcile_cli(args: impl Iterator<Item = String>) -> Result<()> {
    let a =
        RecoveryArgs::parse_from(std::iter::once("reconcile-preview-release".into()).chain(args));
    reconcile(&mut Gh, &a.repo)?;
    Ok(())
}
#[cfg(test)]
mod tests;
