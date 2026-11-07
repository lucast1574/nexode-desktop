use super::{Gh, Github, ci, cmd, recovery};
use anyhow::{Context, Result, ensure};
use clap::Parser;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};
fn required() -> BTreeSet<String> {
    let mut names: BTreeSet<_> = [
        "linux-x86_64",
        "linux-aarch64",
        "macos-aarch64",
        "macos-x86_64",
        "windows-x86_64",
        "windows-aarch64",
        "freebsd-x86_64",
    ]
    .iter()
    .map(|p| format!("nexode-{p}.tar.gz"))
    .collect();
    names.extend(
        [
            "nexode-windows-x86_64.exe",
            "nexode-windows-aarch64.exe",
            "SHA256SUMS",
            "release-provenance.json",
        ]
        .map(str::to_string),
    );
    names
}
fn metadata(value: &Value, tag: &str, allow_draft: bool) -> Result<BTreeSet<String>> {
    ensure!(
        value["tag_name"] == tag
            && value["draft"].is_boolean()
            && (value["draft"] == false || allow_draft),
        "Release must be public and match requested tag"
    );
    ensure!(
        value["prerelease"] == false,
        "Official release must not be prerelease"
    );
    let mut names = BTreeSet::new();
    for asset in value["assets"].as_array().context("missing assets")? {
        let name = asset["name"].as_str().context("invalid asset name")?;
        ensure!(
            !name.is_empty()
                && ![".", ".."].contains(&name)
                && !name.contains(['/', '\\', '\0'])
                && asset["state"] == "uploaded"
                && asset["size"].as_u64().is_some_and(|n| n > 0)
                && names.insert(name.to_string()),
            "unsafe, incomplete, empty or duplicate release asset"
        );
    }
    ensure!(
        required().is_subset(&names),
        "Missing required release assets"
    );
    Ok(names)
}
fn hash(path: &Path) -> Result<String> {
    let mut h = Sha256::new();
    std::io::copy(&mut fs::File::open(path)?, &mut h)?;
    Ok(format!("{:x}", h.finalize()))
}
fn downloads(directory: &Path, names: &BTreeSet<String>, tag: &str, sha: &str) -> Result<()> {
    for name in names {
        let meta = fs::symlink_metadata(directory.join(name))
            .context("Missing or unsafe downloaded asset")?;
        ensure!(
            meta.is_file() && !meta.is_symlink() && meta.len() > 0,
            "Missing or unsafe downloaded asset: {name}"
        );
    }
    let line = regex::Regex::new(r"\A([0-9a-fA-F]{64}) [ *]([^/\\]+)\z")?;
    let mut sums = BTreeMap::new();
    for text in fs::read_to_string(directory.join("SHA256SUMS"))?.lines() {
        let c = line.captures(text).context("Malformed checksum entry")?;
        ensure!(
            sums.insert(c[2].to_string(), c[1].to_ascii_lowercase())
                .is_none(),
            "Duplicate checksum entry"
        );
    }
    ensure!(
        sums.keys().cloned().collect::<BTreeSet<_>>()
            == names
                .iter()
                .filter(|n| n.as_str() != "SHA256SUMS")
                .cloned()
                .collect(),
        "Checksum manifest must cover exactly published assets"
    );
    for (name, sum) in sums {
        ensure!(
            hash(&directory.join(&name))? == sum,
            "Checksum mismatch: {name}"
        );
    }
    let p: Value = serde_json::from_slice(&fs::read(directory.join("release-provenance.json"))?)?;
    ensure!(
        p["tag"] == tag
            && p["tag_commit"] == sha
            && p["repair"] == "none"
            && p["eligibility_policy"] == "exact-sha-main-ci",
        "Release provenance does not match validated unpatched commit"
    );
    Ok(())
}
fn fetch(api: &mut impl Github, repo: &str, tag: &str, draft: bool) -> Result<Value> {
    if !draft {
        return api.api(&format!("repos/{repo}/releases/tags/{tag}"), false);
    }
    for page in 1..=10000 {
        let value = api.api(
            &format!("repos/{repo}/releases?per_page=100&page={page}"),
            false,
        )?;
        let releases = value.as_array().context("malformed releases")?;
        let matching: Vec<_> = releases.iter().filter(|r| r["tag_name"] == tag).collect();
        ensure!(matching.len() < 2, "Multiple releases claim tag");
        if let Some(found) = matching.first() {
            return Ok((*found).clone());
        }
        ensure!(releases.len() == 100, "No release found for tag {tag}");
    }
    anyhow::bail!("Release pagination limit exceeded")
}
#[derive(Parser)]
struct Args {
    tag: String,
    #[arg(long, default_value = "nexode-agent/unconfigured")]
    repo: String,
    #[arg(long)]
    sha: String,
    #[arg(long)]
    allow_draft: bool,
    #[arg(long)]
    directory: Option<PathBuf>,
}
pub fn run(args: impl Iterator<Item = String>) -> Result<()> {
    let a = Args::parse_from(std::iter::once("verify-public-release".into()).chain(args));
    recovery::stable_version(&a.tag)?;
    ci::validate(&a.repo, &a.sha, 0.0, 0.0)?;
    ensure!(
        a.sha == a.sha.to_ascii_lowercase(),
        "expected lowercase SHA"
    );
    let fetched = fetch(&mut Gh, &a.repo, &a.tag, a.allow_draft)?;
    let names = metadata(&fetched, &a.tag, a.allow_draft)?;
    let temp;
    let directory = if let Some(path) = a.directory {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::create_dir(&path)?;
        path
    } else {
        temp = if let Some(path) = std::env::var_os("NEXODE_SCRATCH_DIR") {
            tempfile::tempdir_in(path)?
        } else {
            tempfile::tempdir()?
        };
        temp.path().to_path_buf()
    };
    cmd(
        &mut Gh,
        &[
            "release",
            "download",
            &a.tag,
            "--repo",
            &a.repo,
            "--dir",
            directory.to_str().context("invalid directory")?,
        ],
    )?;
    downloads(&directory, &names, &a.tag, &a.sha)?;
    println!(
        "VERIFIED: {}, {} assets, exact provenance and SHA256 hashes",
        a.tag,
        names.len()
    );
    Ok(())
}
pub fn checksums(root: &Path, output: &Path) -> Result<()> {
    fn walk(path: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            if kind.is_dir() {
                walk(&entry.path(), files)?;
            } else if kind.is_file() {
                let name = entry.file_name();
                if name.to_string_lossy().ends_with(".tar.gz")
                    || name.to_string_lossy().ends_with(".exe")
                {
                    files.push(entry.path());
                }
            }
        }
        Ok(())
    }
    let mut files = vec![];
    walk(root, &mut files)?;
    files.sort();
    ensure!(!files.is_empty(), "No release assets for checksums");
    let mut names = BTreeSet::new();
    let mut lines = String::new();
    for p in files {
        let name = p
            .file_name()
            .and_then(|s| s.to_str())
            .context("invalid asset filename")?;
        ensure!(names.insert(name.to_string()), "duplicate asset basename");
        lines.push_str(&format!("{}  {name}\n", hash(&p)?));
    }
    fs::write(output, lines)?;
    Ok(())
}
pub fn availability(path: &Path, env: impl Fn(&str) -> Option<String>) -> Result<()> {
    let start = "<!-- nexode-platform-availability:start -->";
    let end = "<!-- nexode-platform-availability:end -->";
    let body = fs::read_to_string(path)?;
    let re = regex::Regex::new(&format!(
        "(?s)\\n?{}.*?{}\\n?",
        regex::escape(start),
        regex::escape(end)
    ))?;
    let body = re.replace_all(&body, "\n");
    let mut output = format!(
        "{}\n\n{start}\n## Platform availability\n\n",
        body.trim_end()
    );
    for (label, key) in [
        ("Linux x86_64", "LINUX_X86_64"),
        ("Linux aarch64", "LINUX_AARCH64"),
        ("macOS Apple Silicon", "MACOS_AARCH64"),
        ("macOS Intel", "MACOS_X86_64"),
        ("Windows x86_64", "WINDOWS_X86_64"),
        ("Windows ARM64", "WINDOWS_AARCH64"),
        ("FreeBSD x86_64", "FREEBSD_X86_64"),
    ] {
        let value = env(&format!("{key}_AVAILABLE")).context("missing platform availability")?;
        let state = if value == "true" {
            "available"
        } else {
            "not available in this release"
        };
        output.push_str(&format!("- {label}: {state}\n"));
    }
    output.push('\n');
    output.push_str(if env("WINDOWS_SIGNING_ENABLED").as_deref()==Some("true"){"Authenticode signatures verified for both Windows architectures."}else{"Windows executables are **unsigned**. Azure Artifact Signing is not configured and the repository's existing `WINDOWS_SIGNING_REQUIRED=false` override allowed unsigned publication. Windows may show publisher or SmartScreen warnings."});
    output.push_str(&format!("\n{end}\n"));
    fs::write(path, output)?;
    Ok(())
}
#[cfg(test)]
mod tests;
