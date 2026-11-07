use anyhow::{Context, Result, ensure};
use serde_json::json;
use std::{
    fs::{File, OpenOptions},
    io::{BufRead, BufReader, Read, Seek, SeekFrom, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
#[derive(Clone)]
pub struct Scenario {
    pub tarball: PathBuf,
    pub checksum: String,
    pub stem: String,
    pub version: String,
    pub rate: f64,
    pub drop_at: Option<f64>,
    pub drop_stall: f64,
    pub check_delay: f64,
    pub bad_checksum: bool,
    pub log: PathBuf,
}
pub struct Server {
    pub url: String,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
impl Server {
    pub fn start(scenario: Scenario) -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let url = format!("http://{}", listener.local_addr()?);
        let base = url.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let scenario = Arc::new(scenario);
        let dropped = Arc::new(AtomicBool::new(false));
        let handle = thread::spawn(move || {
            while !stopping.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let sc = scenario.clone();
                        let base = base.clone();
                        let dropped = dropped.clone();
                        thread::spawn(move || {
                            if let Err(e) = serve(stream, &sc, &base, &dropped)
                                && let Ok(mut log) =
                                    OpenOptions::new().create(true).append(true).open(&sc.log)
                            {
                                let _ = writeln!(log, "{e:#}");
                            }
                        });
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10))
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            url,
            stop,
            thread: Some(handle),
        })
    }
}
fn headers(stream: &mut TcpStream, code: u16, kind: &str, size: u64, extra: &str) -> Result<()> {
    write!(
        stream,
        "HTTP/1.1 {code} {}\r\nContent-Type: {kind}\r\nContent-Length: {size}\r\nConnection: close\r\n{extra}\r\n",
        match code {
            200 => "OK",
            206 => "Partial Content",
            416 => "Range Not Satisfiable",
            _ => "Not Found",
        }
    )?;
    Ok(())
}
fn bytes(stream: &mut TcpStream, body: &[u8], kind: &str) -> Result<()> {
    headers(stream, 200, kind, body.len() as u64, "")?;
    stream.write_all(body)?;
    Ok(())
}
fn serve(mut stream: TcpStream, sc: &Scenario, base: &str, dropped: &AtomicBool) -> Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut first = String::new();
    reader.read_line(&mut first)?;
    ensure!(first.len() < 8192, "request line too long");
    let mut range = None;
    let mut total = first.len();
    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line)?;
        total += n;
        ensure!(total < 32768, "headers too long");
        if n == 0 || line == "\r\n" {
            break;
        }
        if let Some((k, v)) = line.split_once(':')
            && k.eq_ignore_ascii_case("Range")
        {
            range = Some(v.trim().to_owned());
        }
    }
    let path = first
        .split_whitespace()
        .nth(1)
        .context("missing request path")?;
    let size = sc.tarball.metadata()?.len();
    if path == "/release.json" {
        thread::sleep(Duration::from_secs_f64(sc.check_delay));
        let body = json!({"tag_name":format!("v{}",sc.version),"name":format!("v{} (rehearsal)",sc.version),"html_url":"https://example.invalid/rehearsal","published_at":"2026-01-01T00:00:00Z","target_commitish":"main","assets":[{"name":format!("{}.tar.gz",sc.stem),"browser_download_url":format!("{base}/{}.tar.gz",sc.stem),"size":size},{"name":"SHA256SUMS","browser_download_url":format!("{base}/SHA256SUMS"),"size":0}]});
        return bytes(&mut stream, &serde_json::to_vec(&body)?, "application/json");
    }
    if path == "/SHA256SUMS" {
        return bytes(
            &mut stream,
            format!(
                "{}  {}.tar.gz\n",
                if sc.bad_checksum {
                    "0".repeat(64)
                } else {
                    sc.checksum.clone()
                },
                sc.stem
            )
            .as_bytes(),
            "text/plain",
        );
    }
    if path != format!("/{}.tar.gz", sc.stem) {
        return headers(&mut stream, 404, "text/plain", 0, "");
    }
    let start = match range {
        Some(r) => match r
            .strip_prefix("bytes=")
            .and_then(|r| r.split_once('-'))
            .and_then(|(s, e)| {
                if e.is_empty() {
                    s.parse::<u64>().ok()
                } else {
                    None
                }
            }) {
            Some(s) if s < size => s,
            _ => {
                return headers(
                    &mut stream,
                    416,
                    "text/plain",
                    0,
                    &format!("Content-Range: bytes */{size}\r\n"),
                );
            }
        },
        None => 0,
    };
    let code = if start > 0 { 206 } else { 200 };
    let extra = if start > 0 {
        format!(
            "Accept-Ranges: bytes\r\nContent-Range: bytes {start}-{}/{size}\r\n",
            size - 1
        )
    } else {
        "Accept-Ranges: bytes\r\n".into()
    };
    headers(&mut stream, code, "application/gzip", size - start, &extra)?;
    writeln!(
        OpenOptions::new().create(true).append(true).open(&sc.log)?,
        "GET {path} {code} start={start}"
    )?;
    let drop_at = sc.drop_at.map(|p| (size as f64 * p / 100.0) as u64);
    let mut file = File::open(&sc.tarball)?;
    file.seek(SeekFrom::Start(start))?;
    let mut sent = start;
    let began = Instant::now();
    let mut buf = vec![0; 256 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        if drop_at.is_some_and(|d| sent + n as u64 >= d) && !dropped.swap(true, Ordering::SeqCst) {
            writeln!(
                OpenOptions::new().create(true).append(true).open(&sc.log)?,
                "dropping connection at {sent} bytes (simulated)"
            )?;
            thread::sleep(Duration::from_secs_f64(sc.drop_stall));
            return Ok(());
        }
        stream.write_all(&buf[..n])?;
        sent += n as u64;
        if sc.rate > 0.0 {
            let expected =
                Duration::from_secs_f64((sent - start) as f64 / (sc.rate * 1024.0 * 1024.0));
            if let Some(delay) = expected.checked_sub(began.elapsed()) {
                thread::sleep(delay);
            }
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn get(base: &str, path: &str, range: &str) -> Vec<u8> {
        let mut stream = TcpStream::connect(base.strip_prefix("http://").unwrap()).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        write!(
            stream,
            "GET {path} HTTP/1.1\r\nHost: localhost\r\n{range}\r\n"
        )
        .unwrap();
        let mut bytes = Vec::new();
        stream.read_to_end(&mut bytes).unwrap();
        bytes
    }
    #[test]
    fn release_checksum_drop_resume_and_invalid_ranges() {
        let tmp = tempfile::tempdir().unwrap();
        let tarball = tmp.path().join("release");
        std::fs::write(&tarball, vec![42; 600000]).unwrap();
        let sc = Scenario {
            tarball,
            checksum: "a".repeat(64),
            stem: "nexode-linux-x86_64".into(),
            version: "1.0.1".into(),
            rate: 0.0,
            drop_at: Some(50.0),
            drop_stall: 0.0,
            check_delay: 0.0,
            bad_checksum: true,
            log: tmp.path().join("server.log"),
        };
        let server = Server::start(sc).unwrap();
        assert!(
            String::from_utf8(get(&server.url, "/release.json", ""))
                .unwrap()
                .contains("v1.0.1")
        );
        assert!(
            String::from_utf8(get(&server.url, "/SHA256SUMS", ""))
                .unwrap()
                .contains(&"0".repeat(64))
        );
        let first = get(&server.url, "/nexode-linux-x86_64.tar.gz", "");
        let split = first.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
        let count = first.len() - split;
        assert!(count < 600000);
        let resumed = get(
            &server.url,
            "/nexode-linux-x86_64.tar.gz",
            &format!("Range: bytes={count}-\r\n"),
        );
        assert!(resumed.starts_with(b"HTTP/1.1 206"));
        let split = resumed.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
        assert_eq!(resumed.len() - split, 600000 - count);
        assert!(
            get(
                &server.url,
                "/nexode-linux-x86_64.tar.gz",
                "Range: bytes=999999-\r\n"
            )
            .starts_with(b"HTTP/1.1 416")
        );
        assert!(get(&server.url, "/missing", "").starts_with(b"HTTP/1.1 404"));
    }
}
