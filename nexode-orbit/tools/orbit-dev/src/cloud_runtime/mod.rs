//! Immutable dependency bundle staging; activation never reloads the daemon.
use anyhow::{Context, Result, ensure};
use clap::Parser;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};

fn install_script(checksum: &str, activate: bool) -> Result<String> {
    ensure!(
        checksum.len() == 64
            && checksum
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        "invalid SHA-256"
    );
    let mut script = include_str!("stage.sh").replace("CHECKSUM", checksum);
    if activate {
        script.push_str(include_str!("activate.sh"));
    }
    Ok(script)
}
#[cfg(target_os = "linux")]
fn bundle(binary: &Path, directory: &Path) -> Result<(PathBuf, String)> {
    use std::os::unix::fs::PermissionsExt;
    let binary = binary.canonicalize()?;
    let payload = directory.join("payload");
    fs::create_dir(&payload)?;
    fs::copy(&binary, payload.join("nexode.bin"))?;
    let result = crate::process::capture(
        Command::new("ldd")
            .arg(&binary)
            .env_remove("LD_PRELOAD")
            .env_remove("LD_LIBRARY_PATH"),
        Duration::from_secs(30),
    )?;
    ensure!(
        result.status.success(),
        "ldd failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let mut loader = None;
    for line in String::from_utf8(result.stdout)?.lines() {
        ensure!(!line.contains("not found"), "missing dependency: {line}");
        if let Some(path) = line.split_whitespace().find(|p| p.starts_with('/')) {
            let path = Path::new(path);
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .context("invalid dependency path")?;
            ensure!(
                name.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b)),
                "unsupported dependency name"
            );
            fs::copy(path, payload.join(name))?;
            if name.starts_with("ld-linux") {
                loader = Some(name.to_owned());
            }
        }
    }
    let loader = loader.context("binary must have a Linux dynamic loader")?;
    let launcher = payload.join("nexode");
    fs::write(
        &launcher,
        format!(
            "#!/bin/sh\nset -eu\np=$(readlink -f \"$0\")\nd=$(dirname \"$p\")\nexec \"$d/{loader}\" --library-path \"$d\" \"$p.bin\" \"$@\"\n"
        ),
    )?;
    fs::set_permissions(&launcher, fs::Permissions::from_mode(0o755))?;
    let archive = directory.join("runtime.tar.gz");
    let encoder =
        flate2::write::GzEncoder::new(File::create(&archive)?, flate2::Compression::fast());
    let mut tar = tar::Builder::new(encoder);
    let mut entries = fs::read_dir(&payload)?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        tar.append_path_with_name(e.path(), e.file_name())?;
    }
    tar.into_inner()?.finish()?;
    let mut hash = Sha256::new();
    std::io::copy(&mut File::open(&archive)?, &mut hash)?;
    Ok((archive, format!("{:x}", hash.finalize())))
}
#[derive(Parser)]
struct Args {
    binary: PathBuf,
    #[arg(long, default_value = "nexode-cloud")]
    host: String,
    #[arg(long)]
    ssh_config: Option<PathBuf>,
    #[arg(long)]
    activate_launcher: bool,
}
fn upload(
    archive: &Path,
    script: &str,
    a: &Args,
    config: &Path,
    ssh: &Path,
    retry_delay: Duration,
) -> Result<i32> {
    let attempts = if a.activate_launcher { 1 } else { 3 };
    for attempt in 0..attempts {
        let command = format!("bash -c {}", shlex::try_quote(script)?);
        let status = Command::new(ssh)
            .args(["-T", "-o", "BatchMode=yes", "-o", "ForwardAgent=no", "-F"])
            .arg(config)
            .arg("--")
            .arg(&a.host)
            .arg(command)
            .stdin(Stdio::from(File::open(archive)?))
            .status()?;
        let code = status.code().unwrap_or(1);
        if code != 255 || attempt + 1 == attempts {
            return Ok(code);
        }
        eprintln!("SSH dropped while staging, retrying verified bundle");
        std::thread::sleep(retry_delay * (attempt + 1));
    }
    unreachable!()
}
pub fn run(args: impl Iterator<Item = String>) -> Result<i32> {
    let a = Args::parse_from(std::iter::once("update-cloud-runtime".into()).chain(args));
    #[cfg(target_os = "linux")]
    {
        let home = dirs::home_dir().context("home directory unavailable")?;
        let scratch = std::env::var_os("NEXODE_SCRATCH_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".nexode/scratch"));
        fs::create_dir_all(&scratch)?;
        let directory = tempfile::Builder::new()
            .prefix("nexode-runtime-")
            .tempdir_in(scratch)?;
        let (archive, checksum) = bundle(&a.binary, directory.path())?;
        let config = a
            .ssh_config
            .clone()
            .unwrap_or_else(|| home.join(".nexode/cloud/ssh_config"));
        upload(
            &archive,
            &install_script(&checksum, a.activate_launcher)?,
            &a,
            &config,
            Path::new("ssh"),
            Duration::from_secs(2),
        )
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = a;
        anyhow::bail!("runtime bundling requires Linux");
    }
}
#[cfg(all(test, target_os = "linux"))]
mod tests;
