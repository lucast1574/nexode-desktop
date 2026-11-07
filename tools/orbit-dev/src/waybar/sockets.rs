use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::{fs::FileTypeExt, net::UnixStream},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::SyncSender,
    },
    time::Duration,
};
pub(super) fn discover() -> Vec<PathBuf> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| format!("/run/user/{}", unsafe { libc::getuid() }).into());
    let mut paths = vec![];
    for dir in [runtime, PathBuf::from("/tmp")] {
        if let Ok(entries) = fs::read_dir(dir) {
            for e in entries.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if name.starts_with("nexode")
                    && name.ends_with("-debug.sock")
                    && e.file_type().is_ok_and(|t| t.is_socket())
                {
                    paths.push(e.path());
                }
            }
        }
    }
    paths.sort();
    paths.dedup();
    paths
}
fn request(path: &Path, command: &str) -> Result<Value> {
    let mut stream = UnixStream::connect(path)?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    writeln!(
        stream,
        "{}",
        json!({"type":"debug_command","id":1,"command":command})
    )?;
    let bytes = line(&mut BufReader::new(stream), &AtomicBool::new(false))?
        .context("empty socket response")?;
    let r: Value = serde_json::from_slice(&bytes)?;
    ensure!(r["ok"] == true, "debug command failed");
    Ok(serde_json::from_str(
        r["output"].as_str().context("missing output")?,
    )?)
}
fn line(reader: &mut BufReader<UnixStream>, stop: &AtomicBool) -> Result<Option<Vec<u8>>> {
    let mut bytes = vec![];
    loop {
        if stop.load(Ordering::Relaxed) {
            return Ok(None);
        }
        let data = match reader.fill_buf() {
            Ok(data) => data,
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                return Err(e.into());
            }
            Err(e) => return Err(e.into()),
        };
        if data.is_empty() {
            return Ok(None);
        }
        let end = data.iter().position(|b| *b == b'\n').map(|n| n + 1);
        let n = end.unwrap_or(data.len());
        ensure!(bytes.len() + n <= 1024 * 1024, "debug frame too large");
        bytes.extend_from_slice(&data[..n]);
        reader.consume(n);
        if end.is_some() {
            return Ok(Some(bytes));
        }
    }
}
pub(super) fn collect(paths: &[PathBuf]) -> (u64, u64, u64) {
    let mut sessions = BTreeMap::new();
    let mut servers = BTreeMap::new();
    for path in paths {
        if let Ok(v) = request(path, "sessions")
            && let Some(items) = v.as_array()
        {
            for item in items {
                if let Some(id) = item["session_id"].as_str().filter(|s| !s.is_empty()) {
                    sessions.insert(
                        id.to_owned(),
                        item["is_processing"].as_bool().unwrap_or(false),
                    );
                }
            }
        }
        if let Ok(v) = request(path, "server:info")
            && let Some(id) = v["id"].as_str().filter(|s| !s.is_empty())
        {
            servers.insert(
                id.to_owned(),
                v["spawned_swarm_agent_count"].as_u64().unwrap_or(0),
            );
        }
    }
    (
        sessions.values().filter(|v| **v).count() as u64,
        sessions.len() as u64,
        servers.values().copied().fold(0u64, u64::saturating_add),
    )
}
pub(super) struct Subscriber {
    stop: Arc<AtomicBool>,
    alive: Arc<AtomicBool>,
}
impl Subscriber {
    pub(super) fn alive(&self) -> bool {
        self.alive.load(Ordering::Relaxed)
    }
    pub(super) fn start(path: PathBuf, events: SyncSender<()>) -> Self {
        let sub = Self {
            stop: Arc::new(AtomicBool::new(false)),
            alive: Arc::new(AtomicBool::new(true)),
        };
        let stop = sub.stop.clone();
        let alive = sub.alive.clone();
        std::thread::spawn(move || {
            let run = || -> Result<()> {
                let mut stream = UnixStream::connect(path)?;
                stream.set_read_timeout(Some(Duration::from_secs(1)))?;
                stream.set_write_timeout(Some(Duration::from_secs(2)))?;
                writeln!(
                    stream,
                    "{}",
                    json!({"type":"debug_command","id":1,"command":"events:subscribe:status_change,member_change"})
                )?;
                let mut reader = BufReader::new(stream);
                let mut pending = vec![];
                while !stop.load(Ordering::Relaxed) {
                    // Keep incomplete frames across socket timeouts.
                    match reader.fill_buf() {
                        Ok([]) => break,
                        Ok(bytes) => {
                            let owned = bytes.to_vec();
                            let n = owned.len();
                            reader.consume(n);
                            pending.extend_from_slice(&owned);
                            ensure!(pending.len() <= 1024 * 1024, "debug frame too large");
                            while let Some(i) = pending.iter().position(|b| *b == b'\n') {
                                let line: Vec<_> = pending.drain(..=i).collect();
                                if serde_json::from_slice::<Value>(&line)
                                    .is_ok_and(|v| v["type"] == "event")
                                {
                                    let _ = events.try_send(());
                                }
                            }
                        }
                        Err(e)
                            if matches!(
                                e.kind(),
                                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                            ) => {}
                        Err(e) => return Err(e.into()),
                    }
                }
                Ok(())
            };
            let _ = run();
            alive.store(false, Ordering::Relaxed);
            let _ = events.try_send(());
        });
        sub
    }
}
impl Drop for Subscriber {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unix_queries_deduplicate_sessions_and_servers() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("debug.sock");
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        let task = std::thread::spawn(move || {
            for _ in 0..4 {
                let (mut s, _) = listener.accept().unwrap();
                let mut r = String::new();
                BufReader::new(s.try_clone().unwrap())
                    .read_line(&mut r)
                    .unwrap();
                let v: Value = serde_json::from_str(&r).unwrap();
                let out = if v["command"] == "sessions" {
                    json!([{"session_id":"s","is_processing":true}])
                } else {
                    json!({"id":"server","spawned_swarm_agent_count":3})
                };
                writeln!(s, "{}", json!({"ok":true,"output":out.to_string()})).unwrap();
            }
        });
        assert_eq!(collect(&[path.clone(), path]), (1, 1, 3));
        task.join().unwrap();
    }
}
