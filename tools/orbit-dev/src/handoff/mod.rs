//! Same-run artifact integrity, not a cryptographic build attestation.
mod preflight;
mod signer;
mod signing;
use anyhow::{Context, Result, ensure};
use clap::Parser;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};
const TOOLCHAIN: &str = "nightly-2026-10-08";
const REVISION: &str = "a7e7d02cbd50358d1aea056d3c17e5496586d62a";
const REPOSITORY: &str = "nexode-agent/unconfigured";
fn contract(lane: &str) -> Result<Value> {
    match lane {
        "linux-arm64" => Ok(
            json!({"target":"aarch64-unknown-linux-gnu","producer":"hosted-linux-x86_64","command":["build","--remote","--locked","--release","--bin","nexode","--target","aarch64-unknown-linux-gnu","-Zbuild-std=std,panic_abort","--features","linux-compat-vendored-openssl"],"features":["default","linux-compat-vendored-openssl"],"target_rustflags":"-Ztls-model=emulated"}),
        ),
        "macos-arm64" => Ok(
            json!({"target":"aarch64-apple-darwin","producer":"native-macos-arm64","command":["build","--local","--locked","--release","--bin","nexode","--target","aarch64-apple-darwin"],"features":["default"],"target_rustflags":""}),
        ),
        _ => anyhow::bail!("unknown handoff lane"),
    }
}
fn regular(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_file() && !metadata.is_symlink(),
        "not a regular file: {}",
        path.display()
    );
    Ok(())
}
fn digest(path: &Path) -> Result<String> {
    regular(path)?;
    let mut h = Sha256::new();
    std::io::copy(&mut fs::File::open(path)?, &mut h)?;
    Ok(format!("{:x}", h.finalize()))
}
fn architecture(path: &Path, lane: &str) -> Result<()> {
    regular(path)?;
    let mut header = vec![];
    fs::File::open(path)?.take(64).read_to_end(&mut header)?;
    if lane == "linux-arm64" {
        ensure!(
            header.len() >= 64
                && &header[..6] == b"\x7fELF\x02\x01"
                && u16::from_le_bytes([header[18], header[19]]) == 183
                && matches!(u16::from_le_bytes([header[16], header[17]]), 2 | 3),
            "expected little-endian AArch64 ELF64 executable"
        );
    } else {
        ensure!(
            lane == "macos-arm64"
                && header.len() >= 32
                && &header[..4] == b"\xcf\xfa\xed\xfe"
                && u32::from_le_bytes(header[4..8].try_into()?) == 0x0100000c
                && u32::from_le_bytes(header[12..16].try_into()?) == 2,
            "expected thin ARM64 Mach-O executable"
        );
    }
    Ok(())
}
fn context(env: &BTreeMap<String, String>) -> Result<Value> {
    let sha = env.get("GITHUB_SHA").context("GITHUB_SHA required")?;
    ensure!(
        sha.len() == 40
            && sha
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        "expected exact source SHA"
    );
    ensure!(
        env.get("GITHUB_REPOSITORY").map(String::as_str) == Some(REPOSITORY)
            && env.get("GITHUB_REF").map(String::as_str) == Some("refs/heads/master"),
        "only canonical master dispatches trusted"
    );
    let mut result = json!({});
    for key in [
        "GITHUB_SHA",
        "GITHUB_REPOSITORY",
        "GITHUB_REF",
        "GITHUB_RUN_ID",
        "GITHUB_RUN_ATTEMPT",
    ] {
        result[key] = json!(env.get(key).with_context(|| format!("{key} required"))?);
    }
    Ok(result)
}
fn expected(root: &Path, lane: &str, env: &BTreeMap<String, String>) -> Result<Value> {
    let c = contract(lane)?;
    Ok(
        json!({"schema":1,"context":context(env)?,"lane":lane,"toolchain":TOOLCHAIN,"justrust_revision":REVISION,"contract":c,"profile_opt_level":"3","lock_sha256":digest(&root.join("Cargo.lock"))?,"features":c["features"]}),
    )
}
fn receipt(
    directory: &Path,
    lane: &str,
    sealing: bool,
    env: &BTreeMap<String, String>,
) -> Result<Value> {
    let path = directory.join("build-report.json");
    regular(&path)?;
    let report: Value = serde_json::from_slice(&fs::read(&path)?)?;
    let expected: Vec<_> = contract(lane)?["command"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|a| *a != "--remote" && *a != "--local")
        .cloned()
        .collect();
    ensure!(
        report["exit"].as_i64() == Some(0) && report["args"] == json!(expected),
        "build receipt does not match successful pinned command"
    );
    let hosted = lane == "linux-arm64";
    let remote = &report["remote"];
    let remote = match remote {
        Value::Null => false,
        Value::Bool(v) => *v,
        Value::Object(v) => !v.is_empty(),
        Value::Array(v) => !v.is_empty(),
        Value::String(v) => !v.is_empty(),
        Value::Number(v) => v.as_f64().is_some_and(|n| n != 0.0),
    };
    ensure!(
        remote == hosted,
        "build receipt execution location mismatch"
    );
    if sealing && hosted {
        let id = report["id"].as_str().context("missing justrust run ID")?;
        ensure!(
            !id.is_empty()
                && id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b)),
            "invalid justrust run ID"
        );
        let home = env
            .get("JUSTRUST_HOME")
            .map(PathBuf::from)
            .or_else(|| dirs::home_dir().map(|p| p.join(".justrust")))
            .context("justrust home missing")?;
        let meta: Value =
            serde_json::from_slice(&fs::read(home.join("runs").join(id).join("meta.json"))?)?;
        ensure!(
            meta["place"] == "hosted",
            "ARM build did not run on hosted infrastructure"
        );
    }
    Ok(json!({"report_sha256":digest(&path)?,"place":if hosted{"hosted"}else{"native"}}))
}
fn seal(
    root: &Path,
    directory: &Path,
    lane: &str,
    env: &BTreeMap<String, String>,
) -> Result<String> {
    let binary = directory.join("nexode");
    architecture(&binary, lane)?;
    let mut manifest = expected(root, lane, env)?;
    manifest["build_receipt"] = receipt(directory, lane, true, env)?;
    manifest["binary_sha256"] = json!(digest(&binary)?);
    let path = directory.join("provenance.json");
    ensure!(
        !fs::symlink_metadata(&path).is_ok_and(|m| m.is_symlink()),
        "manifest destination is symlink"
    );
    fs::write(&path, serde_json::to_string_pretty(&manifest)? + "\n")?;
    digest(&path)
}
fn verify(
    root: &Path,
    directory: &Path,
    lane: &str,
    hash: &str,
    env: &BTreeMap<String, String>,
) -> Result<()> {
    let path = directory.join("provenance.json");
    ensure!(
        hash.len() == 64
            && hash
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            && digest(&path)? == hash,
        "manifest digest differs from producer output"
    );
    let manifest: Value = serde_json::from_slice(&fs::read(path)?)?;
    let binary = directory.join("nexode");
    architecture(&binary, lane)?;
    let mut wanted = expected(root, lane, env)?;
    wanted["build_receipt"] = receipt(directory, lane, false, env)?;
    wanted["binary_sha256"] = json!(digest(&binary)?);
    ensure!(
        manifest == wanted,
        "binary or same-run source/build provenance mismatch"
    );
    Ok(())
}
pub fn smoke(directory: &Path) -> Result<()> {
    let path = directory.join("nexode");
    regular(&path)?;
    let binary = fs::canonicalize(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o755))?;
    }
    let home = tempfile::tempdir()?;
    for (args, marker) in [
        (vec!["--version"], "nexode"),
        (vec!["--help"], "Usage:"),
        (vec!["run", "--help"], "Usage:"),
    ] {
        let mut cmd = Command::new(&binary);
        cmd.args(args)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", home.path())
            .env("XDG_CONFIG_HOME", home.path())
            .env("XDG_CACHE_HOME", home.path())
            .env("NEXODE_NO_UPDATE", "1")
            .env("TERM", "dumb")
            .current_dir(home.path());
        let result = crate::process::capture(&mut cmd, Duration::from_secs(30))?;
        ensure!(result.status.success(), "handoff smoke failed");
        let text = String::from_utf8(result.stdout)?;
        ensure!(text.contains(marker), "missing {marker} in smoke output");
        println!("{text}");
    }
    Ok(())
}
#[derive(Parser)]
struct Args {
    #[arg(value_parser=["seal","verify","smoke"])]
    action: String,
    #[arg(long,value_parser=["linux-arm64","macos-arm64"])]
    lane: String,
    #[arg(long, default_value = "handoff")]
    directory: PathBuf,
    #[arg(long, default_value = "")]
    manifest_digest: String,
}
pub fn run(command: &str, args: impl Iterator<Item = String>) -> Result<()> {
    match command {
        "native-handoff" => {
            let a = Args::parse_from(std::iter::once(command.into()).chain(args));
            let env = std::env::vars().collect();
            let root = std::env::current_dir()?;
            if a.action == "seal" {
                println!("{}", seal(&root, &a.directory, &a.lane, &env)?);
            } else {
                verify(&root, &a.directory, &a.lane, &a.manifest_digest, &env)?;
                if a.action == "smoke" {
                    smoke(&a.directory)?;
                }
            }
            Ok(())
        }
        "native-handoff-preflight" => preflight::run(args),
        "sign-release" => signer::run(args),
        "signing-check" => signing::run(args),
        _ => anyhow::bail!("unknown handoff command"),
    }
}
#[cfg(test)]
mod tests;
