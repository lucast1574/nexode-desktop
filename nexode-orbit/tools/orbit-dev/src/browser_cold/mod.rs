//! Real Firefox acceptance is restricted to PID 1 in a private network namespace.
use anyhow::{Context, Result, ensure};
use clap::Parser;
use serde_json::{Value, json};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};
#[derive(Parser)]
struct Args {
    #[arg(long)]
    host_netns: String,
    #[arg(long)]
    output_dir: PathBuf,
    #[arg(long)]
    nexode: Option<PathBuf>,
    #[arg(long)]
    bridge_dir: Option<PathBuf>,
}
fn isolated(pid: u32, current: &str, host: &str) -> Result<()> {
    ensure!(
        cfg!(target_os = "linux") && pid == 1,
        "run as PID 1 in a private Linux PID namespace"
    );
    ensure!(
        !host.is_empty() && current != host,
        "a private network namespace is required"
    );
    Ok(())
}
fn prepare(root: &Path, source: &Path) -> Result<Value> {
    let home = root.join("home");
    let bridge = home.join(".nexode/browser");
    let profile = home.join(".mozilla/firefox/acceptance.default");
    let runtime = root.join("runtime");
    for p in [&bridge, &profile.join("extensions"), &runtime] {
        fs::create_dir_all(p)?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700))?;
    }
    for name in [
        "browser",
        "firefox-agent-bridge-host",
        "browser-agent-bridge.xpi",
    ] {
        fs::copy(source.join(name), bridge.join(name))?;
    }
    fs::write(
        bridge.join(".setup-complete"),
        "previous successful setup\n",
    )?;
    fs::write(bridge.join(".browser"), "firefox")?;
    let mut archive =
        zip::ZipArchive::new(fs::File::open(bridge.join("browser-agent-bridge.xpi"))?)?;
    let mut bytes = vec![];
    archive
        .by_name("manifest.json")?
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 1024 * 1024, "extension manifest too large");
    let manifest: Value = serde_json::from_slice(&bytes)?;
    let id = manifest["browser_specific_settings"]["gecko"]["id"]
        .as_str()
        .filter(|id| !id.is_empty() && !id.contains(['/', '\\', '\0']))
        .context("unsafe or missing extension ID")?;
    fs::copy(
        bridge.join("browser-agent-bridge.xpi"),
        profile.join("extensions").join(format!("{id}.xpi")),
    )?;
    fs::write(
        home.join(".mozilla/firefox/profiles.ini"),
        "[General]\nStartWithLastProfile=1\nVersion=2\n\n[Profile0]\nName=acceptance\nIsRelative=1\nPath=acceptance.default\nDefault=1\n",
    )?;
    fs::write(
        profile.join("user.js"),
        r#"user_pref("extensions.autoDisableScopes", 0);
user_pref("extensions.enabledScopes", 15);
user_pref("browser.shell.checkDefaultBrowser", false);
user_pref("browser.startup.homepage", "about:blank");
user_pref("browser.startup.page", 0);
user_pref("browser.startup.homepage_override.mstone", "ignore");
user_pref("datareporting.policy.dataSubmissionEnabled", false);
user_pref("toolkit.telemetry.reportingpolicy.firstRun", false);
user_pref("browser.aboutwelcome.enabled", false);
"#,
    )?;
    let hosts = home.join(".mozilla/native-messaging-hosts");
    fs::create_dir_all(&hosts)?;
    fs::write(
        hosts.join("firefox_agent_bridge.json"),
        serde_json::to_vec(
            &json!({"allowed_extensions":[id,"firefox-agent-bridge@local"],"description":"Isolated acceptance native host","name":"firefox_agent_bridge","path":bridge.join("firefox-agent-bridge-host"),"type":"stdio"}),
        )?,
    )?;
    Ok(manifest)
}
fn environment(root: &Path) -> Vec<(String, String)> {
    let home = root.join("home");
    let mut env = vec![];
    for (k, v) in [
        ("PATH", "/usr/bin:/bin"),
        ("LANG", "C.UTF-8"),
        ("MOZ_HEADLESS", "1"),
        ("MOZ_LEGACY_PROFILES", "1"),
        ("MOZ_DISABLE_CONTENT_SANDBOX", "1"),
        ("NEXODE_BROWSER", "firefox"),
        ("NEXODE_BROWSER_AUTO_UPDATE", "0"),
        ("NEXODE_NO_TELEMETRY", "1"),
    ] {
        env.push((k.into(), v.into()));
    }
    for (k, p) in [
        ("HOME", home.clone()),
        ("NEXODE_HOME", home.join(".nexode")),
        ("XDG_CONFIG_HOME", home.join(".config")),
        ("XDG_CACHE_HOME", home.join(".cache")),
        ("XDG_RUNTIME_DIR", root.join("runtime")),
    ] {
        env.push((k.into(), p.to_string_lossy().into_owned()));
    }
    env
}
fn passed(
    setup: &std::process::Output,
    status: &std::process::Output,
    tabs: &std::process::Output,
) -> bool {
    let setup_text = String::from_utf8_lossy(&setup.stdout);
    let status_text = String::from_utf8_lossy(&status.stdout);
    let data: Value = serde_json::from_slice(&tabs.stdout).unwrap_or(Value::Null);
    setup.status.success()
        && setup_text.contains("was launched and the browser bridge reconnected")
        && !setup_text.contains("extension install prompt")
        && status.status.success()
        && status_text.contains("Built-in browser tool is ready")
        && tabs.status.success()
        && data["totalTabs"] == 1
        && data["windows"][0]["tabs"][0]["url"] == "about:blank"
}
#[cfg(target_os = "linux")]
struct Zombie(i32);
#[cfg(target_os = "linux")]
extern "C" fn stop_namespace(signal: libc::c_int) {
    unsafe {
        libc::_exit(128 + signal);
    }
}
#[cfg(target_os = "linux")]
impl Zombie {
    fn create() -> Result<Self> {
        let pid = unsafe { libc::fork() };
        ensure!(pid >= 0, "fork failed");
        if pid == 0 {
            unsafe {
                libc::prctl(
                    libc::PR_SET_NAME,
                    c"firefox".as_ptr() as libc::c_ulong,
                    0 as libc::c_ulong,
                    0 as libc::c_ulong,
                    0 as libc::c_ulong,
                );
                libc::_exit(0);
            }
        }
        let child = Self(pid);
        for _ in 0..100 {
            let stat = fs::read_to_string(format!("/proc/{pid}/stat"))?;
            if stat
                .rsplit_once(')')
                .is_some_and(|(_, s)| s.split_whitespace().next() == Some("Z"))
            {
                return Ok(child);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        anyhow::bail!("failed to establish zombie precondition")
    }
}
#[cfg(target_os = "linux")]
impl Drop for Zombie {
    fn drop(&mut self) {
        unsafe {
            libc::waitpid(self.0, std::ptr::null_mut(), 0);
        }
    }
}
pub fn run(args: impl Iterator<Item = String>) -> Result<i32> {
    let a = Args::parse_from(std::iter::once("verify-browser-cold-start".into()).chain(args));
    let current =
        fs::read_link("/proc/self/ns/net").context("Linux network namespace unavailable")?;
    isolated(
        std::process::id(),
        &current.to_string_lossy(),
        &a.host_netns,
    )?;
    #[cfg(not(target_os = "linux"))]
    {
        let _ = a;
        anyhow::bail!("Linux required");
    }
    #[cfg(target_os = "linux")]
    {
        // Namespace PID 1 ignores default termination signals; explicitly exit so
        // the kernel also kills every browser descendant in this private namespace.
        for signal in [libc::SIGINT, libc::SIGTERM] {
            let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
            action.sa_sigaction = stop_namespace as *const () as usize;
            unsafe {
                libc::sigemptyset(&mut action.sa_mask);
            }
            ensure!(
                unsafe { libc::sigaction(signal, &action, std::ptr::null_mut()) } == 0,
                "cannot install namespace termination handler"
            );
        }
        let home = dirs::home_dir().context("home unavailable")?;
        let binary = a
            .nexode
            .unwrap_or(home.join(".nexode/builds/shared-server/nexode"))
            .canonicalize()?;
        let source = a.bridge_dir.unwrap_or(home.join(".nexode/browser"));
        fs::create_dir_all(&a.output_dir)?;
        let root = tempfile::Builder::new()
            .prefix("firefox-cold-")
            .tempdir_in(&a.output_dir)?
            .keep();
        let manifest = prepare(&root, &source)?;
        let env = environment(&root);
        let lo = crate::process::capture(
            Command::new("/usr/bin/ip")
                .args(["link", "set", "lo", "up"])
                .env_clear()
                .envs(env.iter().cloned()),
            Duration::from_secs(5),
        )?;
        ensure!(lo.status.success(), "failed to enable private loopback");
        let zombie = Zombie::create()?;
        println!(
            "{}",
            json!({"root":root,"binary":binary,"zombie_pid":zombie.0,"zombie_state":"Z","extension_version":manifest["version"]})
        );
        let execute = |exe: &Path, args: &[&str], secs| {
            crate::process::capture(
                Command::new(exe)
                    .args(args)
                    .env_clear()
                    .envs(env.iter().cloned()),
                Duration::from_secs(secs),
            )
        };
        let setup = execute(&binary, &["browser", "setup", "firefox"], 85)?;
        let status = execute(&binary, &["browser", "status", "firefox"], 25)?;
        let tabs = execute(
            &root.join("home/.nexode/browser/browser"),
            &["listTabs"],
            15,
        )?;
        for (label, result) in [("setup", &setup), ("status", &status), ("tabs", &tabs)] {
            fs::write(root.join(format!("{label}.stdout")), &result.stdout)?;
            fs::write(root.join(format!("{label}.stderr")), &result.stderr)?;
            print!("{}", String::from_utf8_lossy(&result.stdout));
            eprint!("{}", String::from_utf8_lossy(&result.stderr));
        }
        let pass = passed(&setup, &status, &tabs);
        let evidence = json!({"passed":pass,"setup_exit":setup.status.code(),"status_exit":status.status.code(),"tabs_exit":tabs.status.code(),"root":root,"binary":binary});
        fs::write(
            root.join("result.json"),
            serde_json::to_vec_pretty(&evidence)?,
        )?;
        println!("{evidence}");
        Ok(i32::from(!pass))
    }
}
#[cfg(test)]
mod tests;
