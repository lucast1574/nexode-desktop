use anyhow::{Context, Result, ensure};
use regex::Regex;
use std::{
    collections::VecDeque,
    num::NonZeroU16,
    process::Command,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
#[derive(Clone, Default)]
pub(super) struct Snapshot {
    pub rows: Vec<String>,
    pub frames: u64,
    pub bytes: u64,
    pub visible_at: Option<Instant>,
    pub colors: std::collections::BTreeSet<[u8; 3]>,
    pub first_output: Option<Instant>,
    pub last_output: Option<Instant>,
    pub output_events: VecDeque<Instant>,
    pub history: VecDeque<(u64, Instant, Option<String>)>,
    pub error: Option<String>,
    pub eof: bool,
}
pub(super) struct Client {
    pub pid: u32,
    state: Arc<Mutex<Snapshot>>,
    input: mpsc::SyncSender<Vec<u8>>,
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}
pub(super) fn composer(rows: &[String]) -> Option<String> {
    static PROMPT: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    static STATUS: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let prompt = PROMPT.get_or_init(|| Regex::new(r"^\s*\d+>\s?(.*)$").unwrap());
    let status = STATUS.get_or_init(|| Regex::new(r"\s{2,}\S+/\S+\s+[▱▰]*\s*\d+%\s*$").unwrap());
    rows.iter().find_map(|r| {
        prompt
            .captures(r)
            .map(|m| status.replace(&m[1], "").trim_end().to_owned())
    })
}
fn replies(rows: u16, cols: u16) -> Vec<(Vec<u8>, Vec<u8>)> {
    let mut pairs = vec![
        (b"\x1b[6n".to_vec(), b"\x1b[1;1R".to_vec()),
        (b"\x1b[c".to_vec(), b"\x1b[?62;c".to_vec()),
        (b"\x1b[14t".to_vec(), b"\x1b[4;600;800t".to_vec()),
        (b"\x1b[16t".to_vec(), b"\x1b[6;16;8t".to_vec()),
        (
            b"\x1b[18t".to_vec(),
            format!("\x1b[8;{rows};{cols}t").into_bytes(),
        ),
        (b"\x1b[?u".to_vec(), b"\x1b[?3u".to_vec()),
    ];
    for mode in [1016, 2027, 2031, 1004, 2004, 2026] {
        pairs.push((
            format!("\x1b[?{mode}$p").into_bytes(),
            format!("\x1b[?{mode};1$y").into_bytes(),
        ));
    }
    for (code, color) in [(10, "ffff/ffff/ffff"), (11, "0000/0000/0000")] {
        for end in ["\x1b\\", "\x07"] {
            pairs.push((
                format!("\x1b]{code};?{end}").into_bytes(),
                format!("\x1b]{code};rgb:{color}{end}").into_bytes(),
            ));
        }
    }
    for end in ["\x1b\\", "\x07"] {
        pairs.push((
            format!("\x1b]4;0;?{end}").into_bytes(),
            format!("\x1b]4;0;rgb:0000/0000/0000{end}").into_bytes(),
        ));
    }
    pairs
}
fn answers(pending: &mut Vec<u8>, chunk: &[u8], queries: &[(Vec<u8>, Vec<u8>)]) -> Vec<Vec<u8>> {
    pending.extend_from_slice(chunk);
    let mut out = vec![];
    while let Some((index, query, answer)) = queries
        .iter()
        .filter_map(|(q, a)| {
            pending
                .windows(q.len())
                .position(|w| w == q)
                .map(|i| (i, q, a))
        })
        .min_by_key(|(i, _, _)| *i)
    {
        pending.drain(index..index + query.len());
        out.push(answer.clone());
    }
    let keep = queries
        .iter()
        .map(|(q, _)| q.len())
        .max()
        .unwrap_or(1)
        .saturating_sub(1);
    if pending.len() > keep {
        pending.drain(..pending.len() - keep);
    }
    out
}
impl Client {
    pub(super) fn spawn(command: &mut Command, rows: u16, cols: u16) -> Result<Self> {
        Self::spawn_palette(command, rows, cols, None)
    }
    pub(super) fn spawn_palette(
        command: &mut Command,
        rows: u16,
        cols: u16,
        light: Option<bool>,
    ) -> Result<Self> {
        let r = NonZeroU16::new(rows).context("rows must be positive")?;
        let c = NonZeroU16::new(cols).context("cols must be positive")?;
        let mut terminal = crate::pty::Terminal::spawn(command, rows, cols)?;
        let pid = terminal.pid();
        let state = Arc::new(Mutex::new(Snapshot::default()));
        let view = state.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let signal = stop.clone();
        let (input, rx) = mpsc::sync_channel::<Vec<u8>>(256);
        let worker = std::thread::spawn(move || {
            let run = || -> Result<()> {
                let mut parser = vt100::Parser::new(r, c, 0);
                let mut queries = replies(rows, cols);
                if light == Some(true) {
                    for (query, answer) in &mut queries {
                        if query.starts_with(b"\x1b]10;") || query.starts_with(b"\x1b]11;") {
                            *answer = String::from_utf8_lossy(answer)
                                .replace("ffff", "TEMP")
                                .replace("0000", "ffff")
                                .replace("TEMP", "0000")
                                .into_bytes();
                        }
                    }
                }
                let mut colors = super::theme::Colors::default();
                let mut pending = vec![];
                let mut sequence = 0;
                while !signal.load(Ordering::Relaxed) {
                    while let Ok(bytes) = rx.try_recv() {
                        terminal.write(&bytes)?;
                    }
                    let bytes = terminal.read(Duration::from_millis(2))?;
                    if !bytes.is_empty() {
                        parser.process(&bytes);
                        let at = Instant::now();
                        let rows: Vec<_> = parser.screen().rows(0, cols).collect();
                        let current = composer(&rows);
                        if light.is_some() {
                            colors.feed(&bytes)?;
                        }
                        {
                            let mut state = view.lock().unwrap();
                            state.frames += 1;
                            state.bytes += bytes.len() as u64;
                            if state.visible_at.is_none()
                                && rows
                                    .iter()
                                    .flat_map(|r| r.chars())
                                    .filter(|c| c.is_alphanumeric())
                                    .take(9)
                                    .count()
                                    > 8
                            {
                                state.visible_at = Some(at);
                            }
                            if light.is_some() {
                                state.colors = colors.values.clone();
                            }
                            state.first_output.get_or_insert(at);
                            state.last_output = Some(at);
                            state.output_events.push_back(at);
                            if state.output_events.len() > 100_000 {
                                state.output_events.pop_front();
                            }
                            state.rows = rows;
                            if state
                                .history
                                .back()
                                .is_none_or(|(_, _, old)| *old != current)
                            {
                                sequence += 1;
                                state.history.push_back((sequence, at, current));
                                if state.history.len() > 100_000 {
                                    state.history.pop_front();
                                }
                            }
                        }
                        for answer in answers(&mut pending, &bytes, &queries) {
                            terminal.write(&answer)?;
                        }
                    }
                    if terminal.eof || terminal.status()?.is_some() {
                        break;
                    }
                }
                Ok(())
            };
            let mut run = run;
            let result = run();
            let mut s = view.lock().unwrap();
            if let Err(e) = result {
                s.error = Some(e.to_string());
            }
            s.eof = true;
        });
        Ok(Self {
            pid,
            state,
            input,
            stop,
            worker: Some(worker),
        })
    }
    pub(super) fn snapshot(&self) -> Result<Snapshot> {
        let s = self.state.lock().unwrap();
        ensure!(
            s.error.is_none(),
            "terminal pump failed: {}",
            s.error.as_deref().unwrap_or("")
        );
        Ok(s.clone())
    }
    pub(super) fn send(&self, bytes: &[u8]) -> Result<()> {
        ensure!(bytes.len() <= 65536, "terminal input too large");
        self.input
            .try_send(bytes.to_vec())
            .context("terminal input queue closed or full")
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
#[cfg(test)]
mod tests;
