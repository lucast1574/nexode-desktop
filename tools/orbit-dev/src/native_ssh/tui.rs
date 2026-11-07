use super::*;
use crate::pty::Terminal;
use std::{
    collections::{BTreeMap, BTreeSet},
    os::unix::fs::FileTypeExt,
    time::Instant,
};
pub(super) fn visible(b: &[u8]) -> String {
    regex::Regex::new(r"\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|\x1b\[[0-?]*[ -/]*[@-~]|\x1b[@-_]")
        .unwrap()
        .replace_all(&String::from_utf8_lossy(b), "")
        .into_owned()
}
#[derive(Clone)]
pub(super) struct Process {
    pub parent: u32,
    pub start: String,
    pub argv: Vec<u8>,
}
pub(super) fn info(pid: u32) -> Option<Process> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let fields: Vec<_> = stat.rsplit_once(')')?.1.split_whitespace().collect();
    Some(Process {
        parent: fields.get(1)?.parse().ok()?,
        start: fields.get(19)?.to_string(),
        argv: fs::read(format!("/proc/{pid}/cmdline")).ok()?,
    })
}
fn owned(pid: u32) -> BTreeSet<(u32, String)> {
    let mut all = BTreeMap::new();
    if let Ok(dir) = fs::read_dir("/proc") {
        for p in dir.flatten() {
            if let Ok(pid) = p.file_name().to_string_lossy().parse::<u32>()
                && let Some(i) = info(pid)
            {
                all.insert(pid, i);
            }
        }
    }
    let mut ids = BTreeSet::from([pid]);
    loop {
        let n = ids.len();
        for (&p, i) in &all {
            if ids.contains(&i.parent) {
                ids.insert(p);
            }
        }
        if ids.len() == n {
            break;
        }
    }
    ids.into_iter()
        .filter_map(|p| {
            let i = all.get(&p)?;
            let arg = i.argv.split(|b| *b == 0).next()?;
            let name = Path::new(std::str::from_utf8(arg).ok()?).file_name()?;
            (name == "ssh").then(|| (p, i.start.clone()))
        })
        .collect()
}
fn sockets(root: &Path) -> Result<BTreeSet<PathBuf>> {
    let mut out = BTreeSet::new();
    for e in fs::read_dir(root)? {
        let p = e?.path();
        if p.file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("nexode-ssh-")
        {
            let s = p.join("native.sock");
            if fs::symlink_metadata(&s).is_ok_and(|m| m.file_type().is_socket()) {
                out.insert(s);
            }
        }
    }
    Ok(out)
}
pub(super) struct Tui {
    terminal: Terminal,
    _socket_temp: tempfile::TempDir,
    output: Vec<u8>,
    children: BTreeSet<(u32, String)>,
    sockets: BTreeSet<PathBuf>,
    answered: BTreeSet<(usize, usize)>,
    host: String,
    import: bool,
}
impl Tui {
    pub fn new(
        c: &Config,
        e: &BTreeMap<String, String>,
        root: &Path,
        id: &str,
        import: bool,
    ) -> Result<Self> {
        let socket_temp = tempfile::Builder::new()
            .prefix("jssh-")
            .tempdir_in("/tmp")?;
        let mut command = c.local();
        command
            .args(["--resume", id])
            .current_dir(root)
            .env_clear()
            .envs(e)
            .env("TMPDIR", socket_temp.path());
        let terminal = Terminal::spawn(&mut command, 60, 600)?;
        Ok(Self {
            terminal,
            _socket_temp: socket_temp,
            output: vec![],
            children: BTreeSet::new(),
            sockets: BTreeSet::new(),
            answered: BTreeSet::new(),
            host: c.host.clone(),
            import,
        })
    }
    pub fn pump(&mut self, d: Duration) -> Result<()> {
        let until = Instant::now() + d;
        while Instant::now() < until {
            self.children.extend(owned(self.terminal.pid()));
            self.sockets.extend(sockets(self._socket_temp.path())?);
            let b = self.terminal.read(Duration::from_millis(50))?;
            self.output.extend_from_slice(&b);
            ensure!(
                self.output.len() <= 32 * 1024 * 1024,
                "Unbounded private PTY output"
            );
            for (i, (query, reply)) in [
                (b"\x1b[6n".as_slice(), b"\x1b[1;1R".as_slice()),
                (b"\x1b[?u", b"\x1b[?0u"),
                (b"\x1b[c", b"\x1b[?1;2c"),
                (b"\x1b[>c", b"\x1b[>0;0;0c"),
            ]
            .iter()
            .enumerate()
            {
                for n in self
                    .output
                    .windows(query.len())
                    .enumerate()
                    .filter_map(|(n, b)| (b == *query).then_some(n))
                {
                    if self.answered.insert((i, n)) {
                        self.terminal.write(reply)?
                    }
                }
            }
            if self.import {
                ensure!(
                    auth::token_hashes(&self.output).is_empty()
                        && auth::token_hashes(self.text().as_bytes()).is_empty(),
                    "Synthetic token appeared in PTY; contents withheld"
                );
                for p in std::iter::once(self.terminal.pid())
                    .chain(self.children.iter().map(|(p, _)| *p))
                {
                    ensure!(
                        info(p).is_none_or(|i| auth::token_hashes(&i.argv).is_empty()),
                        "Synthetic token in child argv"
                    );
                }
            }
        }
        Ok(())
    }
    pub fn text(&self) -> String {
        visible(&self.output)
    }
    pub fn mark(&self) -> usize {
        self.output.len()
    }
    pub fn write(&mut self, b: &[u8]) -> Result<()> {
        self.terminal.write(b)
    }
    pub fn repaint(&mut self) -> Result<()> {
        for w in [599, 600] {
            self.terminal.resize(60, w)?;
            self.pump(Duration::from_millis(300))?;
        }
        Ok(())
    }
    pub fn wait(&mut self, marker: &str, since: usize) -> Result<()> {
        let until = Instant::now() + TIMEOUT;
        let mut repaint = Instant::now() + Duration::from_secs(1);
        loop {
            self.pump(Duration::from_millis(150))?;
            if visible(&self.output[since..]).contains(marker) {
                return Ok(());
            }
            ensure!(
                self.terminal.status()?.is_none(),
                "PTY exited before expected marker; raw output withheld"
            );
            ensure!(
                Instant::now() < until,
                "PTY timed out waiting for {}; raw output withheld",
                marker.split("https://").next().unwrap_or("marker")
            );
            if Instant::now() >= repaint {
                self.repaint()?;
                repaint = Instant::now() + Duration::from_secs(1);
            }
        }
    }
    pub fn command(&mut self, s: &str) -> Result<usize> {
        let allowed = if self.import {
            [
                "/login",
                "/login --import-local",
                "/login --import-local openai",
                "/login --import-local claude",
                "/cancel",
                "/quit",
                "confirm",
                "yes",
            ]
            .contains(&s)
        } else {
            [
                "/login",
                "/login openai",
                "/login claude",
                "/login openai-api",
                "/login not-a-provider",
                "/cancel",
                "/quit",
            ]
            .contains(&s)
        };
        ensure!(
            allowed,
            "Only allowlisted non-inference commands are permitted"
        );
        self.pump(Duration::from_millis(200))?;
        let n = self.mark();
        self.write(format!("{s}\r").as_bytes())?;
        Ok(n)
    }
    pub fn startup(&mut self, accept: bool) -> Result<()> {
        self.wait(
            &format!(
                "No logins are configured on {}. Import a local login first?",
                self.host
            ),
            0,
        )?;
        let n = self.mark();
        self.write(if accept { b"yes\r" } else { b"\r" })?;
        self.repaint()?;
        self.wait(&format!("Login on {}:", self.host), n)?;
        if accept {
            self.wait("Import local OpenAI login", n)?;
            self.wait("Import local Claude login", n)?;
        } else {
            self.wait("Anthropic/Claude", n)?;
        }
        Ok(())
    }
    pub fn dismiss(&mut self) -> Result<()> {
        self.startup(false)?;
        let n = self.command("/cancel")?;
        self.wait("No authorization was started.", n)
    }
    pub fn catalog(&mut self) -> Result<()> {
        for (q, labels) in [
            ("openai api", ["OpenAI API", "API key"]),
            ("copilot", ["GitHub Copilot", "device code"]),
            (
                "import local",
                ["Import local OpenAI login", "Import local Claude login"],
            ),
        ] {
            let n = self.mark();
            self.write(format!("\x1b[200~{q}\x1b[201~").as_bytes())?;
            self.repaint()?;
            for label in labels {
                self.wait(label, n)?;
            }
            self.write(b"\x1b")?;
            self.pump(Duration::from_millis(200))?;
        }
        Ok(())
    }
    pub fn quit(&mut self, sighup: bool) -> Result<()> {
        ensure!(
            !self.children.is_empty() && !self.sockets.is_empty(),
            "Did not observe owned real SSH children and native sockets"
        );
        for s in &self.sockets {
            ensure!(
                fs::metadata(s.parent().unwrap())?.permissions().mode() & 0o777 == 0o700,
                "Adapter directory not private0700"
            );
        }
        if sighup {
            ensure!(
                unsafe { libc::kill(self.terminal.pid() as i32, libc::SIGHUP) } == 0,
                "signal owned UI"
            );
        } else {
            self.command("/quit")?;
        }
        let until = Instant::now() + Duration::from_secs(15);
        loop {
            self.pump(Duration::from_millis(100))?;
            if let Some(s) = self.terminal.status()? {
                ensure!(s.success(), "PTY failed on quit");
                break;
            }
            ensure!(Instant::now() < until, "PTY failed to exit");
        }
        for (pid, start) in &self.children {
            ensure!(
                info(*pid).is_none_or(|i| i.start != *start),
                "Owned SSH child survived exit"
            );
        }
        ensure!(
            self.sockets
                .iter()
                .all(|p| !p.exists() && !p.parent().unwrap().exists()),
            "Adapter socket or directory survived exit"
        );
        Ok(())
    }
    pub fn alive(&mut self) -> Result<bool> {
        Ok(self.terminal.status()?.is_none())
    }
}
impl Drop for Tui {
    fn drop(&mut self) {
        let _ = self.terminal.stop();
        for (pid, start) in &self.children {
            if info(*pid).is_some_and(|i| i.start == *start) {
                unsafe {
                    libc::kill(*pid as i32, libc::SIGKILL);
                }
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn strips_controls_only() {
        assert_eq!(
            visible(b"\x1b]0;title\x07\x1b[31mSSH dev\x1b[0m sentinel"),
            "SSH dev sentinel"
        );
    }
    #[test]
    fn sockets_are_private_scoped() {
        use std::os::unix::net::UnixListener;
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("nexode-ssh-owned");
        mkdir(&p).unwrap();
        let s = p.join("native.sock");
        let _listener = UnixListener::bind(&s).unwrap();
        assert_eq!(sockets(d.path()).unwrap(), BTreeSet::from([s]));
    }
}
