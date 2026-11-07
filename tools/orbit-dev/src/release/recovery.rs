//! Exact-byte, allowlisted historical repair with unchanged Git identity.
use super::{Gh, Github, cmd};
use anyhow::{Context, Result, ensure};
use clap::Parser;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{fs, path::Path, process::Command, time::Duration};
const SOURCE: &str = "crates/nexode-companion/src/server.rs";
const ORIGINAL: &str = "    let listener = bind(&options.socket)?;\n";
const REPLACEMENT: &str = "    let listener = bind(&options.socket)?;\n    // Windows named-pipe acceptance replaces the listening pipe instance.\n    #[cfg(windows)]\n    let mut listener = listener;\n";
const HELPERS: &[&str] = &[
    "scripts/install_justrust_ci.sh",
    "scripts/build_linux_compat.sh",
    "scripts/orbit_dev.sh",
];
struct Policy<'a> {
    original: &'a str,
    repaired: &'a str,
    commit: &'a str,
}
const REVIEWED: Policy = Policy {
    original: "e4215aa2a36fea7fadfb10235c47ecf34e91a37227e5657d8e909a44994cb2fe",
    repaired: "40ccfc4c74f15d013f7fc41b16f7509aa808c0b7f7576f038c53f8568954a1e6",
    commit: "d4f4c8c8d25c9f8e062773683adcc0c19cf26144",
};
fn git(root: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let output = crate::process::capture(
        Command::new("git").current_dir(root).args(args),
        Duration::from_secs(60),
    )?;
    ensure!(output.status.success(), "Git source verification failed");
    Ok(output.stdout)
}
fn text(root: &Path, args: &[&str]) -> Result<String> {
    Ok(String::from_utf8(git(root, args)?)?.trim().into())
}
pub fn stable_version(tag: &str) -> Result<[u64; 3]> {
    ensure!(
        regex::Regex::new(r"\Av[0-9]+\.[0-9]+\.[0-9]+\z")?.is_match(tag),
        "expected vMAJOR.MINOR.PATCH"
    );
    let parts: Vec<u64> = tag[1..]
        .split('.')
        .map(str::parse)
        .collect::<std::result::Result<_, _>>()?;
    Ok([parts[0], parts[1], parts[2]])
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn apply(
    root: &Path,
    tag: &str,
    repair: &str,
    workflow: &str,
    output: &Path,
    policy: &Policy<'_>,
) -> Result<Value> {
    stable_version(tag)?;
    ensure!(
        workflow.len() == 40
            && workflow
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        "Workflow commit must be full lowercase SHA"
    );
    let commit = text(
        root,
        &[
            "rev-parse",
            "--verify",
            &format!("refs/tags/{tag}^{{commit}}"),
        ],
    )?;
    ensure!(
        text(root, &["rev-parse", "HEAD"])? == commit,
        "Checkout must remain at release tag commit"
    );
    ensure!(
        text(root, &["status", "--porcelain", "--untracked-files=no"])?.is_empty(),
        "Tracked checkout must be clean before recovery"
    );
    let manifest: toml::Value =
        String::from_utf8(git(root, &["show", "HEAD:Cargo.toml"])?)?.parse()?;
    ensure!(
        manifest["package"]["version"].as_str() == Some(&tag[1..]),
        "Tag does not match package version"
    );
    let mut provenance = json!({"tag":tag,"tag_commit":commit,"workflow_commit":workflow,"repair":repair,"run_url":std::env::var("RELEASE_RUN_URL").unwrap_or_default(),"eligibility_policy":if repair=="none"{"exact-sha-main-ci"}else{"allowlisted-repair-native-companion-preflight"},"release_helpers":{}});
    if repair != "none" {
        ensure!(
            repair == "windows-listener" && ["v0.96.0", "v0.97.0"].contains(&tag),
            "Repair not allowlisted for this tag"
        );
        let original = git(root, &["show", &format!("HEAD:{SOURCE}")])?;
        let canonical = String::from_utf8(original.clone())?;
        ensure!(
            fs::read_to_string(root.join(SOURCE))?.replace("\r\n", "\n") == canonical,
            "Working source differs from canonical tagged source"
        );
        ensure!(
            hash(&original) == policy.original && canonical.matches(ORIGINAL).count() == 1,
            "Original source does not match reviewed repair"
        );
        let repaired = canonical.replace(ORIGINAL, REPLACEMENT);
        ensure!(
            hash(repaired.as_bytes()) == policy.repaired,
            "Repaired source does not match reviewed repair"
        );
        git(
            root,
            &["merge-base", "--is-ancestor", policy.commit, workflow],
        )?;
        ensure!(
            hash(&git(
                root,
                &["show", &format!("{}:{SOURCE}", policy.commit)]
            )?) == policy.repaired,
            "Committed repair differs from reviewed content"
        );
        fs::write(root.join(SOURCE), repaired)?;
        ensure!(
            text(root, &["diff", "--name-only"])? == SOURCE
                && text(root, &["rev-parse", "HEAD"])? == commit,
            "Recovery changed unexpected source or Git identity"
        );
        for (key, value) in [
            ("fix_commit", policy.commit),
            ("path", SOURCE),
            ("original_sha256", policy.original),
            ("repaired_sha256", policy.repaired),
        ] {
            provenance[key] = json!(value);
        }
    }
    for helper in HELPERS {
        let content = git(root, &["show", &format!("{workflow}:{helper}")])?;
        let path = root.join(helper);
        fs::write(&path, &content)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755))?;
        }
        provenance["release_helpers"][*helper] = json!(hash(&content));
    }
    fs::write(output, serde_json::to_string_pretty(&provenance)? + "\n")?;
    Ok(provenance)
}
fn public_state(api: &mut impl Github, tag: &str) -> Result<()> {
    let version = stable_version(tag)?;
    ensure!(
        cmd(
            api,
            &[
                "release", "view", tag, "--json", "isDraft", "--jq", ".isDraft"
            ]
        )?
        .trim()
            == "true",
        "Recovery requires existing draft release"
    );
    let releases: Value = serde_json::from_str(&cmd(
        api,
        &[
            "release",
            "list",
            "--exclude-drafts",
            "--limit",
            "1000",
            "--json",
            "tagName",
        ],
    )?)?;
    for release in releases.as_array().context("malformed releases")? {
        if let Some(name) = release["tagName"].as_str()
            && let Ok(other) = stable_version(name)
        {
            ensure!(
                other <= version,
                "Refusing recovery that would downgrade a newer public release"
            );
        }
    }
    Ok(())
}
#[derive(Parser)]
struct Args {
    tag: String,
    #[arg(default_value="none",value_parser=["none","windows-listener"])]
    repair: String,
    workflow_sha: Option<String>,
    #[arg(long)]
    check_public_state: bool,
}
pub fn run(args: impl Iterator<Item = String>) -> Result<()> {
    let a = Args::parse_from(std::iter::once("apply-release-recovery".into()).chain(args));
    if a.check_public_state {
        return public_state(&mut Gh, &a.tag);
    }
    let workflow = a
        .workflow_sha
        .context("workflow_sha required to prepare source")?;
    let output =
        std::env::var("RELEASE_PROVENANCE_PATH").unwrap_or("release-provenance.json".into());
    apply(
        &std::env::current_dir()?,
        &a.tag,
        &a.repair,
        &workflow,
        Path::new(&output),
        &REVIEWED,
    )?;
    Ok(())
}
#[cfg(test)]
mod tests;
