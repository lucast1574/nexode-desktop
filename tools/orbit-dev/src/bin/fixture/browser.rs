//! Native browser daemon/CLI fixtures, without launching a real browser.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Read, Write},
    os::unix::net::{UnixListener, UnixStream},
    path::PathBuf,
    time::Duration,
};
fn append(path: PathBuf, value: &Value) -> Result<()> {
    writeln!(
        OpenOptions::new().create(true).append(true).open(path)?,
        "{value}"
    )?;
    Ok(())
}
pub fn run() -> Result<i32> {
    let home = PathBuf::from(std::env::var_os("NEXODE_HOME").context("fixture home")?);
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| "/tmp".into());
    let mode = fs::read_to_string(home.join("browser-fixture-mode"))?;
    let user = mode == "user";
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.get(..3) == Some(&["session".into(), "start".into(), "--help".into()]) {
        println!("--bind-window");
        return Ok(0);
    }
    if args.get(..2) == Some(&["session".into(), "start".into()]) {
        let name = args.get(2).context("session name")?;
        let base = runtime.join(format!("browser-session-{name}"));
        let pid = std::process::id();
        append(home.join("starts"), &json!([name, pid]))?;
        let generation = fs::read_to_string(home.join("starts"))?.lines().count();
        if !user {
            if name.contains("blocked") {
                while !home.join("release").exists() {
                    std::thread::sleep(Duration::from_millis(10));
                }
            } else {
                std::thread::sleep(Duration::from_millis(200));
            }
        }
        let listener = UnixListener::bind(format!("{}.sock", base.display()))?;
        fs::write(format!("{}.pid", base.display()), pid.to_string())?;
        for connection in listener.incoming() {
            let mut conn = connection?;
            let mut bytes = [0; 65536];
            if conn.read(&mut bytes)? == 0 {
                continue;
            }
            let reply = if !user {
                json!({"ok":true,"pid":pid,"session":name})
            } else if generation == 1 && home.join("stale").exists() {
                json!({"ok":false,"error":"No tab with id: 41."})
            } else {
                json!({"ok":true,"result":{"tabId":100+generation,"windowId":900+generation}})
            };
            // The test contract intentionally retains Python's spaced JSON output.
            let text = reply
                .as_object()
                .context("reply")?
                .iter()
                .map(|(k, v)| format!("{}: {}", serde_json::to_string(k).unwrap(), v))
                .collect::<Vec<_>>()
                .join(", ");
            if let Err(error) = conn.write_all(format!("{{{text}}}\n").as_bytes()) {
                // Readiness probes may close as soon as they recognize a reply.
                if !matches!(
                    error.kind(),
                    std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::ConnectionReset
                ) {
                    return Err(error.into());
                }
            }
        }
        return Ok(0);
    }
    let session = std::env::var("BROWSER_SESSION").ok();
    let action = args.first().context("browser action")?;
    let params: Value = if user {
        serde_json::from_str(args.get(1).context("params")?)?
    } else {
        json!({})
    };
    if user {
        append(
            home.join("calls"),
            &json!({"action":action,"params":params,"session":session}),
        )?;
        if session.is_none() {
            if action == "uploadFile" && params.get("path").is_none() {
                eprintln!("Error: uploadFile requires 'path' param");
                return Ok(1);
            }
            println!(
                "{}",
                json!({"windows":[{"windowId":1,"tabs":[{"tabId":7,"url":"https://app.example.com/"}]}],"direct":true})
            );
            return Ok(0);
        }
    }
    let session = session.context("BROWSER_SESSION")?;
    let mut conn = UnixStream::connect(runtime.join(format!("browser-session-{session}.sock")))?;
    conn.write_all(format!("{}\n", json!({"action":action,"params":params})).as_bytes())?;
    let mut line = String::new();
    BufReader::new(conn).read_line(&mut line)?;
    if user {
        let reply: Value = serde_json::from_str(&line)?;
        if reply["ok"] != true {
            eprintln!(
                "Error: {}",
                reply["error"].as_str().unwrap_or("fixture error")
            );
            return Ok(1);
        }
        println!("{}", reply["result"]);
    } else {
        ensure!(!line.is_empty(), "empty browser response");
        print!("{line}");
    }
    Ok(0)
}
