use anyhow::Result;
use serde_json::Value;
use std::{
    io::{BufRead, Write},
    path::Path,
};
#[cfg(target_os = "linux")]
pub fn heap_server() -> Result<()> {
    use std::{fs, io::BufReader, os::unix::net::UnixListener};
    assert!(std::env::var_os("REAL_PROVIDER_SECRET").is_none());
    let run = std::path::PathBuf::from(std::env::var_os("NEXODE_RUNTIME_DIR").unwrap());
    fs::write(run.join("pid"), std::process::id().to_string())?;
    let listener = UnixListener::bind(run.join("t.sock"))?;
    for connection in listener.incoming() {
        let mut stream = connection?;
        let mut line = String::new();
        if BufReader::new(stream.try_clone()?).read_line(&mut line)? > 0 {
            let v: Value = serde_json::from_str(&line)?;
            assert_eq!(v["type"], "subscribe");
            writeln!(stream, "{{\"type\":\"done\",\"id\":1}}")?;
        }
    }
    Ok(())
}
pub fn ssh(path: &Path) -> Result<()> {
    let frames: Value = serde_json::from_slice(&std::fs::read(path)?)?;
    let mut hello = frames["hello"].clone();
    hello["server"] = Value::String(std::process::id().to_string());
    let mut output = std::io::stdout().lock();
    for line in std::io::stdin().lock().lines() {
        let request: Value = serde_json::from_str(&line?)?;
        let mut reply = if request["id"] == 1 {
            hello.clone()
        } else {
            frames["pong"].clone()
        };
        reply["reply_to"] = request["id"].clone();
        writeln!(output, "{reply}")?;
        output.flush()?;
    }
    Ok(())
}
#[cfg(unix)]
pub fn bridge(args: &[String]) -> Result<i32> {
    if args.iter().any(|a| a == "--help") {
        return Ok(0);
    }
    if args.iter().any(|a| a == "broken") {
        return Ok(3);
    }
    let path = args
        .first()
        .ok_or_else(|| anyhow::anyhow!("socket path required"))?;
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    let listener = std::os::unix::net::UnixListener::bind(path)?;
    for connection in listener.incoming() {
        drop(connection?);
    }
    Ok(0)
}
