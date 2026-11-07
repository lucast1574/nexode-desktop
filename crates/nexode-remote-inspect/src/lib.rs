//! Read-only remote workspace inventory. No shell or interpreter is invoked.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const MAX_JSON_BYTES: u64 = 32 * 1024 * 1024;
const MAX_ITEMS: usize = 4096;

#[derive(Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub current_path: Option<String>,
    pub entries: Vec<Entry>,
    pub truncated: bool,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Entry {
    pub path: String,
    pub agents: usize,
    pub busy: usize,
}
// Only deserialize these fields: transcripts and nested tool payloads are skipped.
#[derive(Debug, Default, Deserialize)]
struct Metadata {
    #[serde(default)]
    working_dir: Value,
    #[serde(default)]
    is_debug: Value,
    #[serde(default)]
    is_canary: Value,
}
struct Budget {
    deadline: Instant,
    limit: usize,
    truncated: bool,
}
impl Budget {
    fn check(&self) -> Result<()> {
        ensure!(
            Instant::now() < self.deadline,
            "workspace inventory deadline"
        );
        Ok(())
    }
    fn children(&mut self, path: &Path) -> Result<Vec<PathBuf>> {
        self.check()?;
        let iter = match fs::read_dir(path) {
            Ok(iter) => iter,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };
        let mut entries = Vec::new();
        for entry in iter {
            self.check()?;
            if entries.len() == self.limit {
                self.truncated = true;
                break;
            }
            entries.push(entry?.path());
        }
        Ok(entries)
    }
    fn json(&self, path: &Path) -> Result<Value> {
        self.check()?;
        let mut bytes = Vec::new();
        File::open(path)?
            .take(MAX_JSON_BYTES + 1)
            .read_to_end(&mut bytes)?;
        ensure!(bytes.len() as u64 <= MAX_JSON_BYTES, "oversized metadata");
        Ok(serde_json::from_slice(&bytes)?)
    }
}
struct TimedReader<R> {
    inner: R,
    deadline: Instant,
}
impl<R: Read> Read for TimedReader<R> {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        if Instant::now() >= self.deadline {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "inventory deadline",
            ));
        }
        self.inner.read(bytes)
    }
}
fn session_fields(path: &Path, budget: &Budget) -> Result<Metadata> {
    let journal = path.with_extension("journal.jsonl");
    match File::open(journal) {
        Ok(file) => {
            let mut reader = BufReader::new(TimedReader {
                inner: file,
                deadline: budget.deadline,
            });
            let mut latest = None;
            loop {
                budget.check()?;
                let mut line = Vec::new();
                let n = reader
                    .by_ref()
                    .take(MAX_JSON_BYTES + 1)
                    .read_until(b'\n', &mut line)?;
                if n == 0 {
                    break;
                }
                ensure!(n as u64 <= MAX_JSON_BYTES, "oversized journal record");
                // A partially appended final record is not authoritative.
                if let Ok(entry) = serde_json::from_slice::<Value>(&line)
                    && let Some(meta) = entry.get("meta").filter(|v| v.is_object())
                {
                    latest = Some(serde_json::from_value(meta.clone())?);
                }
            }
            if let Some(meta) = latest {
                return Ok(meta);
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    let reader = BufReader::new(TimedReader {
        inner: File::open(path)?,
        deadline: budget.deadline,
    });
    let result = serde_json::from_reader(reader)?;
    budget.check()?;
    Ok(result)
}
fn directory(value: Option<&str>) -> Option<PathBuf> {
    let path = Path::new(value?);
    if !path.is_absolute() {
        return None;
    }
    // Missing or inaccessible paths are not inventory candidates.
    let Ok(path) = fs::canonicalize(path) else {
        return None;
    };
    path.is_dir().then_some(path)
}
fn repo_root(path: &Path) -> PathBuf {
    path.ancestors()
        .find(|p| p.join(".git").exists())
        .unwrap_or(path)
        .to_owned()
}
fn marker_pid(path: &Path, alive: &impl Fn(i32) -> bool) -> Option<i32> {
    let mut raw = String::new();
    // Markers may disappear while a session exits, or contain an incomplete write.
    let Ok(file) = File::open(path) else {
        return None;
    };
    if file.take(64).read_to_string(&mut raw).is_err() {
        return None;
    }
    let Ok(pid) = raw.trim().parse::<i32>() else {
        return None;
    };
    (pid > 0 && alive(pid)).then_some(pid)
}
#[cfg(unix)]
fn process_alive(pid: i32) -> bool {
    // Signal zero checks existence/permission without signaling the process.
    if unsafe { libc::kill(pid, 0) } != 0 {
        return std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM);
    }
    #[cfg(target_os = "linux")]
    {
        let Ok(stat) = fs::read_to_string(format!("/proc/{pid}/stat")) else {
            return false;
        };
        if stat
            .rsplit_once(')')
            .is_some_and(|(_, state)| state.trim_start().starts_with('Z'))
        {
            return false;
        }
    }
    true
}
#[cfg(not(unix))]
fn process_alive(_pid: i32) -> bool {
    false
}

pub fn collect(
    current: &Path,
    home: &Path,
    state: &Path,
    session: Option<&str>,
) -> Result<Snapshot> {
    collect_with(
        current,
        home,
        state,
        session,
        process_alive,
        MAX_ITEMS,
        Duration::from_secs(4),
    )
}
fn collect_with(
    current: &Path,
    home: &Path,
    state: &Path,
    session: Option<&str>,
    alive: impl Fn(i32) -> bool,
    limit: usize,
    timeout: Duration,
) -> Result<Snapshot> {
    ensure!(limit > 0, "empty inventory limit");
    let home = fs::canonicalize(home)?;
    let mut budget = Budget {
        deadline: Instant::now() + timeout,
        limit,
        truncated: false,
    };
    let mut roots = BTreeSet::new();
    let mut leases = BTreeMap::new();
    for path in budget.children(&state.join("session_leases"))? {
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let lease = match budget.json(&path) {
            Ok(lease) => lease,
            Err(e) if is_missing(&e) => continue,
            Err(e) => return Err(e),
        };
        let root = directory(lease.get("repo_root").and_then(Value::as_str));
        if let Some(root) = &root {
            roots.insert(root.clone());
        }
        if let Some(id) = path.file_stem() {
            leases.insert(id.to_owned(), (lease, root));
        }
    }
    let current = directory(current.to_str());
    if let Some(path) = &current {
        roots.insert(repo_root(path));
    }
    let active = budget.children(&state.join("active_pids"))?;
    ensure!(!budget.truncated, "incomplete live-session inventory");
    let mut live = Vec::new();
    let mut pending = (0_usize, 0_usize);
    for path in active {
        budget.check()?;
        if marker_pid(&path, &alive).is_none() {
            continue;
        }
        let Some(id) = path.file_name() else {
            continue;
        };
        let lease = leases.get(id);
        if lease.is_some_and(|(v, _)| v.get("away_host").is_some_and(|v| !v.is_null())) {
            continue;
        }
        let root = lease.and_then(|(_, root)| root.clone());
        let metadata = match session_fields(
            &state.join("sessions").join({
                let mut name = id.to_owned();
                name.push(".json");
                name
            }),
            &budget,
        ) {
            Ok(meta) => meta,
            Err(e) if is_missing(&e) => {
                if marker_pid(&path, &alive).is_none() {
                    continue;
                }
                if id.to_str() == session
                    && let Some(current) = &current
                {
                    Metadata {
                        working_dir: Value::String(current.to_string_lossy().into_owned()),
                        ..Metadata::default()
                    }
                } else {
                    pending.0 += 1;
                    pending.1 += usize::from(
                        marker_pid(&state.join("streaming_pids").join(id), &alive).is_some(),
                    );
                    continue;
                }
            }
            Err(e) => return Err(e),
        };
        if truthy(&metadata.is_debug) || truthy(&metadata.is_canary) {
            continue;
        }
        let workdir = directory(metadata.working_dir.as_str())
            .or(root.clone())
            .context("live session working directory unavailable")?;
        let root = root.unwrap_or_else(|| repo_root(&workdir));
        roots.insert(root.clone());
        live.push((
            root,
            marker_pid(&state.join("streaming_pids").join(id), &alive).is_some(),
        ));
    }
    let mut parents = BTreeSet::from([home.clone()]);
    for name in ["workspaces", "projects", "repos"] {
        let path = home.join(name);
        if path.is_dir() {
            parents.insert(path);
        }
    }
    for root in &roots {
        if let Some(parent) = root.parent()
            && parent.parent().is_some()
            && Some(parent) != home.parent()
        {
            parents.insert(parent.to_owned());
        }
    }
    for parent in parents {
        for path in budget.children(&parent)? {
            if path
                .file_name()
                .is_some_and(|n| !n.to_string_lossy().starts_with('.'))
                && path.is_dir()
            {
                roots.insert(fs::canonicalize(path)?);
            }
        }
    }
    let mut counts: BTreeMap<_, _> = roots.into_iter().map(|r| (r, (0, 0))).collect();
    for (root, busy) in live {
        let entry = counts.entry(root).or_default();
        entry.0 += 1;
        entry.1 += usize::from(busy);
    }
    let current = current.map(|p| repo_root(&p));
    let mut ordered: Vec<_> = counts.into_iter().collect();
    ordered.sort_by(|(a, ac), (b, bc)| {
        (Some(a) != current.as_ref())
            .cmp(&(Some(b) != current.as_ref()))
            .then_with(|| bc.1.cmp(&ac.1))
            .then_with(|| bc.0.cmp(&ac.0))
            .then_with(|| a.cmp(b))
    });
    let display = |p: &Path| match p.strip_prefix(&home) {
        Ok(rel) if rel.as_os_str().is_empty() => "~".to_string(),
        Ok(rel) => format!("~/{}", rel.to_string_lossy()),
        Err(_) => p.to_string_lossy().into_owned(),
    };
    let available = limit - usize::from(pending.0 > 0);
    budget.truncated |= ordered.len() > available;
    let mut entries: Vec<_> = ordered
        .into_iter()
        .take(available)
        .map(|(p, (agents, busy))| Entry {
            path: display(&p),
            agents,
            busy,
        })
        .collect();
    if pending.0 > 0 {
        entries.push(Entry {
            path: "(directory pending)".into(),
            agents: pending.0,
            busy: pending.1,
        });
    }
    budget.check()?;
    Ok(Snapshot {
        current_path: current.as_deref().map(display),
        entries,
        truncated: budget.truncated,
    })
}
fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64() != Some(0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}
fn is_missing(e: &anyhow::Error) -> bool {
    e.downcast_ref::<std::io::Error>()
        .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound)
}
#[cfg(unix)]
pub fn run(args: impl IntoIterator<Item = OsString>) -> Result<()> {
    let mut args = args.into_iter();
    let current = args
        .next()
        .context("usage: nexode __remote-inventory <directory> [session]")?;
    let session = args.next();
    if args.next().is_some() {
        anyhow::bail!("unexpected inventory argument");
    }
    {
        let home = dirs::home_dir().context("home directory unavailable")?;
        let state = std::env::var_os("NEXODE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".nexode"));
        let snapshot = collect(
            Path::new(&current),
            &home,
            &state,
            session.as_deref().and_then(|s| s.to_str()),
        )?;
        serde_json::to_writer(std::io::stdout().lock(), &snapshot)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;

#[cfg(not(unix))]
pub fn run(_args: impl IntoIterator<Item = OsString>) -> Result<()> {
    anyhow::bail!("remote inventory currently supports Unix hosts")
}
