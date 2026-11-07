//! Local release-update rehearsal. No Python, gzip process, or live release API.
mod server;
use anyhow::{Context, Result, ensure};
use clap::Parser;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
#[derive(Parser, Debug)]
#[command(
    name = "cargo xtask update-rehearsal",
    about = "Run the real updater against an isolated local fake release"
)]
pub struct Args {
    #[arg(long)]
    binary: Option<PathBuf>,
    #[arg(long)]
    root: Option<PathBuf>,
    #[arg(long)]
    from_version: Option<String>,
    #[arg(long)]
    to_version: Option<String>,
    #[arg(long, default_value_t = 10.0)]
    rate: f64,
    #[arg(long)]
    drop_at: Option<f64>,
    #[arg(long, default_value_t = 0.0)]
    drop_stall: f64,
    #[arg(long, default_value_t = 0.0)]
    check_delay: f64,
    #[arg(long)]
    bad_checksum: bool,
    #[arg(long)]
    up_to_date: bool,
    #[arg(long)]
    manual: bool,
    #[arg(long)]
    no_strip: bool,
    #[arg(long)]
    keep: bool,
    #[arg(long)]
    prepare_only: bool,
    #[arg(long)]
    headless_check: bool,
    #[arg(last = true)]
    nexode_args: Vec<String>,
}
fn version(v: &str) -> Result<[u64; 3]> {
    let parts: Vec<_> = v
        .split('.')
        .map(str::parse::<u64>)
        .collect::<std::result::Result<_, _>>()?;
    ensure!(parts.len() == 3, "expected major.minor.patch version");
    Ok([parts[0], parts[1], parts[2]])
}
fn stem() -> Result<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Ok("nexode-linux-x86_64"),
        ("linux", "aarch64") => Ok("nexode-linux-aarch64"),
        ("macos", "x86_64") => Ok("nexode-macos-x86_64"),
        ("macos", "aarch64") => Ok("nexode-macos-aarch64"),
        _ => anyhow::bail!("update rehearsal currently supports Linux and macOS"),
    }
}
fn wrapper(version: &str, payload: &str, hash: &str) -> String {
    format!(
        r#"#!/usr/bin/env sh
set -eu
export NEXODE_RUNTIME_RELEASE_SEMVER="{version}"
export NEXODE_RUNTIME_RELEASE_GIT_HASH="{hash}"
export NEXODE_RUNTIME_RELEASE_GIT_DATE="rehearsal"
export NEXODE_RUNTIME_RELEASE_GIT_TAG="v{version}"
self=$0
while [ -L "$self" ]; do
    link=$(readlink -- "$self")
    case $link in
        /*) self=$link ;;
        *) self=$(dirname -- "$self")/$link ;;
    esac
done
self_dir=$(CDPATH= cd -- "$(dirname -- "$self")" && pwd)
exec "$self_dir/{payload}" "$@"
"#
    )
}
fn executable(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}
fn digest(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hash = Sha256::new();
    let mut buf = vec![0; 4 * 1024 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
fn package(
    payload: &Path,
    destination: &Path,
    version: &str,
    stem: &str,
    hash: &str,
) -> Result<()> {
    let file = File::create(destination)?;
    let gzip = flate2::write::GzEncoder::new(file, flate2::Compression::fast());
    let mut tar = tar::Builder::new(gzip);
    let script = wrapper(version, &format!("{stem}.bin"), hash);
    let mut header = tar::Header::new_gnu();
    header.set_size(script.len() as u64);
    header.set_mode(0o755);
    header.set_cksum();
    tar.append_data(&mut header, stem, script.as_bytes())?;
    tar.append_path_with_name(payload, format!("{stem}.bin"))?;
    tar.into_inner()?.finish()?;
    Ok(())
}
fn prepare_root(root: &Path) -> Result<PathBuf> {
    // Repeated runs may reset only a directory this tool owns. Never erase an
    // arbitrary --root/home simply because it has the same layout.
    fs::create_dir_all(root)?;
    let root = root.canonicalize()?;
    let marker = root.join(".orbit-update-rehearsal");
    if marker.exists() {
        ensure!(
            !marker.is_symlink() && fs::read_to_string(&marker)? == "orbit-update-rehearsal-v1\n",
            "invalid sandbox marker"
        );
    } else {
        ensure!(
            fs::read_dir(&root)?.next().is_none(),
            "refusing to reset nonempty unmarked sandbox {}",
            root.display()
        );
        fs::write(&marker, "orbit-update-rehearsal-v1\n")?;
    }
    for name in ["home", "run", "workspace", "cache"] {
        ensure!(
            !root.join(name).is_symlink(),
            "sandbox directory {name} must not be a symlink"
        );
    }
    cleanup(&root.join("home"))?;
    for name in ["home", "run", "workspace"] {
        let p = root.join(name);
        if p.exists() {
            fs::remove_dir_all(&p)?;
        }
        fs::create_dir(&p)?;
    }
    fs::create_dir_all(root.join("cache"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(root.join("run"), fs::Permissions::from_mode(0o700))?;
    }
    let log = root.join("server.log");
    if log.exists() || log.is_symlink() {
        fs::remove_file(log)?;
    }
    Ok(root)
}
#[cfg(target_os = "linux")]
fn cleanup(home: &Path) -> Result<()> {
    let needle = format!("NEXODE_HOME={}", home.display());
    let mut pids = Vec::new();
    for entry in fs::read_dir("/proc")? {
        let entry = entry?;
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<i32>() else {
            continue;
        };
        if pid == std::process::id() as i32 {
            continue;
        }
        let Ok(env) = fs::read(entry.path().join("environ")) else {
            continue;
        };
        if env
            .split(|b| *b == 0)
            .any(|value| value == needle.as_bytes())
        {
            pids.push(pid);
        }
    }
    for signal in [libc::SIGTERM, libc::SIGKILL] {
        for &pid in &pids {
            // Re-check ownership before each signal to avoid PID reuse.
            if fs::read(format!("/proc/{pid}/environ"))
                .is_ok_and(|env| env.split(|b| *b == 0).any(|v| v == needle.as_bytes()))
            {
                unsafe {
                    libc::kill(pid, signal);
                }
            }
        }
        let until = Instant::now() + Duration::from_secs(3);
        while Instant::now() < until
            && pids
                .iter()
                .any(|p| Path::new(&format!("/proc/{p}")).exists())
        {
            std::thread::sleep(Duration::from_millis(50));
        }
        pids.retain(|p| Path::new(&format!("/proc/{p}")).exists());
        if pids.is_empty() {
            break;
        }
    }
    Ok(())
}
#[cfg(not(target_os = "linux"))]
fn cleanup(_home: &Path) -> Result<()> {
    eprintln!("Process cleanup relies on /proc; check sandbox processes on this platform");
    Ok(())
}
struct Cleanup {
    home: PathBuf,
    keep: bool,
}
impl Drop for Cleanup {
    fn drop(&mut self) {
        if !self.keep
            && let Err(e) = cleanup(&self.home)
        {
            eprintln!("sandbox cleanup: {e:#}");
        }
    }
}
#[cfg(unix)]
fn install(home: &Path, payload: &Path, old: &str, stem: &str, hash: &str) -> Result<PathBuf> {
    use std::os::unix::fs::symlink;
    let builds = home.join("builds");
    let dir = builds.join("versions").join(old);
    fs::create_dir_all(&dir)?;
    let binary = dir.join(format!("{stem}.bin"));
    if fs::hard_link(payload, &binary).is_err() {
        fs::copy(payload, &binary)?;
    }
    let script = dir.join("nexode");
    fs::write(&script, wrapper(old, &format!("{stem}.bin"), hash))?;
    executable(&script)?;
    for channel in ["stable", "current", "shared-server"] {
        fs::create_dir(builds.join(channel))?;
        symlink(&script, builds.join(channel).join("nexode"))?;
        fs::write(builds.join(format!("{channel}-version")), old)?;
    }
    fs::create_dir(home.join("bin"))?;
    let launcher = home.join("bin/nexode");
    symlink(builds.join("current/nexode"), &launcher)?;
    Ok(launcher)
}
#[cfg(not(unix))]
fn install(_home: &Path, _payload: &Path, _old: &str, _stem: &str, _hash: &str) -> Result<PathBuf> {
    anyhow::bail!("rehearsal requires Unix")
}
fn command(launcher: &Path, root: &Path, url: &str) -> Command {
    let mut c = Command::new(launcher);
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("NEXODE_") || key == "XDG_RUNTIME_DIR" {
            c.env_remove(key);
        }
    }
    c.current_dir(root.join("workspace"))
        .env("NEXODE_HOME", root.join("home"))
        .env("NEXODE_RUNTIME_DIR", root.join("run"))
        .env("NEXODE_INSTALL_DIR", root.join("home/bin"))
        .env("NEXODE_UPDATE_RELEASE_URL", url)
        .env("NEXODE_NO_TELEMETRY", "1");
    let mut paths = vec![root.join("home/bin")];
    if let Some(p) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&p));
    }
    if let Ok(p) = std::env::join_paths(paths) {
        c.env("PATH", p);
    }
    c
}
fn timed_output(mut command: Command, seconds: u64, log: &Path) -> Result<String> {
    let stdout = File::create(log)?;
    let stderr = stdout.try_clone()?;
    let mut child = command
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(stderr)
        .spawn()?;
    let deadline = Instant::now() + Duration::from_secs(seconds);
    loop {
        if child.try_wait()?.is_some() {
            break;
        }
        if Instant::now() >= deadline {
            child.kill()?;
            child.wait()?;
            anyhow::bail!("rehearsal command timed out after {seconds}s")
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Ok(fs::read_to_string(log)?)
}
pub fn run(repo: &Path, args: impl Iterator<Item = String>) -> Result<()> {
    let args = Args::parse_from(std::iter::once("update-rehearsal".into()).chain(args));
    for n in [args.rate, args.drop_stall, args.check_delay] {
        ensure!(
            n.is_finite() && n >= 0.0,
            "rates and delays must be finite and nonnegative"
        );
    }
    ensure!(
        args.drop_at
            .is_none_or(|n| n.is_finite() && (0.0..=100.0).contains(&n)),
        "drop percentage must be 0..100"
    );
    let stem = stem()?;
    let binary = if let Some(binary) = args.binary {
        binary
    } else {
        let mut candidates: Vec<_> = ["selfdev", "release"]
            .into_iter()
            .map(|p| repo.join("target").join(p).join("nexode"))
            .filter(|p| p.is_file())
            .collect();
        candidates.sort_by_key(|p| p.metadata().and_then(|m| m.modified()).ok());
        candidates
            .pop()
            .context("build nexode first or supply --binary")?
    }
    .canonicalize()?;
    let manifest: toml::Value = fs::read_to_string(repo.join("Cargo.toml"))?.parse()?;
    let old = args.from_version.unwrap_or_else(|| {
        manifest["package"]["version"]
            .as_str()
            .unwrap_or("0.1.0")
            .to_owned()
    });
    let v = version(&old)?;
    let new = match args.to_version {
        Some(v) => v,
        None if args.up_to_date => old.clone(),
        None => format!(
            "{}.{}.{}",
            v[0],
            v[1],
            v[2].checked_add(1).context("version overflow")?
        ),
    };
    version(&new)?;
    let default_root = dirs::cache_dir()
        .context("cache directory unavailable")?
        .join("nexode-update-rehearsal-rust");
    let requested_root = args.root.unwrap_or(default_root);
    let absolute_root = std::path::absolute(&requested_root)?;
    let checked_root = if absolute_root.exists() {
        absolute_root.canonicalize()?
    } else {
        absolute_root
    };
    ensure!(
        !binary.starts_with(&checked_root),
        "binary must be outside sandbox"
    );
    let root = prepare_root(&requested_root)?;
    let home = root.join("home");
    let _cleanup = Cleanup {
        home: home.clone(),
        keep: args.keep || args.prepare_only,
    };
    ensure!(
        !binary.starts_with(&home) && !binary.starts_with(root.join("cache")),
        "binary must be outside sandbox"
    );
    let payload = root.join("cache/payload.bin");
    if payload.exists() {
        fs::remove_file(&payload)?;
    }
    fs::copy(&binary, &payload)?;
    executable(&payload)?;
    if !args.no_strip {
        match Command::new("strip")
            .arg("--strip-unneeded")
            .arg(&payload)
            .status()
        {
            Ok(s) if !s.success() => eprintln!("strip failed; using unstripped payload"),
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
            _ => {}
        }
    }
    let hash = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .current_dir(repo)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .filter(|s| s.chars().all(|c| c.is_ascii_hexdigit()))
        .unwrap_or_else(|| "rehearsal".into());
    let tarball = root.join("cache/release.tar.gz");
    package(&payload, &tarball, &new, stem, &hash)?;
    let checksum = digest(&tarball)?;
    let launcher = install(&home, &payload, &old, stem, &hash)?;
    fs::write(
        home.join("config.toml"),
        format!(
            "[features]\ncheck_updates = {}\nupdate_channel = \"stable\"\n",
            !args.manual
        ),
    )?;
    fs::write(home.join("setup_hints.json"),json!({"launch_count":100,"hotkey_dismissed":true,"alacritty_dismissed":true,"startup_spawn_hint_dismissed":true,"mac_ghostty_dismissed":true}).to_string())?;
    let server = server::Server::start(server::Scenario {
        tarball,
        checksum,
        stem: stem.into(),
        version: new.clone(),
        rate: args.rate,
        drop_at: args.drop_at,
        drop_stall: args.drop_stall,
        check_delay: args.check_delay,
        bad_checksum: args.bad_checksum,
        log: root.join("server.log"),
    })?;
    let url = format!("{}/release.json", server.url);
    println!(
        "Sandbox: {}\nInstalled v{old}; local release v{new}: {url}",
        root.display()
    );
    if args.prepare_only {
        println!("Prepared {} (server stops on exit)", launcher.display());
        return Ok(());
    }
    if args.headless_check {
        let mut before = command(&launcher, &root, &url);
        before.arg("--version");
        println!(
            "Before: {}",
            timed_output(before, 60, &root.join("before.log"))?.trim()
        );
        let mut update = command(&launcher, &root, &url);
        update.args(["update", "--no-selfdev"]);
        println!("{}", timed_output(update, 600, &root.join("update.log"))?);
        let mut after = command(&launcher, &root, &url);
        after.arg("--version");
        let after = timed_output(after, 60, &root.join("after.log"))?;
        let current = fs::read_to_string(home.join("builds/current-version"))?;
        let expected = if args.bad_checksum || args.up_to_date {
            &old
        } else {
            &new
        };
        ensure!(
            after.contains(&format!("v{expected}")) && current.trim() == expected,
            "update rehearsal failed: expected {expected}, current={current}, version={after}"
        );
        println!("PASS");
    } else {
        let status = command(&launcher, &root, &url)
            .arg("--no-selfdev")
            .args(args.nexode_args)
            .status()?;
        ensure!(status.success(), "sandbox exited with {status}");
    }
    for c in ["stable", "current", "shared-server"] {
        println!(
            "{c}: {}",
            fs::read_to_string(home.join("builds").join(format!("{c}-version")))?.trim()
        );
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn refuses_unowned_or_symlinked_sandboxes() {
        let t = tempfile::tempdir().unwrap();
        fs::write(t.path().join("important"), "keep").unwrap();
        assert!(prepare_root(t.path()).is_err());
        assert_eq!(
            fs::read_to_string(t.path().join("important")).unwrap(),
            "keep"
        );
        let root = t.path().join("sandbox");
        prepare_root(&root).unwrap();
        fs::write(root.join("home/stale"), "old").unwrap();
        prepare_root(&root).unwrap();
        assert!(!root.join("home/stale").exists());
        #[cfg(unix)]
        {
            fs::remove_dir(root.join("home")).unwrap();
            std::os::unix::fs::symlink(t.path(), root.join("home")).unwrap();
            assert!(prepare_root(&root).is_err());
        }
    }
    #[test]
    fn versions_and_archive_members() {
        for v in ["1.2", "../1.2.3", "1.2.3;echo nope", "1.2.3.4"] {
            assert!(version(v).is_err());
        }
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("payload");
        fs::write(&p, b"native-binary").unwrap();
        let dst = t.path().join("release.tar.gz");
        package(&p, &dst, "1.2.3", "nexode-linux-x86_64", "abc123").unwrap();
        let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(File::open(&dst).unwrap()));
        let names: Vec<_> = tar
            .entries()
            .unwrap()
            .map(|e| e.unwrap().path().unwrap().into_owned())
            .collect();
        assert_eq!(
            names,
            vec![
                PathBuf::from("nexode-linux-x86_64"),
                PathBuf::from("nexode-linux-x86_64.bin")
            ]
        );
        assert_eq!(digest(&dst).unwrap().len(), 64);
    }
}
