//! Rust port of justrust's hash-pinned signing contract at 17c344f2.
//! Upstream copyright and MIT terms are retained in the repository NOTICE.
use super::*;
use std::io::{Seek, SeekFrom};
#[derive(Parser, Clone)]
struct Args {
    #[arg(long)]
    input: PathBuf,
    #[arg(long)]
    output: PathBuf,
    #[arg(long)]
    sha256: String,
    #[arg(long,value_parser=["aarch64-apple-darwin","x86_64-apple-darwin","x86_64-pc-windows-msvc","aarch64-pc-windows-msvc"])]
    target: String,
    #[arg(long)]
    identity: Option<String>,
    #[arg(long, default_value = "com.nexode.cli")]
    identifier: String,
    #[arg(long)]
    thumbprint: Option<String>,
    #[arg(long)]
    signtool: Option<PathBuf>,
    #[arg(long, default_value = "https://timestamp.digicert.com")]
    timestamp_url: String,
    #[arg(long)]
    execute: bool,
}
fn hex(s: &str, n: usize) -> bool {
    s.len() == n && s.bytes().all(|b| b.is_ascii_hexdigit())
}
fn header(path: &Path, target: &str) -> Result<()> {
    let mac = target.ends_with("apple-darwin");
    let arm = target.starts_with("aarch64");
    let mut file = fs::File::open(path)?;
    let mut head = vec![];
    (&mut file).take(64).read_to_end(&mut head)?;
    if mac {
        ensure!(
            head.len() >= 32
                && &head[..4] == b"\xcf\xfa\xed\xfe"
                && u32::from_le_bytes(head[4..8].try_into()?)
                    == if arm { 0x0100000c } else { 0x01000007 }
                && u32::from_le_bytes(head[12..16].try_into()?) == 2,
            "Mach-O architecture or executable type mismatch"
        );
    } else {
        ensure!(
            head.len() == 64 && &head[..2] == b"MZ",
            "expected PE executable"
        );
        let offset = u32::from_le_bytes(head[60..64].try_into()?);
        ensure!(offset >= 64, "invalid PE offset");
        file.seek(SeekFrom::Start(offset.into()))?;
        let mut pe = [0u8; 26];
        file.read_exact(&mut pe)?;
        let flags = u16::from_le_bytes(pe[22..24].try_into()?);
        ensure!(
            &pe[..4] == b"PE\0\0"
                && u16::from_le_bytes(pe[4..6].try_into()?) == if arm { 0xaa64 } else { 0x8664 }
                && flags & 2 != 0
                && flags & 0x2000 == 0
                && u16::from_le_bytes(pe[24..26].try_into()?) == 0x20b,
            "PE architecture or executable header mismatch"
        );
    }
    Ok(())
}
fn commands(a: &Args, artifact: &str) -> Result<Vec<Vec<String>>> {
    let make = |v: Vec<&str>| v.into_iter().map(str::to_string).collect();
    if a.target.ends_with("apple-darwin") {
        let identity = a
            .identity
            .as_deref()
            .context("Developer ID identity required")?;
        ensure!(
            hex(identity, 40) || super::signing::team(identity).is_ok(),
            "full Developer ID Application name or 40-hex certificate SHA1 required"
        );
        ensure!(
            a.thumbprint.is_none() && a.signtool.is_none(),
            "Windows arguments cannot be used for macOS"
        );
        ensure!(
            regex::Regex::new(r"\A[A-Za-z0-9][A-Za-z0-9.-]*\z")?.is_match(&a.identifier),
            "invalid signing identifier"
        );
        let requirement = format!(
            "=anchor apple generic and certificate 1[field.1.2.840.113635.100.6.2.6] exists and certificate leaf[field.1.2.840.113635.100.6.1.13] exists and identifier \"{}\"",
            a.identifier
        );
        Ok(vec![
            make(vec![
                "/usr/bin/codesign",
                "--force",
                "--sign",
                identity,
                "--identifier",
                &a.identifier,
                "--options",
                "runtime",
                "--timestamp",
                artifact,
            ]),
            make(vec![
                "/usr/bin/codesign",
                "--verify",
                "--strict",
                "--verbose=2",
                "-R",
                &requirement,
                artifact,
            ]),
        ])
    } else {
        ensure!(
            a.identity.is_none(),
            "macOS identity cannot be used for Windows"
        );
        let thumb = a
            .thumbprint
            .as_deref()
            .context("installed certificate thumbprint required")?;
        ensure!(hex(thumb, 40), "expected 40-hex thumbprint");
        let tool = a
            .signtool
            .as_ref()
            .context("absolute signtool path required")?;
        ensure!(
            tool.is_absolute(),
            "absolute trusted signtool path required"
        );
        let tool = tool.to_str().context("invalid signtool path")?;
        ensure!(
            regex::Regex::new(r"\Ahttps://[A-Za-z0-9.-]+(?::[0-9]+)?(?:/[A-Za-z0-9./_-]*)?\z")?
                .is_match(&a.timestamp_url),
            "timestamp URL must be HTTPS without credentials or query"
        );
        Ok(vec![
            make(vec![
                tool,
                "sign",
                "/s",
                "My",
                "/sha1",
                thumb,
                "/fd",
                "SHA256",
                "/tr",
                &a.timestamp_url,
                "/td",
                "SHA256",
                artifact,
            ]),
            make(vec![tool, "verify", "/pa", "/all", "/v", artifact]),
        ])
    }
}
fn environment(mac: bool) -> BTreeMap<String, String> {
    let keys: &[&str] = if mac {
        &["HOME", "USER", "LOGNAME", "TMPDIR"]
    } else {
        &[
            "SystemRoot",
            "WINDIR",
            "USERPROFILE",
            "APPDATA",
            "LOCALAPPDATA",
            "TEMP",
            "TMP",
        ]
    };
    let mut env: BTreeMap<_, _> = keys
        .iter()
        .filter_map(|k| std::env::var(k).ok().map(|v| (k.to_string(), v)))
        .collect();
    if mac {
        env.insert("PATH".into(), "/usr/bin:/bin".into());
    }
    env
}
fn sign(
    a: &Args,
    host: &str,
    mut native: impl FnMut(&[String], &BTreeMap<String, String>) -> Result<()>,
) -> Result<Value> {
    ensure!(
        hex(&a.sha256, 64),
        "expected SHA256 must be 64 hex characters"
    );
    let source = fs::canonicalize(&a.input)?;
    let output = std::path::absolute(&a.output)?;
    ensure!(
        source.is_file() && fs::symlink_metadata(&output).is_err(),
        "output must be distinct and not exist; input must be file"
    );
    let parent = output.parent().context("missing output parent")?;
    ensure!(parent.is_dir(), "output parent must already exist");
    ensure!(
        digest(&source)? == a.sha256.to_ascii_lowercase(),
        "input SHA256 mismatch"
    );
    header(&source, &a.target)?;
    let planned = commands(a, "<temporary-artifact>")?;
    let mac = a.target.ends_with("apple-darwin");
    let system = if mac { "Darwin" } else { "Windows" };
    if !a.execute {
        return Ok(
            json!({"mode":"plan-only","target":a.target,"output":output,"commands":planned,"requires_host":system}),
        );
    }
    ensure!(host == system, "execution requires target operating system");
    let staged = tempfile::Builder::new()
        .prefix(".sign-release-")
        .suffix(if mac { "" } else { ".exe" })
        .tempfile_in(parent)?;
    fs::copy(&source, staged.path())?;
    ensure!(
        digest(staged.path())? == a.sha256.to_ascii_lowercase(),
        "copied input SHA256 mismatch"
    );
    header(staged.path(), &a.target)?;
    let env = environment(mac);
    for command in commands(a, staged.path().to_str().context("invalid staging path")?)? {
        native(&command, &env)?;
    }
    // Atomic no-clobber persistence keeps a concurrent destination intact.
    staged.persist_noclobber(&output).map_err(|e| e.error)?;
    Ok(json!({"mode":"signed-and-native-verified","output":output}))
}
pub fn run(args: impl Iterator<Item = String>) -> Result<()> {
    let a = Args::parse_from(std::iter::once("sign-release".into()).chain(args));
    let host = if cfg!(target_os = "macos") {
        "Darwin"
    } else if cfg!(windows) {
        "Windows"
    } else {
        "Other"
    };
    let value = sign(&a, host, |args, env| {
        let out = crate::process::capture(
            Command::new(&args[0])
                .args(&args[1..])
                .env_clear()
                .envs(env),
            Duration::from_secs(300),
        )?;
        ensure!(out.status.success(), "native signer or verification failed");
        Ok(())
    })?;
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}
#[cfg(test)]
mod tests;
