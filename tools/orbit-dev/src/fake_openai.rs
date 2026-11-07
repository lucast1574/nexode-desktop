//! Deterministic loopback provider. No real credentials or inference service.
use anyhow::{Context, Result};
use clap::Parser;
use regex::Regex;
use serde_json::{Value, json};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tiny_http::{Header, Method, Request, Response, Server as HttpServer};
#[derive(Clone)]
pub struct Options {
    pub log: PathBuf,
    pub label: String,
    pub control: Option<PathBuf>,
}
pub struct Server {
    pub port: u16,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}
impl Server {
    pub fn start(port: u16, opts: Options) -> Result<Self> {
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(&opts.log)?;
        let server = HttpServer::http(("127.0.0.1", port))
            .map_err(|e| anyhow::anyhow!("bind provider: {e}"))?;
        let port = server
            .server_addr()
            .to_ip()
            .context("IP listener required")?
            .port();
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let lock = Arc::new(Mutex::new(()));
        let handle = thread::spawn(move || {
            while !stopping.load(Ordering::Relaxed) {
                match server.recv_timeout(Duration::from_millis(30)) {
                    Ok(Some(request)) => {
                        let opts = opts.clone();
                        let lock = lock.clone();
                        let stopping = stopping.clone();
                        thread::spawn(move || {
                            if let Err(e) = serve(request, &opts, &lock, &stopping) {
                                eprintln!("fixture provider: {e:#}");
                            }
                        });
                    }
                    Ok(None) => {}
                    Err(e) => {
                        eprintln!("fixture listener: {e}");
                        break;
                    }
                }
            }
        });
        Ok(Self {
            port,
            stop,
            handle: Some(handle),
        })
    }
}
fn text_of(message: &Value) -> String {
    match &message["content"] {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter(|p| p.is_object())
            .map(|p| p["text"].as_str().unwrap_or(""))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}
fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(v) => *v,
        Value::Array(v) => !v.is_empty(),
        Value::Object(v) => !v.is_empty(),
        Value::String(v) => !v.is_empty(),
        Value::Number(v) => v.as_f64() != Some(0.0),
    }
}
fn respond(request: Request, body: String, kind: &str, code: u16) -> Result<()> {
    request.respond(
        Response::from_string(body)
            .with_status_code(code)
            .with_header(
                Header::from_bytes("content-type", kind)
                    .map_err(|_| anyhow::anyhow!("content type"))?,
            ),
    )?;
    Ok(())
}
fn serve(mut request: Request, opts: &Options, lock: &Mutex<()>, stop: &AtomicBool) -> Result<()> {
    if request.method() == &Method::Get {
        return respond(
            request,
            json!({"data":[{"id":"fake-model","object":"model"}]}).to_string(),
            "application/json",
            200,
        );
    }
    if request.method() != &Method::Post {
        return respond(request, "Unsupported method".into(), "text/plain", 405);
    }
    let mut bytes = Vec::new();
    request
        .as_reader()
        .take(8 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 8 * 1024 * 1024 {
        return respond(request, "Request too large".into(), "text/plain", 413);
    }
    let req: Value = if bytes.is_empty() {
        json!({})
    } else {
        match serde_json::from_slice(&bytes) {
            Ok(v) => v,
            Err(_) => return respond(request, "Invalid JSON".into(), "text/plain", 400),
        }
    };
    let msgs = req["messages"].as_array().cloned().unwrap_or_default();
    {
        let _guard = lock
            .lock()
            .map_err(|_| anyhow::anyhow!("request log mutex poisoned"))?;
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs_f64();
        let record =
            json!({"label":opts.label,"messages":msgs,"agent":truthy(&req["tools"]),"time":now});
        writeln!(
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(&opts.log)?,
            "{record}"
        )?;
    }
    let flat = serde_json::to_string(&msgs)?;
    let seen = flat.contains("moved from") && flat.contains("to cloud host");
    let text = format!(
        "[{}] ack. saw_cloud_notice={}",
        opts.label,
        if seen { "True" } else { "False" }
    );
    let last = msgs.iter().rposition(|m| m["role"] == "user");
    let prompt = last.map(|i| text_of(&msgs[i])).unwrap_or_default();
    let has_tool = msgs[last.map_or(0, |i| i + 1)..]
        .iter()
        .any(|m| m["role"] == "tool");
    let mut tool = None;
    if let Some(control) = &opts.control {
        if prompt.contains("[e2e:todo]") && !has_tool {
            tool = Some(
                json!({"id":"call_acceptance_todo","type":"function","function":{"name":"todo","arguments":json!({"intent":"Create an open task for the handoff acceptance test","todos":[{"id":"acceptance-open","content":"Continue on cloud","status":"in_progress","priority":"high","confidence":"plausible"}]}).to_string()}}),
            );
        } else if let Some(c) = Regex::new(r"\[e2e:hold:([a-z0-9_-]+)\]")?.captures(&prompt) {
            let token = &c[1];
            fs::create_dir_all(control)?;
            fs::write(
                control.join(format!("{token}.held")),
                "provider request received\n",
            )?;
            let deadline = Instant::now() + Duration::from_secs(90);
            while !control.join(format!("{token}.release")).exists() {
                if Instant::now() >= deadline || stop.load(Ordering::Relaxed) {
                    return respond(
                        request,
                        "acceptance test did not release provider gate".into(),
                        "text/plain",
                        504,
                    );
                }
                thread::sleep(Duration::from_millis(50));
            }
        }
    }
    let finish = if tool.is_some() { "tool_calls" } else { "stop" };
    let usage = json!({"prompt_tokens":1,"completion_tokens":1,"total_tokens":2});
    if !truthy(&req["stream"]) {
        let mut message =
            json!({"role":"assistant","content":if tool.is_some(){Value::Null}else{json!(text)}});
        if let Some(tool) = tool {
            message["tool_calls"] = json!([tool]);
        }
        return respond(request,json!({"id":"x","object":"chat.completion","model":"fake-model","choices":[{"index":0,"finish_reason":finish,"message":message}],"usage":usage}).to_string(),"application/json",200);
    }
    let delta = if let Some(mut tool) = tool {
        tool["index"] = json!(0);
        json!({"role":"assistant","tool_calls":[tool]})
    } else {
        json!({"role":"assistant","content":text})
    };
    let first = json!({"id":"x","object":"chat.completion.chunk","model":"fake-model","choices":[{"index":0,"delta":delta}]});
    let last = json!({"id":"x","object":"chat.completion.chunk","model":"fake-model","choices":[{"index":0,"delta":{},"finish_reason":finish}],"usage":usage});
    respond(
        request,
        format!("data: {first}\n\ndata: {last}\n\ndata: [DONE]\n\n"),
        "text/event-stream",
        200,
    )
}
#[derive(Parser)]
struct Args {
    port: u16,
}
pub fn run(args: impl Iterator<Item = String>) -> Result<()> {
    let args = Args::parse_from(std::iter::once("fake-openai-server".into()).chain(args));
    let log = std::env::var_os("FAKE_LOG")
        .filter(|s| !s.is_empty())
        .context("FAKE_LOG must name an isolated request log")?;
    let server = Server::start(
        args.port,
        Options {
            log: log.into(),
            label: std::env::var("FAKE_LABEL").unwrap_or_else(|_| "model".into()),
            control: std::env::var_os("FAKE_CONTROL_DIR")
                .filter(|s| !s.is_empty())
                .map(PathBuf::from),
        },
    )?;
    if let Some(p) = std::env::var_os("FAKE_PORT_FILE") {
        fs::write(p, server.port.to_string())?;
    }
    loop {
        thread::park();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpStream;
    fn request(port: u16, messages: Value, stream: bool) -> String {
        let body = json!({"messages":messages,"stream":stream}).to_string();
        let mut socket = TcpStream::connect(("127.0.0.1", port)).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        write!(socket,"POST /v1/chat/completions HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
        let mut reply = String::new();
        socket.read_to_string(&mut reply).unwrap();
        assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
        reply.split_once("\r\n\r\n").unwrap().1.into()
    }
    fn start(root: &std::path::Path) -> Server {
        Server::start(
            0,
            Options {
                log: root.join("requests.jsonl"),
                label: "model".into(),
                control: Some(root.into()),
            },
        )
        .unwrap()
    }
    #[test]
    fn todo_uses_real_tool_wire_shape_before_gate() {
        let t = tempfile::tempdir().unwrap();
        let server = start(t.path());
        let reply: Value = serde_json::from_str(&request(
            server.port,
            json!([{"role":"user","content":"[e2e:todo] [e2e:hold:abc] work"}]),
            false,
        ))
        .unwrap();
        assert_eq!(reply["choices"][0]["finish_reason"], "tool_calls");
        let tool = &reply["choices"][0]["message"]["tool_calls"][0];
        assert_eq!(tool["function"]["name"], "todo");
        let args: Value =
            serde_json::from_str(tool["function"]["arguments"].as_str().unwrap()).unwrap();
        assert_eq!(args["todos"][0]["status"], "in_progress");
        assert!(!t.path().join("abc.held").exists());
    }
    #[test]
    fn gate_waits_for_release_and_does_not_repeat_tool() {
        let t = tempfile::tempdir().unwrap();
        let server = start(t.path());
        let port = server.port;
        let handle = thread::spawn(move || {
            request(
                port,
                json!([{"role":"user","content":"[e2e:todo] [e2e:hold:abc] work"},{"role":"assistant","content":null,"tool_calls":[]},{"role":"tool","tool_call_id":"call_acceptance_todo","content":"saved"}]),
                true,
            )
        });
        let deadline = Instant::now() + Duration::from_secs(3);
        while !t.path().join("abc.held").exists() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(t.path().join("abc.held").exists());
        assert!(!handle.is_finished());
        fs::write(t.path().join("abc.release"), "").unwrap();
        let reply = handle.join().unwrap();
        assert!(reply.contains("[DONE]"));
        assert!(!reply.contains("\"tool_calls\""));
        assert_eq!(
            fs::read_to_string(t.path().join("requests.jsonl"))
                .unwrap()
                .lines()
                .count(),
            1
        );
    }
    #[test]
    fn newer_prompt_does_not_trigger_historical_gate_and_notice_is_detected() {
        let t = tempfile::tempdir().unwrap();
        let server = start(t.path());
        let reply:Value=serde_json::from_str(&request(server.port,json!([{"role":"user","content":"[e2e:hold:old]"},{"role":"assistant","content":"done"},{"role":"user","content":[{"type":"text","text":"moved from laptop to cloud host"}]}]),false)).unwrap();
        assert_eq!(reply["choices"][0]["finish_reason"], "stop");
        assert!(
            reply["choices"][0]["message"]["content"]
                .as_str()
                .unwrap()
                .contains("saw_cloud_notice=True")
        );
        assert!(!t.path().join("old.held").exists());
    }
}
