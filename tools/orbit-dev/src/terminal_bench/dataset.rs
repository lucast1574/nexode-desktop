//! Resolve the legacy Harbor registry directly, pinning each task's Git commit.
use super::*;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io::Read,
    process::{Command, Stdio},
    time::{Duration, Instant},
};
pub(super) fn resolve(dataset: &str) -> Result<PathBuf> {
    let (name, version) = dataset
        .rsplit_once('@')
        .context("registry dataset must include @version")?;
    let cache = dirs::cache_dir()
        .context("cache directory unavailable")?
        .join("orbit-terminal-bench")
        .join(format!("{:x}", Sha256::digest(dataset.as_bytes())));
    if cache.join("registry-selection.json").is_file() {
        return Ok(cache.join("tasks"));
    }
    fs::create_dir_all(cache.parent().unwrap())?;
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()?;
    let response = client
        .get(env(
            "NEXODE_TB_REGISTRY_URL",
            "https://raw.githubusercontent.com/harbor-framework/harbor/main/registry.json",
        ))
        .send()?
        .error_for_status()?;
    let mut bytes = vec![];
    response
        .take(64 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 64 * 1024 * 1024, "registry exceeds 64 MiB");
    let registry: Value = serde_json::from_slice(&bytes)?;
    let selected = registry
        .as_array()
        .context("registry must be an array")?
        .iter()
        .find(|v| v["name"] == name && v["version"] == version)
        .context("dataset/version not present in registry; use --path for an installed dataset")?;
    let temp = tempfile::tempdir_in(cache.parent().unwrap())?;
    let output = temp.path().join("tasks");
    fs::create_dir(&output)?;
    let mut repos: BTreeMap<String, PathBuf> = BTreeMap::new();
    for task in selected["tasks"].as_array().context("dataset tasks")? {
        let name = task["name"].as_str().context("task name")?;
        safe_name(name)?;
        let url = task["git_url"].as_str().context("task Git URL")?;
        let parsed = url::Url::parse(url)?;
        ensure!(
            parsed.scheme() == "https"
                && parsed.username().is_empty()
                && parsed.password().is_none(),
            "registry Git URL must use HTTPS without credentials"
        );
        let commit = task["git_commit_id"].as_str().context("task Git commit")?;
        ensure!(
            commit.len() == 40 && commit.bytes().all(|b| b.is_ascii_hexdigit()),
            "registry must pin full Git commit IDs"
        );
        let key = format!("{url}#{commit}");
        let checkout = if let Some(p) = repos.get(&key) {
            p.clone()
        } else {
            let p = temp.path().join(format!("repo-{}", repos.len()));
            fs::create_dir(&p)?;
            git(&p, &["init", "--quiet"])?;
            git(
                &p,
                &[
                    "-c",
                    "protocol.file.allow=never",
                    "fetch",
                    "--quiet",
                    "--depth",
                    "1",
                    url,
                    commit,
                ],
            )?;
            git(&p, &["checkout", "--quiet", "--detach", "FETCH_HEAD"])?;
            repos.insert(key, p.clone());
            p
        };
        let relative = Path::new(task["path"].as_str().context("task path")?);
        ensure!(
            !relative.is_absolute()
                && !relative
                    .components()
                    .any(|c| matches!(c, std::path::Component::ParentDir)),
            "registry task path escapes checkout"
        );
        let source = checkout.join(relative).canonicalize()?;
        ensure!(
            source.starts_with(checkout.canonicalize()?),
            "registry task symlink escapes checkout"
        );
        copy(&source, &output.join(name), &checkout.canonicalize()?)?;
    }
    write_json(&temp.path().join("registry-selection.json"), selected)?;
    // Publish only the tasks and exact registry entry, not intermediate Git clones.
    let final_dir = tempfile::tempdir_in(cache.parent().unwrap())?;
    fs::rename(&output, final_dir.path().join("tasks"))?;
    fs::rename(
        temp.path().join("registry-selection.json"),
        final_dir.path().join("registry-selection.json"),
    )?;
    if cache.exists() {
        ensure!(
            cache.join("registry-selection.json").is_file(),
            "incomplete dataset cache; choose a separate cache location"
        );
        return Ok(cache.join("tasks"));
    }
    fs::rename(final_dir.path(), &cache)?;
    Ok(cache.join("tasks"))
}
fn copy(from: &Path, to: &Path, allowed: &Path) -> Result<()> {
    let resolved = from.canonicalize()?;
    ensure!(
        resolved.starts_with(allowed),
        "dataset symlink escapes checkout"
    );
    if resolved.is_dir() {
        fs::create_dir(to)?;
        for e in fs::read_dir(resolved)? {
            let e = e?;
            copy(&e.path(), &to.join(e.file_name()), allowed)?;
        }
    } else {
        fs::copy(resolved, to)?;
    }
    Ok(())
}
fn git(cwd: &Path, args: &[&str]) -> Result<()> {
    let mut c = Command::new("git");
    c.args(["-c", "core.hooksPath=/dev/null"])
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    use std::os::unix::process::CommandExt;
    c.process_group(0);
    let mut child = c.spawn()?;
    let end = Instant::now() + Duration::from_secs(180);
    loop {
        if let Some(s) = child.try_wait()? {
            ensure!(s.success(), "dataset git operation failed");
            return Ok(());
        }
        if Instant::now() > end {
            unsafe {
                libc::kill(-(child.id() as i32), libc::SIGKILL);
            };
            let _ = child.wait();
            anyhow::bail!("dataset git operation timed out")
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}
