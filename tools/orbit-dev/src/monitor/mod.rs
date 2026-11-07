//! Debug socket monitor with persistent framing across nonblocking reads.
use anyhow::{Result, ensure};
use clap::Parser;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, VecDeque},
    io::{Read, Write},
    path::PathBuf,
    time::{Duration, Instant},
};
#[derive(Default)]
struct State {
    connected: bool,
    session: String,
    processing: bool,
    input: u64,
    output: u64,
    text: String,
    tools: BTreeMap<String, String>,
    history: VecDeque<(String, bool, String)>,
    events: u64,
    last: Option<Instant>,
    errors: VecDeque<String>,
}
fn field(v: &Value, key: &str) -> String {
    v[key].as_str().unwrap_or("").to_owned()
}
fn tail(s: &str, n: usize) -> String {
    s.chars()
        .skip(s.chars().count().saturating_sub(n))
        .collect()
}
fn preview(s: &str, n: usize) -> String {
    let s: String = s
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    if s.chars().count() > n {
        s.chars()
            .take(n.saturating_sub(3))
            .chain("...".chars())
            .collect()
    } else {
        s
    }
}
impl State {
    fn error(&mut self, error: String) {
        self.errors.push_back(error);
        while self.errors.len() > 10 {
            self.errors.pop_front();
        }
    }
    fn event(&mut self, v: &Value) {
        self.events += 1;
        self.last = Some(Instant::now());
        match v["type"].as_str().unwrap_or("") {
            "ack" => self.processing = true,
            "text_delta" => self.text = tail(&(self.text.clone() + &field(v, "text")), 2000),
            "tool_start" => {
                self.tools.insert(
                    field(v, "id"),
                    v["name"].as_str().unwrap_or("unknown").into(),
                );
            }
            "tool_done" => {
                self.tools.remove(&field(v, "id"));
                let output = field(v, "output");
                self.history.push_back((
                    v["name"].as_str().unwrap_or("unknown").into(),
                    v["error"].is_null(),
                    if output.is_empty() {
                        "(empty)".into()
                    } else {
                        output.chars().take(100).collect()
                    },
                ));
                while self.history.len() > 20 {
                    self.history.pop_front();
                }
            }
            "tokens" => {
                self.input = v["input"].as_u64().unwrap_or(0);
                self.output = v["output"].as_u64().unwrap_or(0);
            }
            "done" => {
                self.processing = false;
                self.text.clear();
            }
            "error" => self.error(v["message"].as_str().unwrap_or("unknown error").into()),
            "state" => {
                self.session = field(v, "session_id");
                self.processing = v["is_processing"].as_bool().unwrap_or(false);
            }
            "session" => self.session = field(v, "session_id"),
            _ => {}
        }
    }
    fn render(&self) -> String {
        let mut s = format!(
            "\x1b[2J\x1b[H\x1b[44;37;1m                         NEXODE MONITOR                         \x1b[0m\n\n  Status: {} | Session: {}\n  State: {}\n\n  Tokens\n    Input: {}\n    Output: {}\n\n  Active Tools\n",
            if self.connected {
                "CONNECTED"
            } else {
                "DISCONNECTED"
            },
            preview(&self.session, 40),
            if self.processing {
                "PROCESSING"
            } else {
                "idle"
            },
            self.input,
            self.output
        );
        if self.tools.is_empty() {
            s.push_str("    (none)\n");
        }
        for name in self.tools.values() {
            s.push_str(&format!("    \x1b[36m> {}\x1b[0m\n", preview(name, 70)));
        }
        s.push_str("\n  Recent Tools\n");
        if self.history.is_empty() {
            s.push_str("    (none)\n");
        }
        for (name, ok, output) in self
            .history
            .iter()
            .skip(self.history.len().saturating_sub(5))
        {
            s.push_str(&format!(
                "    \x1b[{}m{}\x1b[0m: {}\n",
                if *ok { 32 } else { 31 },
                preview(name, 28),
                preview(output, 40)
            ));
        }
        s.push_str("\n  Streaming Text\n");
        if self.text.is_empty() {
            s.push_str("    (waiting...)\n");
        }
        let lines: Vec<_> = self.text.split('\n').collect();
        for line in lines.iter().skip(lines.len().saturating_sub(4)) {
            s.push_str(&format!("    {}\n", preview(line, 74)));
        }
        s.push_str(&format!(
            "\n  Events: {} | Last: {}\n",
            self.events,
            self.last
                .map(|t| format!("{:.1}s ago", t.elapsed().as_secs_f64()))
                .unwrap_or_else(|| "waiting".into())
        ));
        if !self.errors.is_empty() {
            s.push_str("\n  Errors\n");
        }
        for error in self.errors.iter().skip(self.errors.len().saturating_sub(3)) {
            s.push_str(&format!("    \x1b[31m{}\x1b[0m\n", preview(error, 74)));
        }
        s
    }
}
#[derive(Default)]
struct Frames {
    bytes: Vec<u8>,
}
impl Frames {
    fn feed(&mut self, bytes: &[u8], state: &mut State) -> Result<()> {
        self.bytes.extend_from_slice(bytes);
        while let Some(i) = self.bytes.iter().position(|b| *b == b'\n') {
            ensure!(i <= 1024 * 1024, "debug event exceeds 1 MiB");
            let line: Vec<_> = self.bytes.drain(..=i).collect();
            if line.iter().all(u8::is_ascii_whitespace) {
                continue;
            }
            match serde_json::from_slice::<Value>(&line) {
                Ok(v) => state.event(&v),
                Err(_) => state.error("Malformed debug event".into()),
            }
        }
        ensure!(self.bytes.len() <= 1024 * 1024, "debug event exceeds 1 MiB");
        Ok(())
    }
}
#[cfg(unix)]
struct Connection {
    stream: std::os::unix::net::UnixStream,
    frames: Frames,
    pending: Vec<u8>,
}
#[cfg(unix)]
impl Connection {
    fn connect(path: &std::path::Path, id: &mut u64) -> Result<Self> {
        let stream = std::os::unix::net::UnixStream::connect(path)?;
        stream.set_nonblocking(true)?;
        let mut c = Self {
            stream,
            frames: Frames::default(),
            pending: vec![],
        };
        c.request("subscribe", id);
        c.request("state", id);
        Ok(c)
    }
    fn request(&mut self, kind: &str, id: &mut u64) {
        self.pending
            .extend_from_slice(format!("{}\n", json!({"type":kind,"id":*id})).as_bytes());
        *id += 1;
    }
    fn tick(&mut self, state: &mut State) -> Result<()> {
        while !self.pending.is_empty() {
            match self.stream.write(&self.pending) {
                Ok(0) => anyhow::bail!("debug socket closed during write"),
                Ok(n) => {
                    self.pending.drain(..n);
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e.into()),
            }
        }
        let mut buffer = [0u8; 4096];
        // Bound each tick so a busy producer cannot starve rendering and pings.
        for _ in 0..256 {
            match self.stream.read(&mut buffer) {
                Ok(0) => anyhow::bail!("debug socket disconnected"),
                Ok(n) => self.frames.feed(&buffer[..n], state)?,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    }
}
#[derive(Parser)]
struct Args {
    /// Legacy positional socket path.
    path: Option<PathBuf>,
    #[arg(long, conflicts_with = "path")]
    socket: Option<PathBuf>,
}
pub fn run(args: impl Iterator<Item = String>) -> Result<()> {
    let args = Args::parse_from(std::iter::once("monitor".into()).chain(args));
    #[cfg(not(unix))]
    {
        let _ = args;
        anyhow::bail!("monitor requires a Unix debug socket");
    }
    #[cfg(unix)]
    {
        let default = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(format!("/run/user/{}", unsafe { libc::getuid() })))
            .join("nexode-debug.sock");
        let path = args.socket.or(args.path).unwrap_or(default);
        let mut state = State::default();
        let mut connection: Option<Connection> = None;
        let mut id = 1;
        let mut last_connect = Instant::now() - Duration::from_secs(1);
        let mut last_ping = Instant::now();
        loop {
            if connection.is_none() && last_connect.elapsed() >= Duration::from_secs(1) {
                last_connect = Instant::now();
                match Connection::connect(&path, &mut id) {
                    Ok(c) => {
                        connection = Some(c);
                        state.connected = true;
                        last_ping = Instant::now();
                    }
                    Err(e) => state.error(e.to_string()),
                }
            }
            if let Some(c) = connection.as_mut() {
                if last_ping.elapsed() >= Duration::from_secs(5) {
                    c.request("ping", &mut id);
                    last_ping = Instant::now();
                }
                if let Err(e) = c.tick(&mut state) {
                    state.error(e.to_string());
                    state.connected = false;
                    connection = None;
                }
            }
            print!("{}", state.render());
            std::io::stdout().flush()?;
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}
#[cfg(test)]
mod tests;
