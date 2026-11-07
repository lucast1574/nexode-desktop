//! Native implementations of the Desktop offline capture and read-only preview protocol.
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
#[cfg(unix)]
pub(super) async fn inspect(pid: u32) -> Result<String> {
    let directory = std::env::var_os("XDG_RUNTIME_DIR")
        .map(|p| PathBuf::from(p).join("nexode-desktop-preview"))
        .unwrap_or_else(|| {
            std::env::temp_dir().join(format!("nexode-desktop-preview-{}", unsafe {
                libc::geteuid()
            }))
        });
    inspect_at(&directory, pid).await
}
#[cfg(unix)]
async fn inspect_at(directory: &Path, pid: u32) -> Result<String> {
    use std::os::unix::fs::{FileTypeExt, MetadataExt};
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};
    ensure!(pid > 0, "Desktop PID must be positive");
    let private = |p: &Path, socket: bool| -> Result<()> {
        let meta = std::fs::symlink_metadata(p)?;
        ensure!(
            if socket {
                meta.file_type().is_socket()
            } else {
                meta.file_type().is_dir()
            },
            "unexpected preview endpoint type"
        );
        ensure!(
            meta.uid() == unsafe { libc::getuid() } && meta.mode() & 0o077 == 0,
            "preview state must be private and owned by this user"
        );
        Ok(())
    };
    private(directory, false)?;
    let endpoint = directory.join(format!("{pid}.sock"));
    private(&endpoint, true)?;
    let value: Value = tokio::time::timeout(Duration::from_secs(3), async {
        let mut stream = tokio::net::UnixStream::connect(endpoint).await?;
        stream.write_all(b"{\"command\":\"list\"}\n").await?;
        let mut line = Vec::new();
        let mut reader = tokio::io::BufReader::new(stream.take(65537));
        reader.read_until(b'\n', &mut line).await?;
        ensure!(
            line.len() <= 65536 && line.last() == Some(&b'\n'),
            "preview response too large or incomplete"
        );
        Ok::<Value, anyhow::Error>(serde_json::from_slice(&line)?)
    })
    .await
    .context("Desktop preview inspection timed out")??;
    ensure!(
        value["ok"] == true,
        "Desktop preview catalog rejected the request: {}",
        value.get("error").unwrap_or(&Value::Null)
    );
    Ok(serde_json::to_string_pretty(&value)?)
}
#[cfg(not(unix))]
pub(super) async fn inspect(_pid: u32) -> Result<String> {
    anyhow::bail!("Desktop preview inspection requires Unix sockets")
}
#[cfg(target_os = "linux")]
struct Programs {
    cargo: PathBuf,
    xvfb: PathBuf,
    wm: PathBuf,
    keyboard: PathBuf,
    capture: PathBuf,
    driver: PathBuf,
}
#[cfg(target_os = "linux")]
impl Programs {
    fn installed() -> Result<Self> {
        let driver = std::fs::read_dir("/usr/share/vulkan/icd.d")?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .find(|p| {
                p.file_name().is_some_and(|n| {
                    n.to_string_lossy().starts_with("lvp_icd")
                        && n.to_string_lossy().ends_with(".json")
                })
            })
            .context("Mesa lavapipe is required for offline Desktop captures")?;
        Ok(Self {
            cargo: "cargo".into(),
            xvfb: "Xvfb".into(),
            wm: "openbox".into(),
            keyboard: "xdotool".into(),
            capture: "import".into(),
            driver,
        })
    }
}
#[cfg(target_os = "linux")]
fn isolated_env(root: &Path) -> Result<BTreeMap<String, String>> {
    use std::os::unix::fs::PermissionsExt;
    let mut env = BTreeMap::new();
    for (key, dir) in [
        ("HOME", "home"),
        ("XDG_RUNTIME_DIR", "runtime"),
        ("XDG_CONFIG_HOME", "config"),
        ("XDG_CACHE_HOME", "cache"),
        ("XDG_DATA_HOME", "data"),
        ("XDG_STATE_HOME", "logs"),
        ("NEXODE_HOME", "nexode"),
    ] {
        let p = root.join(dir);
        std::fs::create_dir_all(&p)?;
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o700))?;
        env.insert(key.into(), p.to_string_lossy().into_owned());
    }
    for (key, value) in [
        ("PATH", "/usr/bin:/bin"),
        ("NEXODE_DESKTOP_SCREENSHOT", "1"),
        ("LIBGL_ALWAYS_SOFTWARE", "1"),
        ("LANG", "C.UTF-8"),
        ("NEXODE_DESKTOP_SCREENSHOT_TRANSCRIPT", "all"),
        ("NEXODE_DESKTOP_SCREENSHOT_PANELS", "1"),
    ] {
        env.insert(key.into(), value.into());
    }
    env.insert(
        "NEXODE_DESKTOP_STATE".into(),
        root.join("state").to_string_lossy().into_owned(),
    );
    env.insert(
        "DBUS_SESSION_BUS_ADDRESS".into(),
        format!("unix:path={}", root.join("no-dbus").display()),
    );
    Ok(env)
}
#[cfg(target_os = "linux")]
struct OwnedChild {
    child: tokio::process::Child,
    _group: super::ProcessGroup,
}
#[cfg(target_os = "linux")]
fn spawn(mut command: tokio::process::Command) -> Result<OwnedChild> {
    use std::os::unix::process::CommandExt;
    command.kill_on_drop(true).stdin(Stdio::null());
    command.as_std_mut().process_group(0);
    let child = command.spawn()?;
    let group = super::ProcessGroup(child.id().context("child PID")?);
    Ok(OwnedChild {
        child,
        _group: group,
    })
}
#[cfg(target_os = "linux")]
async fn checked(command: tokio::process::Command) -> Result<()> {
    let mut process = spawn(command)?;
    ensure!(
        process.child.wait().await?.success(),
        "Desktop capture subprocess failed"
    );
    Ok(())
}
#[cfg(target_os = "linux")]
async fn capture(root: &Path, output: &Path, programs: Programs) -> Result<String> {
    use tokio::io::AsyncBufReadExt;
    super::reject_symlink_components(output)?;
    ensure!(
        !output.exists(),
        "refusing to overwrite {}",
        output.display()
    );
    let parent = output.parent().context("capture output parent")?;
    std::fs::create_dir_all(parent)?;
    let mut build = tokio::process::Command::new(programs.cargo);
    build
        .args([
            "build",
            "--locked",
            "-p",
            "nexode-desktop",
            "-p",
            "nexode-desktop-ui",
        ])
        .env_remove("RUSTC_WORKSPACE_WRAPPER")
        .current_dir(root)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    checked(build)
        .await
        .context("fresh Desktop host/UI build failed")?;
    let binary = root
        .join("target/debug/nexode-desktop")
        .canonicalize()
        .context("built Desktop binary missing")?;
    let scratch = tempfile::Builder::new()
        .prefix("desktop-capture-")
        .tempdir_in(root.join("target"))?;
    let dir = scratch.path();
    let mut env = isolated_env(dir)?;
    let config = dir.join("desktop.toml");
    std::fs::write(
        &config,
        "[appearance]\nlayout_mode = \"folder_tabs\"\ntheme = \"warm-neutral\"\n",
    )?;
    env.insert(
        "NEXODE_DESKTOP_CONFIG".into(),
        config.to_string_lossy().into_owned(),
    );
    env.insert(
        "VK_DRIVER_FILES".into(),
        programs.driver.to_string_lossy().into_owned(),
    );
    let wm_config = dir.join("openbox.xml");
    std::fs::write(
        &wm_config,
        r#"<openbox_config xmlns="http://openbox.org/3.4/rc"><applications><application class="*"><decor>no</decor><maximized>yes</maximized></application></applications></openbox_config>"#,
    )?;
    let command = |program: &Path, env: &BTreeMap<String, String>| {
        let mut c = tokio::process::Command::new(program);
        c.env_clear()
            .envs(env)
            .current_dir(dir)
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        c
    };
    let mut xvfb_command = command(&programs.xvfb, &env);
    xvfb_command
        .args([
            "-displayfd",
            "1",
            "-screen",
            "0",
            "1440x1000x24",
            "-nolisten",
            "tcp",
        ])
        .stdout(Stdio::piped());
    let mut xvfb = spawn(xvfb_command)?;
    let mut display = String::new();
    tokio::time::timeout(
        Duration::from_secs(15),
        tokio::io::BufReader::new(xvfb.child.stdout.take().context("Xvfb display pipe")?)
            .read_line(&mut display),
    )
    .await
    .context("Xvfb did not become ready")??;
    let display = display.trim();
    ensure!(
        !display.is_empty() && display.len() < 10 && display.bytes().all(|b| b.is_ascii_digit()),
        "Xvfb failed to allocate a private display"
    );
    env.insert("DISPLAY".into(), format!(":{display}"));
    let mut wm_command = command(&programs.wm, &env);
    wm_command
        .args(["--sm-disable", "--config-file"])
        .arg(wm_config);
    let mut wm = spawn(wm_command)?;
    tokio::time::sleep(Duration::from_millis(500)).await;
    ensure!(
        wm.child.try_wait()?.is_none(),
        "private Openbox failed to start"
    );
    let mut app = spawn(command(&binary, &env))?;
    let state = dir.join("state");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(45);
    loop {
        if std::fs::read_to_string(&state).is_ok_and(|s| s.contains("widths=1.00")) {
            break;
        }
        ensure!(
            app.child.try_wait()?.is_none() && tokio::time::Instant::now() < deadline,
            "Desktop failed to render the offline fixture"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    tokio::time::sleep(Duration::from_secs(2)).await;
    let mut keyboard = command(&programs.keyboard, &env);
    keyboard.args(["key", "--clearmodifiers", "Escape"]);
    checked(keyboard).await?;
    tokio::time::sleep(Duration::from_millis(300)).await;
    ensure!(
        app.child.try_wait()?.is_none(),
        "Desktop exited before capture"
    );
    let staged = dir.join("capture.png");
    let mut grab = command(&programs.capture, &env);
    grab.args(["-window", "root"])
        .arg(format!("png:{}", staged.display()));
    checked(grab).await?;
    let bytes = std::fs::read(&staged)?;
    ensure!(
        bytes.starts_with(b"\x89PNG\r\n\x1a\n") && bytes.len() > 24,
        "capture did not produce a PNG"
    );
    // Recheck after rendering; never follow a newly introduced output symlink.
    super::reject_symlink_components(output)?;
    let mut target = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)?;
    std::io::Write::write_all(&mut target, &bytes)?;
    Ok(format!(
        "Screenshot: {}\nFixture state: {}",
        output.display(),
        std::fs::read_to_string(state)?.trim()
    ))
}
#[cfg(target_os = "linux")]
pub(super) async fn screenshot(root: &Path, output: &Path) -> Result<String> {
    capture(root, output, Programs::installed()?).await
}
#[cfg(not(target_os = "linux"))]
pub(super) async fn screenshot(_root: &Path, _output: &Path) -> Result<String> {
    anyhow::bail!("private Xvfb Desktop captures require Linux")
}

#[cfg(all(test, target_os = "linux"))]
use crate::native_fixture;
#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
    #[tokio::test]
    async fn inspection_reads_only_private_catalog_socket_and_bounds_replies() {
        let root = tempfile::tempdir().unwrap();
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let socket = root.path().join("42.sock");
        let listener = tokio::net::UnixListener::bind(&socket).unwrap();
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut line = String::new();
            tokio::io::BufReader::new(&mut stream)
                .read_line(&mut line)
                .await
                .unwrap();
            assert_eq!(
                serde_json::from_str::<Value>(&line).unwrap(),
                json!({"command":"list"})
            );
            stream
                .write_all(b"{\"ok\":true,\"states\":[\"empty\"]}\n")
                .await
                .unwrap();
        });
        assert!(inspect_at(root.path(), 42).await.unwrap().contains("empty"));
        server.await.unwrap();
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o666)).unwrap();
        assert!(
            inspect_at(root.path(), 42)
                .await
                .unwrap_err()
                .to_string()
                .contains("private")
        );
        std::fs::remove_file(&socket).unwrap();
        std::os::unix::fs::symlink(root.path().join("missing"), &socket).unwrap();
        assert!(inspect_at(root.path(), 42).await.is_err());
        std::fs::remove_file(&socket).unwrap();
        let listener = tokio::net::UnixListener::bind(&socket).unwrap();
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut line = String::new();
            tokio::io::BufReader::new(&mut stream)
                .read_line(&mut line)
                .await
                .unwrap();
            let _ = stream.write_all(&vec![b'x'; 65537]).await;
        });
        assert!(inspect_at(root.path(), 42).await.is_err());
        server.await.unwrap();
    }
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn offline_capture_owns_its_display_builds_and_preserves_existing_output() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("target/debug")).unwrap();
        let helper = native_fixture::binary();
        let alias = |name: &str| {
            let path = root.path().join(name);
            std::os::unix::fs::symlink(helper, &path).unwrap();
            path
        };
        std::fs::copy(helper, root.path().join("target/debug/nexode-desktop")).unwrap();
        let programs = Programs {
            cargo: alias("cargo"),
            xvfb: alias("Xvfb"),
            wm: alias("openbox"),
            keyboard: alias("xdotool"),
            capture: alias("import"),
            driver: root.path().join("fake-driver.json"),
        };
        std::fs::write(&programs.driver, "{}").unwrap();
        let output = root.path().join("target/capture.png");
        let message = capture(root.path(), &output, programs).await.unwrap();
        assert!(message.contains("widths=1.00"));
        assert!(std::fs::read(&output).unwrap().starts_with(b"\x89PNG"));
        let env = isolated_env(&root.path().join("isolated")).unwrap();
        assert!(!env.contains_key("NEXODE_TEST_SECRET"));
        assert!(!env.contains_key("WAYLAND_DISPLAY"));
        assert!(!env.contains_key("DISPLAY"));
        let inert = Programs {
            cargo: "missing".into(),
            xvfb: "missing".into(),
            wm: "missing".into(),
            keyboard: "missing".into(),
            capture: "missing".into(),
            driver: "missing".into(),
        };
        assert!(
            capture(root.path(), &output, inert)
                .await
                .unwrap_err()
                .to_string()
                .contains("overwrite")
        );
    }
}

pub(super) async fn run(
    root: &Path,
    spec: super::CommandSpec,
    timeout: u64,
) -> Result<super::ToolOutput> {
    let result = tokio::time::timeout(Duration::from_secs(timeout), async {
        match spec.program.as_str() {
            "native:desktop-screenshot" => {
                screenshot(
                    root,
                    Path::new(spec.args.first().context("screenshot output")?),
                )
                .await
            }
            "native:desktop-preview" => {
                inspect(spec.args.first().context("Desktop PID")?.parse()?).await
            }
            _ => bail!("Unknown native Desktop action"),
        }
    })
    .await
    .context("Native Desktop action timed out; private children terminated")?;
    let (success, stdout, stderr) = match result {
        Ok(text) => (true, text, String::new()),
        Err(error) => (false, String::new(), format!("{error:#}")),
    };
    Ok(super::ToolOutput::new(serde_json::to_string_pretty(
        &json!({"program":spec.program,"args":spec.args,"cwd":root,"note":spec.note,"success":success,"exit_code":if success{0}else{1},"stdout":stdout,"stderr":stderr}),
    )?))
}
