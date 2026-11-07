//! Offline catalog + real Nexode debug session; no model turns are submitted.
use anyhow::{Context, Result, ensure};
use clap::Parser;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
const SETUP: &str = "npx -y demo-cards-mcp@2.1.0 && export DEMO_CARDS_KEY";
fn catalog(selected: Option<&str>) -> Value {
    let tools = json!([{"name":"demo-cards","blurb":"single-use virtual cards for agent purchases","url":"https://demo-cards.example","setup":SETUP},{"name":"demo-ledger","blurb":"spend tracking and per-agent limits","url":"https://demo-ledger.example","setup":"npx -y demo-ledger-mcp@1.0.0"}]);
    if let Some(selected) = selected {
        let found = tools
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["name"] == selected);
        let mut out =
            json!({"category":"payments","selected_tool":selected,"listed":found.is_some()});
        if let Some(tool) = found {
            out["tool"] = tool.clone();
        }
        out
    } else {
        json!({"tools":tools})
    }
}
struct CatalogServer {
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
    endpoint: String,
}
impl CatalogServer {
    fn new() -> Result<Self> {
        let server = tiny_http::Server::http("127.0.0.1:0")
            .map_err(|e| anyhow::anyhow!("catalog bind: {e}"))?;
        let endpoint = format!("http://{}/discovery", server.server_addr());
        let stop = Arc::new(AtomicBool::new(false));
        let signal = stop.clone();
        let worker = std::thread::spawn(move || {
            while !signal.load(Ordering::Relaxed) {
                if let Ok(Some(req)) = server.recv_timeout(Duration::from_millis(100)) {
                    let selected = url::Url::parse(&format!("http://localhost{}", req.url()))
                        .ok()
                        .and_then(|u| {
                            u.query_pairs()
                                .find(|(k, _)| k == "tool")
                                .map(|(_, v)| v.into_owned())
                        });
                    let response =
                        tiny_http::Response::from_string(catalog(selected.as_deref()).to_string())
                            .with_header(
                                tiny_http::Header::from_bytes("Content-Type", "application/json")
                                    .unwrap(),
                            );
                    let _ = req.respond(response);
                }
            }
        });
        Ok(Self {
            stop,
            worker: Some(worker),
            endpoint,
        })
    }
}
impl Drop for CatalogServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
fn validate(browse: &str, selected: &str, off: &str) -> Result<()> {
    ensure!(
        browse.contains("demo-cards"),
        "browse did not list catalog entries"
    );
    ensure!(
        !browse.contains(SETUP) && !browse.contains("demo-cards-mcp"),
        "browse leaked setup instructions"
    );
    ensure!(
        browse.contains("action `select`"),
        "browse did not direct the agent to select"
    );
    ensure!(
        selected.contains("demo-cards-mcp@2.1.0"),
        "select did not return withheld setup instructions"
    );
    for required in [
        "Selected off-catalog product 'demo-other'",
        "Selection recorded as demand data",
        "no provider information",
    ] {
        ensure!(
            off.contains(required),
            "off-catalog receipt missing {required}"
        );
    }
    for forbidden in ["Setup:", SETUP, "https://"] {
        ensure!(
            !off.contains(forbidden),
            "off-catalog selection leaked provider/setup information"
        );
    }
    Ok(())
}
struct Owned {
    child: Child,
    root: PathBuf,
}
impl Drop for Owned {
    fn drop(&mut self) {
        if let Err(e) = crate::detached_acceptance::cleanup_workspace(&self.root) {
            eprintln!("discovery fixture cleanup: {e}");
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
#[derive(Parser)]
struct Args {
    binary: Option<PathBuf>,
}
pub fn run(args: impl Iterator<Item = String>) -> Result<()> {
    ensure!(
        cfg!(target_os = "linux"),
        "Discovery acceptance process ownership requires Linux"
    );
    let a = Args::parse_from(std::iter::once("verify-discovery-select".into()).chain(args));
    let binary = a
        .binary
        .or_else(|| std::env::var_os("NEXODE_BIN").map(PathBuf::from))
        .unwrap_or_else(|| "nexode".into());
    let binary = if binary.components().count() > 1 {
        binary.canonicalize()?
    } else {
        binary
    };
    let server = CatalogServer::new()?;
    let temp = tempfile::Builder::new()
        .prefix("orbit-discovery-")
        .tempdir()?;
    let root = temp.path();
    let home = root.join("home");
    let workspace = root.join("workspace");
    fs::create_dir(&home)?;
    fs::create_dir(&workspace)?;
    fs::write(
        home.join("config.toml"),
        format!(
            "[sponsors]\nenabled = true\nendpoint = {:?}\n",
            server.endpoint
        ),
    )?;
    let socket = root.join("nexode.sock");
    let mut env = BTreeMap::new();
    for (k, v) in [
        ("HOME", home.clone()),
        ("NEXODE_HOME", home.clone()),
        ("NEXODE_RUNTIME_DIR", root.to_owned()),
        ("NEXODE_TEST_WORKSPACE", root.to_owned()),
    ] {
        env.insert(k.to_owned(), v.to_string_lossy().into_owned());
    }
    env.insert(
        "PATH".into(),
        std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".into()),
    );
    for (k, v) in [
        ("NEXODE_DISCOVERY_BENCHMARK", "1"),
        ("NEXODE_DEBUG_CONTROL", "1"),
        ("NEXODE_NO_TELEMETRY", "1"),
        ("NEXODE_TEMP_SERVER", "1"),
    ] {
        env.insert(k.into(), v.into());
    }
    env.insert(
        "NEXODE_SERVER_OWNER_PID".into(),
        std::process::id().to_string(),
    );
    let command = || {
        let mut c = Command::new(&binary);
        c.env_clear()
            .envs(&env)
            .current_dir(&workspace)
            .arg("--socket")
            .arg(&socket)
            .args(["--no-selfdev", "--no-update"]);
        c
    };
    let log = fs::File::create(root.join("server.log"))?;
    let mut launch = command();
    launch
        .args([
            "serve",
            "--server-name",
            &format!("discovery-e2e-{}", std::process::id()),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(log);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        launch.process_group(0);
    }
    let mut owned = Owned {
        child: launch.spawn()?,
        root: root.to_owned(),
    };
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let probe = crate::process::capture(
            command().args(["debug", "server:info"]),
            Duration::from_secs(2),
        );
        if probe.is_ok_and(|o| o.status.success()) {
            break;
        }
        ensure!(
            Instant::now() < deadline && owned.child.try_wait()?.is_none(),
            "benchmark server did not start: {}",
            fs::read_to_string(root.join("server.log")).unwrap_or_default()
        );
        std::thread::sleep(Duration::from_millis(200));
    }
    let execute = |args: &[String]| -> Result<Value> {
        let o = crate::process::capture(command().args(args), Duration::from_secs(60))?;
        ensure!(
            o.status.success(),
            "debug command failed: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        Ok(serde_json::from_slice(&o.stdout)?)
    };
    let created = execute(&[
        "debug".into(),
        format!("create_session:{}", workspace.display()),
    ])?;
    let session = created["session_id"]
        .as_str()
        .context("session ID missing")?;
    let tool = |action: &str, name: Option<&str>, reason: &str| -> Result<String> {
        let mut payload = json!({"action":action,"category":"payments","query":"virtual card capability for agent initiated online purchases","reason":reason});
        if let Some(name) = name {
            payload["tool"] = json!(name);
        }
        let result = execute(&[
            "debug".into(),
            "-S".into(),
            session.into(),
            "tool".into(),
            format!("discover_tools {payload}"),
        ])?;
        Ok(result["output"]
            .as_str()
            .context("tool response missing output")?
            .into())
    };
    let browse = tool(
        "search",
        None,
        "The task needs a spending-limited payment method and no current tool provides one.",
    )?;
    let selected = tool(
        "select",
        Some("demo-cards"),
        "Single-use cards with a hard spending limit match the requested constraint exactly.",
    )?;
    let off = tool(
        "select",
        Some("demo-other"),
        "The user explicitly chose this other product after comparing the available options.",
    )?;
    for (label, text) in [
        ("browse", &browse),
        ("select", &selected),
        ("off-catalog select", &off),
    ] {
        println!("--- {label} ---\n{text}");
    }
    validate(&browse, &selected, &off)?;
    println!(
        "OK: browse withholds setup and names select; catalog select returns setup; off-catalog select records demand without provider or setup information."
    );
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalog_and_receipt_regressions() {
        assert_eq!(catalog(None)["tools"].as_array().unwrap().len(), 2);
        assert_eq!(catalog(Some("demo-cards"))["tool"]["setup"], SETUP);
        let off = catalog(Some("demo-other"));
        assert_eq!(off["listed"], false);
        assert!(off.get("tool").is_none());
        let browse = "demo-cards; action `select`";
        let selected = "demo-cards-mcp@2.1.0";
        let off = "Selected off-catalog product 'demo-other'. Selection recorded as demand data; no provider information";
        validate(browse, selected, off).unwrap();
        assert!(validate(&format!("{browse} {SETUP}"), selected, off).is_err());
        assert!(validate("demo-cards", selected, off).is_err());
        assert!(validate(browse, "missing", off).is_err());
        for suffix in [" Setup:", " https://provider.example"] {
            assert!(validate(browse, selected, &format!("{off}{suffix}")).is_err());
        }
    }
}
