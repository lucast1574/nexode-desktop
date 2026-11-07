//! Offline real-SDK acceptance with private homes and owned daemons.
use anyhow::{Context, Result, ensure};
use clap::Parser;
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs,
    os::unix::{
        fs::{PermissionsExt, symlink},
        net::UnixStream,
        process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

#[derive(Parser)]
struct Args {
    #[arg(long)]
    nexode_binary: Option<PathBuf>,
    #[arg(long)]
    bridge_binary: Option<PathBuf>,
    #[arg(long)]
    desktop_repo: Option<PathBuf>,
    #[arg(long)]
    output_dir: Option<PathBuf>,
    #[arg(long)]
    compile_only: bool,
}
struct Owned(Child);
impl Drop for Owned {
    fn drop(&mut self) {
        unsafe {
            libc::kill(-(self.0.id() as i32), libc::SIGTERM);
        }
        let end = Instant::now() + Duration::from_secs(2);
        while Instant::now() < end {
            if self.0.try_wait().ok().flatten().is_some() {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        unsafe {
            libc::kill(-(self.0.id() as i32), libc::SIGKILL);
        }
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn wait_socket(path: &Path, child: &mut Owned) -> Result<()> {
    let end = Instant::now() + Duration::from_secs(30);
    loop {
        ensure!(
            crate::ci_suites::unix::interrupted().is_none(),
            "acceptance interrupted"
        );
        ensure!(
            child.0.try_wait()?.is_none(),
            "private process exited; see retained logs"
        );
        if UnixStream::connect(path).is_ok() {
            return Ok(());
        }
        ensure!(
            Instant::now() < end,
            "private socket readiness timeout: {}",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}
fn private_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}
fn outside(path: &Path, repo: &Path, desktop: &Path) -> Result<()> {
    ensure!(
        !path.starts_with(repo) && !path.starts_with(desktop),
        "artifacts must be outside both repositories"
    );
    Ok(())
}
fn environment(root: &Path) -> BTreeMap<String, String> {
    let mut env = BTreeMap::from([
        (
            "PATH".into(),
            std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".into()),
        ),
        ("LANG".into(), "C.UTF-8".into()),
        (
            "NEXODE_SERVER_OWNER_PID".into(),
            std::process::id().to_string(),
        ),
    ]);
    for (k, v) in [
        ("HOME", "home"),
        ("XDG_RUNTIME_DIR", "runtime"),
        ("XDG_CONFIG_HOME", "config"),
        ("XDG_CACHE_HOME", "cache"),
        ("XDG_DATA_HOME", "data"),
        ("XDG_STATE_HOME", "state"),
        ("TMPDIR", "tmp"),
        ("NEXODE_HOME", "nexode"),
        ("NEXODE_RUNTIME_DIR", "runtime"),
        ("NEXODE_SOCKET", "runtime/daemon.sock"),
        ("NEXODE_API_SOCKET", "runtime/api.sock"),
    ] {
        env.insert(k.into(), root.join(v).to_string_lossy().into_owned());
    }
    for (k, v) in [
        ("NEXODE_DEBUG_CONTROL", "1"),
        ("NEXODE_NO_TELEMETRY", "1"),
        ("NEXODE_TEMP_SERVER", "1"),
        ("NEXODE_TEMP_SERVER_IDLE_SECS", "300"),
    ] {
        env.insert(k.into(), v.into());
    }
    env
}
fn launch(
    root: &Path,
    env: &BTreeMap<String, String>,
    name: &str,
    command: &mut Command,
) -> Result<Owned> {
    let log = fs::File::create(root.join(format!("{name}.log")))?;
    Ok(Owned(
        command
            .current_dir(root)
            .env_clear()
            .envs(env)
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log)
            .process_group(0)
            .spawn()?,
    ))
}
pub fn run(repo: &Path, args: impl Iterator<Item = String>) -> Result<()> {
    let a = Args::parse_from(std::iter::once("test-desktop-selfdev".into()).chain(args));
    let _signals = crate::ci_suites::unix::Signals::install()?;
    let repo = repo.canonicalize()?;
    let binary = a
        .nexode_binary
        .unwrap_or_else(|| repo.join("target/selfdev/nexode"))
        .canonicalize()?;
    let bridge = a
        .bridge_binary
        .unwrap_or_else(|| repo.join("target/selfdev/nexode-harness-api-bridge"))
        .canonicalize()?;
    let desktop = a
        .desktop_repo
        .unwrap_or_else(|| repo.parent().unwrap().join("nexode-desktop"))
        .canonicalize()?;
    let nested = desktop.join("crates/nexode-desktop-ui/src");
    ensure!(nested.is_dir(), "a Desktop source checkout is required");
    let root = if let Some(path) = a.output_dir {
        // Resolve the parent first so a symlink cannot bypass the checkout guard.
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        fs::create_dir_all(parent)?;
        let path = parent.canonicalize()?.join(
            path.file_name()
                .context("artifact directory name required")?,
        );
        outside(&path, &repo, &desktop)?;
        fs::create_dir(&path)?;
        path
    } else {
        let scratch = std::env::var_os("NEXODE_SCRATCH_DIR")
            .map(PathBuf::from)
            .unwrap_or(
                dirs::home_dir()
                    .context("home unavailable")?
                    .join(".cache/nexode-acceptance"),
            );
        fs::create_dir_all(&scratch)?;
        let scratch = scratch.canonicalize()?;
        outside(&scratch, &repo, &desktop)?;
        tempfile::Builder::new()
            .prefix("desktop-selfdev-")
            .tempdir_in(scratch)?
            .keep()
    };
    private_dir(&root)?;
    println!("Artifacts: {}", root.display());
    ensure!(
        root.join("runtime/daemon-debug.sock")
            .as_os_str()
            .as_encoded_bytes()
            .len()
            < 104,
        "artifact path too long for Unix sockets"
    );
    for name in [
        "home",
        "runtime",
        "config",
        "cache",
        "data",
        "state",
        "nexode",
        "tmp",
        "sdk/src",
        "regular/nexode-desktop",
    ] {
        private_dir(&root.join(name))?;
    }
    symlink(&nested, root.join("desktop-link"))?;
    fs::write(
        root.join("nexode/config.toml"),
        "[features]\nmemory = false\n[telemetry]\nenabled = false\n",
    )?;
    let sdk = toml::Value::String(
        repo.join("crates/nexode-sdk")
            .to_string_lossy()
            .into_owned(),
    )
    .to_string();
    fs::write(
        root.join("sdk/Cargo.toml"),
        format!(
            "[package]\nname = \"desktop-selfdev-sdk-acceptance\"\nversion = \"0.0.0\"\nedition = \"2024\"\n[workspace]\n[dependencies]\nserde_json = \"1\"\nnexode-sdk = {{ path = {sdk} }}\n"
        ),
    )?;
    fs::write(root.join("sdk/src/main.rs"), include_str!("sdk.rs"))?;
    let out = crate::process::capture_interruptible(
        Command::new("cargo")
            .args(["build", "--offline", "--manifest-path"])
            .arg(root.join("sdk/Cargo.toml"))
            .arg("--target-dir")
            .arg(root.join("sdk-target"))
            .current_dir(&repo),
        Duration::from_secs(600),
        || crate::ci_suites::unix::interrupted().is_some(),
    )?;
    fs::write(
        root.join("sdk-build.log"),
        [out.stdout, out.stderr].concat(),
    )?;
    ensure!(out.status.success(), "SDK build failed; see sdk-build.log");
    if a.compile_only {
        println!("PASS: real Rust SDK acceptance client compiled. No runtime was started.");
        return Ok(());
    }
    let env = environment(&root);
    let mut daemon = launch(
        &root,
        &env,
        "daemon",
        Command::new(binary).args([
            "--no-update",
            "--no-selfdev",
            "--provider",
            "nexode",
            "serve",
        ]),
    )?;
    wait_socket(Path::new(&env["NEXODE_SOCKET"]), &mut daemon)?;
    let debug = root.join("runtime/daemon-debug.sock");
    wait_socket(&debug, &mut daemon)?;
    let mut adapter = launch(
        &root,
        &env,
        "bridge",
        Command::new(bridge)
            .arg(&env["NEXODE_API_SOCKET"])
            .arg(&env["NEXODE_SOCKET"]),
    )?;
    wait_socket(Path::new(&env["NEXODE_API_SOCKET"]), &mut adapter)?;
    let out = crate::process::capture_interruptible(
        Command::new(root.join("sdk-target/debug/desktop-selfdev-sdk-acceptance"))
            .arg(&env["NEXODE_API_SOCKET"])
            .args([
                &debug,
                &desktop,
                &nested,
                &root.join("desktop-link"),
                &root.join("regular/nexode-desktop"),
                &repo,
            ])
            .current_dir(&root)
            .env_clear()
            .envs(&env),
        Duration::from_secs(180),
        || crate::ci_suites::unix::interrupted().is_some(),
    )?;
    fs::write(root.join("acceptance.stdout"), &out.stdout)?;
    fs::write(root.join("acceptance.stderr"), &out.stderr)?;
    ensure!(
        out.status.success(),
        "SDK acceptance failed; see acceptance.stderr"
    );
    let text = String::from_utf8(out.stdout)?;
    let evidence: Vec<Value> = text
        .lines()
        .filter(|s| s.starts_with('{'))
        .map(serde_json::from_str)
        .collect::<Result<_, _>>()?;
    ensure!(
        evidence.len() == 5,
        "expected evidence for all five SDK paths"
    );
    fs::write(
        root.join("acceptance.json"),
        serde_json::to_string_pretty(&evidence)?,
    )?;
    print!("{text}");
    println!(
        "PASS: SDK create/reattach, provider context, Desktop status/test and mode denials. No inference."
    );
    Ok(())
}
