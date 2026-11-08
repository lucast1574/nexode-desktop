use super::*;
use std::{
    os::{fd::AsRawFd, unix::process::CommandExt},
    process::{Child, ChildStderr, ChildStdout},
    time::Instant,
};
pub(super) struct Bridge {
    child: Child,
    out: ChildStdout,
    err: ChildStderr,
    buffer: Vec<u8>,
    stderr: Vec<u8>,
    eof: bool,
}
impl Bridge {
    pub fn new(c: &Config) -> Result<Self> {
        let mut command = c.ssh(&c.remote_command());
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0);
        let mut child = command.spawn()?;
        let out = child.stdout.take().context("stdout")?;
        let err = child.stderr.take().context("stderr")?;
        Ok(Self {
            child,
            out,
            err,
            buffer: vec![],
            stderr: vec![],
            eof: false,
        })
    }
    pub fn send(&mut self, v: Value) -> Result<()> {
        ensure!(
            v["type"] != "message" || v["no_reply"] == true,
            "Only context-only messages are permitted"
        );
        let mut b = serde_json::to_vec(&v)?;
        b.push(b'\n');
        let input = self.child.stdin.as_mut().context("stdin closed")?;
        input.write_all(&b)?;
        input.flush()?;
        Ok(())
    }
    fn frame(&mut self, until: Instant) -> Result<Value> {
        loop {
            if let Some(n) = self.buffer.iter().position(|&b| b == b'\n') {
                ensure!(n <= MAX_FRAME, "Oversized native frame");
                let line: Vec<_> = self.buffer.drain(..=n).collect();
                let v: Value = serde_json::from_slice(&line)?;
                ensure!(v.is_object(), "Native event must be an object");
                return Ok(v);
            }
            ensure!(self.buffer.len() <= MAX_FRAME, "Unbounded native event");
            ensure!(
                !self.eof && Instant::now() < until,
                "No native frame before EOF/deadline; private output withheld"
            );
            let mut poll = [
                libc::pollfd {
                    fd: self.out.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                },
                libc::pollfd {
                    fd: self.err.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                },
            ];
            let ready = unsafe { libc::poll(poll.as_mut_ptr(), 2, 100) };
            if ready < 0 {
                let e = std::io::Error::last_os_error();
                if e.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(e.into());
            }
            for (i, p) in poll.iter().enumerate() {
                if p.revents & (libc::POLLIN | libc::POLLHUP) != 0 {
                    let mut b = [0; 65536];
                    let n = if i == 0 {
                        self.out.read(&mut b)?
                    } else {
                        self.err.read(&mut b)?
                    };
                    if i == 0 {
                        if n == 0 {
                            self.eof = true
                        } else {
                            self.buffer.extend_from_slice(&b[..n])
                        }
                    } else {
                        self.stderr.extend_from_slice(&b[..n]);
                        if self.stderr.len() > 16384 {
                            self.stderr.drain(..self.stderr.len() - 16384);
                        }
                    }
                }
            }
        }
    }
    pub fn until(&mut self, p: impl Fn(&Value) -> bool) -> Result<Value> {
        let end = Instant::now() + TIMEOUT;
        loop {
            let v = self.frame(end)?;
            ensure!(
                v["type"] != "error",
                "Native server rejected request; details withheld"
            );
            ensure!(
                ![
                    "text_delta",
                    "tool_start",
                    "tool_exec",
                    "tool_done",
                    "thinking_delta"
                ]
                .iter()
                .any(|t| v["type"] == *t),
                "Unexpected model/tool activity"
            );
            if p(&v) {
                return Ok(v);
            }
        }
    }
    pub fn handshake(&mut self) -> Result<Value> {
        let h = self.frame(Instant::now() + TIMEOUT)?;
        ensure!(
            h["kind"] == "nexode-native-stdio" && h["protocol"] == 1,
            "Wrong native SSH handshake"
        );
        for k in ["version", "socket_path", "working_dir"] {
            ensure!(
                h[k].as_str().is_some_and(|s| !s.is_empty()),
                "Missing handshake metadata"
            );
        }
        self.send(json!({"type":"ping","id":100}))?;
        let pong = self.until(|v| v["type"] == "pong" && v["id"] == 100)?;
        ensure!(
            pong["native_ssh_protocol"] == 1,
            "Missing persistence capability"
        );
        Ok(h)
    }
    pub fn subscribe(&mut self, cwd: &str, instance: &str, id: Option<&str>) -> Result<Value> {
        let mut v = json!({"type":"subscribe","id":101,"working_dir":cwd,"client_instance_id":instance,"client_has_local_history":false,"allow_session_takeover":true,"continue_on_disconnect":true,"crash_on_disconnect":false,"terminal_env":[]});
        if let Some(id) = id {
            v["target_session_id"] = id.into()
        }
        self.send(v)?;
        self.send(json!({"type":"get_history","id":102}))?;
        self.until(|v| v["type"] == "history")
    }
    pub fn finish(mut self) -> Result<()> {
        self.child.stdin.take();
        let end = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(s) = self.child.try_wait()? {
                ensure!(s.success(), "Remote SSH bridge failed; output withheld");
                return Ok(());
            }
            ensure!(
                Instant::now() < end,
                "Remote bridge failed to exit after stdin EOF"
            ); // Drain while waiting, avoiding pipe-buffer deadlocks.
            let mut p = [
                libc::pollfd {
                    fd: self.out.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                },
                libc::pollfd {
                    fd: self.err.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                },
            ];
            unsafe {
                libc::poll(p.as_mut_ptr(), 2, 20);
            }
            for (i, p) in p.iter().enumerate() {
                if p.revents & libc::POLLIN != 0 {
                    let mut b = [0; 65536];
                    if i == 0 {
                        let _drained = self.out.read(&mut b)?;
                    } else {
                        let _drained = self.err.read(&mut b)?;
                    }
                }
            }
        }
    }
}
impl Drop for Bridge {
    fn drop(&mut self) {
        if !matches!(self.child.try_wait(), Ok(Some(_))) {
            unsafe {
                libc::kill(-(self.child.id() as i32), libc::SIGKILL);
            }
            let _ = self.child.wait();
        }
    }
}
pub(super) fn seed(c: &Config, prefix: &str) -> Result<(Value, String, String)> {
    let sentinel = format!("{prefix}{}", unique()?);
    let mut b = Bridge::new(c)?;
    let h = b.handshake()?;
    let history = b.subscribe(h["working_dir"].as_str().context("cwd")?, &unique()?, None)?;
    let id = history["session_id"]
        .as_str()
        .context("session id")?
        .to_owned();
    b.send(json!({"type":"message","id":103,"content":sentinel,"images":[],"no_reply":true}))?;
    b.until(|v| v["type"] == "context_message_added" && v["id"] == 103)?;
    b.send(json!({"type":"get_history","id":104}))?;
    let history = b.until(|v| v["type"] == "history" && v["id"] == 104)?;
    ensure!(
        history["session_id"] == id && contains(&history, &sentinel),
        "Context-only message not persisted"
    );
    b.finish()?;
    Ok((h, id, sentinel))
}
