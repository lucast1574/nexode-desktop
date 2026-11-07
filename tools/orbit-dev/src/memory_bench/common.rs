use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Read, Write},
    os::unix::{fs::OpenOptionsExt, process::CommandExt},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
pub(super) fn duration(n: f64, zero: bool) -> Result<Duration> {
    ensure!(
        n.is_finite() && n >= 0. && (zero || n > 0.) && n <= 86400.,
        "invalid duration"
    );
    Ok(Duration::from_secs_f64(n))
}
pub(super) fn binary(name: &str, override_path: Option<&Path>) -> Result<PathBuf> {
    if let Some(p) = override_path {
        return Ok(p.canonicalize()?);
    }
    if name == "nexode"
        && let Some(p) = std::env::var_os("NEXODE_BENCH_BIN")
    {
        return Ok(PathBuf::from(p).canonicalize()?);
    }
    if let Some(path) = std::env::var_os("PATH") {
        for p in std::env::split_paths(&path) {
            let p = p.join(name);
            if p.is_file() {
                return Ok(p);
            }
        }
    }
    if name == "pi"
        && let Ok(out) = Command::new("npm").args(["prefix", "-g"]).output()
    {
        let p = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim()).join("bin/pi");
        if p.is_file() {
            return Ok(p);
        }
    }
    let p = dirs::home_dir()
        .context("home missing")?
        .join(".local/bin")
        .join(name);
    ensure!(
        p.is_file(),
        "{name} not found; install it or pass its --*-bin override"
    );
    Ok(p)
}
pub(super) struct ChildGuard {
    pub child: Child,
}
impl ChildGuard {
    pub fn spawn(c: &mut Command) -> Result<Self> {
        c.process_group(0)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        Ok(Self { child: c.spawn()? })
    }
    pub fn piped(c: &mut Command) -> Result<Self> {
        c.process_group(0)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        Ok(Self { child: c.spawn()? })
    }
    pub fn pid(&self) -> u32 {
        self.child.id()
    }
}
impl Drop for ChildGuard {
    fn drop(&mut self) {
        unsafe {
            libc::kill(-(self.child.id() as i32), libc::SIGKILL);
        };
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
pub(super) fn version(bin: &Path, env: &BTreeMap<String, String>) -> Result<String> {
    let mut c = Command::new(bin);
    c.arg("--version").envs(env);
    let mut p = ChildGuard::piped(&mut c)?;
    drop(p.child.stdin.take());
    let stdout = p.child.stdout.take().context("version stdout")?;
    let reader = std::thread::spawn(move || {
        let mut out = String::new();
        stdout.take(65536).read_to_string(&mut out).map(|_| out)
    });
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        if p.child.try_wait()?.is_some() {
            break;
        }
        ensure!(Instant::now() < end, "version command timed out");
        std::thread::sleep(Duration::from_millis(20));
    }
    Ok(reader
        .join()
        .map_err(|_| anyhow::anyhow!("version reader panicked"))??
        .lines()
        .next()
        .unwrap_or("unknown")
        .into())
}
#[derive(Clone)]
struct Proc {
    parent: u32,
    group: u32,
    start: u64,
}
fn stat(s: &str) -> Option<Proc> {
    let f = s
        .rsplit_once(") ")?
        .1
        .split_whitespace()
        .collect::<Vec<_>>();
    Some(Proc {
        parent: f.get(1)?.parse().ok()?,
        group: f.get(2)?.parse().ok()?,
        start: f.get(19)?.parse().ok()?,
    })
}
fn table() -> BTreeMap<u32, Proc> {
    fs::read_dir("/proc")
        .into_iter()
        .flatten()
        .filter_map(|e| {
            let p = e.ok()?.path();
            let pid = p.file_name()?.to_str()?.parse().ok()?;
            Some((pid, stat(&fs::read_to_string(p.join("stat")).ok()?)?))
        })
        .collect()
}
pub(super) struct Ownership {
    pub marker: String,
    pub roots: Vec<u32>,
    seen: BTreeMap<u32, u64>,
}
impl Ownership {
    pub fn new() -> Self {
        Self {
            marker: format!(
                "{}-{}",
                std::process::id(),
                chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
            ),
            roots: vec![],
            seen: BTreeMap::new(),
        }
    }
    pub fn observe(&mut self) {
        let table = table();
        let mut pids = self.roots.iter().copied().collect::<BTreeSet<_>>();
        loop {
            let old = pids.len();
            for (pid, p) in &table {
                if pids.contains(&p.parent) || self.roots.contains(&p.group) {
                    pids.insert(*pid);
                }
            }
            if old == pids.len() {
                break;
            }
        }
        let tag = format!("ORBIT_MEMORY_BENCH_OWNER={}", self.marker);
        for (pid, p) in &table {
            let tagged = fs::read(format!("/proc/{pid}/environ"))
                .is_ok_and(|b| b.split(|b| *b == 0).any(|v| v == tag.as_bytes()));
            if pids.contains(pid) || tagged {
                self.seen.insert(*pid, p.start);
            }
        }
    }
    pub fn sample(&mut self) -> (f64, f64, usize, usize) {
        self.observe();
        let current = table();
        let (mut pss, mut rss, mut count, mut unreadable) = (0., 0., 0, 0);
        for (pid, start) in &self.seen {
            if current.get(pid).is_none_or(|p| p.start != *start) {
                continue;
            }
            match fs::read_to_string(format!("/proc/{pid}/smaps_rollup")) {
                Ok(s) => {
                    let mut has_pss = false;
                    for l in s.lines() {
                        let mut f = l.split_whitespace();
                        let k = f.next();
                        if let Some(n) = f.next().and_then(|n| n.parse::<f64>().ok()) {
                            if k == Some("Pss:") {
                                pss += n / 1024.;
                                has_pss = true
                            }
                            if k == Some("Rss:") {
                                rss += n / 1024.;
                            }
                        }
                    }
                    if has_pss { count += 1 } else { unreadable += 1 }
                }
                Err(_) => unreadable += 1,
            }
        }
        (
            (pss * 10.).round() / 10.,
            (rss * 10.).round() / 10.,
            count,
            unreadable,
        )
    }
}
impl Drop for Ownership {
    fn drop(&mut self) {
        self.observe();
        let current = table();
        for (pid, start) in &self.seen {
            if current.get(pid).is_some_and(|p| p.start == *start) {
                unsafe {
                    libc::kill(*pid as i32, libc::SIGKILL);
                }
            }
        }
    }
}
pub(super) fn wait_socket(path: &Path, timeout: Duration) -> Result<()> {
    let end = Instant::now() + timeout;
    while Instant::now() < end {
        if std::os::unix::net::UnixStream::connect(path).is_ok() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    anyhow::bail!("server socket did not become ready")
}
fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    if from.is_dir() {
        fs::create_dir_all(to)?;
        for e in fs::read_dir(from)? {
            let e = e?;
            copy_tree(&e.path(), &to.join(e.file_name()))?;
        }
    } else {
        fs::copy(from, to)?;
    }
    Ok(())
}
pub(super) fn nexode_home(root: &Path, memory: bool) -> Result<BTreeMap<String, String>> {
    let home = root.join("home");
    let run = root.join("run");
    fs::create_dir_all(&home)?;
    fs::create_dir_all(&run)?;
    let real = std::env::var_os("NEXODE_HOME")
        .map(PathBuf::from)
        .unwrap_or(dirs::home_dir().context("home")?.join(".nexode"));
    for name in [
        "auth.json",
        "anthropic-auth.json",
        "openai-auth.json",
        "antigravity_oauth.json",
        "gemini_oauth.json",
        "config.toml",
    ] {
        if real.join(name).is_file() {
            let bytes = fs::read(real.join(name))?;
            private_write(&home.join(name), &bytes)?;
        }
    }
    if memory && real.join("models").is_dir() {
        copy_tree(&real.join("models"), &home.join("models"))?;
    }
    Ok(BTreeMap::from_iter(
        [
            ("NEXODE_HOME", home.to_string_lossy().into_owned()),
            ("NEXODE_RUNTIME_DIR", run.to_string_lossy().into_owned()),
            ("NEXODE_TEMP_SERVER", "1".into()),
            ("NEXODE_SERVER_OWNER_PID", std::process::id().to_string()),
            ("NEXODE_NO_TELEMETRY", "1".into()),
            ("NEXODE_DEBUG_CONTROL", "1".into()),
            (
                "NEXODE_MEMORY_ENABLED",
                if memory { "1" } else { "0" }.into(),
            ),
            ("NEXODE_EMBEDDING_IDLE_UNLOAD_SECS", "86400".into()),
        ]
        .into_iter()
        .map(|(k, v)| (k.into(), v)),
    ))
}
pub(super) fn private_write(path: &Path, data: &[u8]) -> Result<()> {
    fs::create_dir_all(path.parent().context("parent")?)?;
    let mut f = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(data)?;
    Ok(())
}
pub(super) fn claude_home(root: &Path) -> Result<BTreeMap<String, String>> {
    let original = dirs::home_dir().context("home unavailable")?;
    let config = std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| original.join(".claude"));
    let home = root.join("claude-home");
    let private_config = home.join(".claude");
    fs::create_dir_all(&private_config)?;
    for name in [".credentials.json", "settings.json", "settings.local.json"] {
        if config.join(name).is_file() {
            private_write(&private_config.join(name), &fs::read(config.join(name))?)?;
        }
    }
    if original.join(".claude.json").is_file() {
        private_write(
            &home.join(".claude.json"),
            &fs::read(original.join(".claude.json"))?,
        )?;
    }
    Ok(BTreeMap::from_iter([
        ("HOME".into(), home.to_string_lossy().into_owned()),
        (
            "CLAUDE_CONFIG_DIR".into(),
            private_config.to_string_lossy().into_owned(),
        ),
    ]))
}
pub(super) fn openai_homes(
    root: &Path,
    label: Option<&str>,
) -> Result<BTreeMap<String, BTreeMap<String, String>>> {
    let homes = BTreeMap::from_iter(
        [
            ("codex", vec![("CODEX_HOME", "codex")]),
            (
                "opencode",
                vec![
                    ("XDG_DATA_HOME", "opencode"),
                    ("XDG_CONFIG_HOME", "opencode-config"),
                    ("XDG_STATE_HOME", "opencode-state"),
                ],
            ),
            ("pi", vec![("PI_CODING_AGENT_DIR", "pi")]),
        ]
        .into_iter()
        .map(|(name, items)| {
            (
                name.into(),
                items
                    .into_iter()
                    .map(|(k, p)| (k.into(), root.join(p).to_string_lossy().into_owned()))
                    .collect(),
            )
        }),
    );
    if let Ok(key) = std::env::var("OPENAI_API_KEY") {
        private_write(
            &root.join("codex/auth.json"),
            &serde_json::to_vec(&json!({"auth_mode":"apikey","OPENAI_API_KEY":key}))?,
        )?;
        return Ok(homes);
    }
    let real = std::env::var_os("NEXODE_HOME")
        .map(PathBuf::from)
        .unwrap_or(dirs::home_dir().context("home")?.join(".nexode"));
    let v: Value = serde_json::from_slice(&fs::read(real.join("openai-auth.json"))?)?;
    let label = label
        .or_else(|| v["active_openai_account"].as_str())
        .context("no selected OpenAI account")?;
    let account = v["openai_accounts"]
        .as_array()
        .context("OpenAI accounts")?
        .iter()
        .find(|a| a["label"] == label)
        .context("selected OpenAI account missing")?;
    for field in ["id_token", "access_token", "refresh_token", "account_id"] {
        ensure!(
            account[field].as_str().is_some(),
            "missing OAuth field {field}"
        );
    }
    let codex = json!({"auth_mode":"chatgpt","OPENAI_API_KEY":null,"tokens":{"id_token":account["id_token"],"access_token":account["access_token"],"refresh_token":account["refresh_token"],"account_id":account["account_id"]},"last_refresh":chrono::Utc::now().to_rfc3339()});
    let oauth = json!({"type":"oauth","access":account["access_token"],"refresh":account["refresh_token"],"expires":account["expires_at"],"accountId":account["account_id"]});
    for (p, v) in [
        ("codex/auth.json", codex),
        ("opencode/opencode/auth.json", json!({"openai":oauth})),
        ("pi/auth.json", json!({"openai-codex":oauth})),
    ] {
        private_write(&root.join(p), &serde_json::to_vec(&v)?)?;
    }
    Ok(homes)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn proc_names_may_contain_parentheses_and_spaces() {
        let mut fields = vec!["0"; 20];
        fields[0] = "S";
        fields[1] = "10";
        fields[2] = "42";
        fields[19] = "991";
        let s = format!("42 (a ) b) {}", fields.join(" "));
        let p = stat(&s).unwrap();
        assert_eq!((p.parent, p.group, p.start), (10, 42, 991));
    }
    #[test]
    fn owned_sampling_excludes_unrelated_process() {
        let mut o = Ownership::new();
        assert_eq!(o.sample().2, 0);
        o.roots.push(std::process::id());
        assert!(o.sample().0 > 0.);
        o.roots.clear();
        o.seen.clear();
    }
}
