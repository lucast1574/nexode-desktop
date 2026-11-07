//! Disposable loopback browser pages and the explicitly invoked live runner.
mod live;
use anyhow::{Context, Result, ensure};
use clap::Parser;
use std::{
    io::Write,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};
use tiny_http::{Header, Response, Server as HttpServer};
fn page(path: &str) -> Option<String> {
    Some(match path.split('?').next().unwrap_or(path){
    "/"=>r#"<h1>Nexode browser handoff test</h1><p>Open Documentation to find browser controls.</p><a href="/docs">Documentation</a>"#.into(),
    "/docs"=>r#"<h1>Documentation</h1><p>Choose the Browser controls section.</p><a href="/browser">Browser controls</a><a href="/">Home</a>"#.into(),
    "/browser"=>"<h1>Browser controls</h1><p>Fast browser integration verified</p>".into(),
    "/blocked"=>r#"<h1>Account verification</h1><label>Verification code <input autocomplete="one-time-code"></label><p>Wait for the user. Do not enter any code.</p>"#.into(),
    "/task"=>format!(r#"<header><nav>{}</nav></header><main><h1>Documentation search</h1><form action="/task/results" method="get"><label>Search <input id="task-search" type="search" name="q" aria-label="Search"></label><select name="section"><option value="all">All documentation</option></select><button type="submit">Search</button></form></main>"#,(0..90).map(|i|format!(r#"<a href="/noise/{i}">Unrelated link {i}</a> "#)).collect::<String>()),
    "/task/results"=>r#"<main><h1>Search results</h1><a id="documentation" href="/task/docs">Documentation</a></main>"#.into(),
    "/task/docs"=>r#"<main><h1>Documentation</h1><p>The Browser controls link is at the bottom of the scrollable sections panel.</p><div id="sections" aria-label="Documentation sections" style="height:200px;overflow-y:auto;border:1px solid"><div style="height:650px">Documentation sections</div><a id="controls" href="/task/complete">Browser controls</a></div></main>"#.into(),
    "/task/complete"=>"<main><h1>Browser controls</h1><p>Whole browser task verified</p></main>".into(),
    _=>return None,
})
}
pub(super) struct Server {
    pub origin: String,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}
impl Server {
    pub fn start(port: u16) -> Result<Self> {
        let server = HttpServer::http(("127.0.0.1", port))
            .map_err(|e| anyhow::anyhow!("bind fixture: {e}"))?;
        let addr = server
            .server_addr()
            .to_ip()
            .context("expected loopback address")?;
        ensure!(addr.ip().is_loopback(), "fixture must remain loopback-only");
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let handle = thread::spawn(move || {
            while !stopped.load(Ordering::Relaxed) {
                match server.recv_timeout(Duration::from_millis(50)) {
                    Ok(Some(request)) => {
                        let content = page(request.url());
                        let status = if content.is_some() { 200 } else { 404 };
                        let body = format!(
                            "<!doctype html><html><head><meta charset=\"utf-8\"><title>Nexode isolated browser fixture</title></head><body>{}</body></html>",
                            content.unwrap_or_else(|| "Not found".into())
                        );
                        let response = Response::from_string(body)
                            .with_status_code(status)
                            .with_header(
                                Header::from_bytes("Content-Type", "text/html; charset=utf-8")
                                    .unwrap(),
                            )
                            .with_header(Header::from_bytes("Cache-Control", "no-store").unwrap());
                        if let Err(e) = request.respond(response) {
                            eprintln!("fixture response: {e}");
                        }
                    }
                    Ok(None) => {}
                    Err(e) => {
                        eprintln!("fixture server: {e}");
                        break;
                    }
                }
            }
        });
        Ok(Self {
            origin: format!("http://{addr}"),
            stop,
            handle: Some(handle),
        })
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}
#[derive(Parser)]
struct Args {
    #[arg(long, default_value_t = 0)]
    port: u16,
}
pub fn run(args: impl Iterator<Item = String>) -> Result<()> {
    let args = Args::parse_from(std::iter::once("browser-fixture".into()).chain(args));
    let server = Server::start(args.port)?;
    println!(
        "Fixture: {}/\nSensitive fixture: /blocked. Stop with Ctrl+C.",
        server.origin
    );
    std::io::stdout().flush()?;
    loop {
        thread::park();
    }
}
pub fn live(repo: &std::path::Path, args: impl Iterator<Item = String>) -> Result<()> {
    live::run(repo, args)
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpStream,
    };
    #[test]
    fn real_loopback_http_pages_and_unknown_routes() {
        let server = Server::start(0).unwrap();
        for (path, expected) in [
            ("/", "Documentation"),
            ("/task/results?q=abc", "id=\"documentation\""),
            ("/blocked", "Do not enter any code"),
            ("/task/docs", "overflow-y:auto"),
            ("/absent", "Not found"),
        ] {
            let mut stream =
                TcpStream::connect(server.origin.trim_start_matches("http://")).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            write!(
                stream,
                "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
            )
            .unwrap();
            let mut text = String::new();
            stream.read_to_string(&mut text).unwrap();
            assert!(text.contains(expected), "{text}");
            assert!(
                text.to_ascii_lowercase()
                    .contains("cache-control: no-store")
            );
            assert!(text.starts_with(if path == "/absent" {
                "HTTP/1.1 404"
            } else {
                "HTTP/1.1 200"
            }));
        }
    }
}
