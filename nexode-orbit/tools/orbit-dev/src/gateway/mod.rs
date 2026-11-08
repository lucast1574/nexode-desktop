//! HTTP pairing and RFC6455 gateway diagnostics. Live calls require explicit CLI use.
mod check;
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::{
    collections::VecDeque,
    io::Read,
    net::{TcpStream, ToSocketAddrs},
    time::{Duration, Instant},
};
use tungstenite::{
    Message, WebSocket,
    client::{IntoClientRequest, client_with_config},
    protocol::WebSocketConfig,
};
fn endpoint(host: &str, port: u16, path: &str) -> Result<url::Url> {
    ensure!(port > 0, "port must be nonzero");
    let mut url = url::Url::parse("http://localhost")?;
    if let Ok(ip) = host.parse() {
        url.set_ip_host(ip)
            .map_err(|_| anyhow::anyhow!("invalid IP host"))?;
    } else {
        url.set_host(Some(host)).context("invalid host")?;
    }
    ensure!(
        url.host_str().is_some() && !host.contains(['/', '@', '?', '#']),
        "host must contain only a hostname or IP"
    );
    url.set_port(Some(port))
        .map_err(|_| anyhow::anyhow!("invalid port"))?;
    url.set_path(path);
    Ok(url)
}
fn http(host: &str, port: u16, path: &str, body: Option<&Value>) -> Result<Value> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let url = endpoint(host, port, path)?;
    let request = if let Some(body) = body {
        client.post(url).json(body)
    } else {
        client.get(url)
    };
    let response = request.send().map_err(|e| e.without_url())?;
    ensure!(
        response.status().is_success(),
        "gateway {path} returned HTTP {}",
        response.status()
    );
    let mut bytes = vec![];
    response.take(1024 * 1024 + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 1024 * 1024, "gateway response too large");
    Ok(serde_json::from_slice(&bytes)?)
}
struct Gateway {
    socket: WebSocket<TcpStream>,
    id: u64,
    queued: VecDeque<Value>,
}
impl Gateway {
    fn connect(host: &str, port: u16, token: &str, timeout: Duration) -> Result<Self> {
        let mut url = endpoint(host, port, "/ws")?;
        url.set_scheme("ws")
            .map_err(|_| anyhow::anyhow!("invalid scheme"))?;
        let mut request = url.as_str().into_client_request()?;
        request.headers_mut().insert(
            "Authorization",
            format!("Bearer {token}")
                .parse()
                .map_err(|_| anyhow::anyhow!("invalid cached token"))?,
        );
        let deadline = Instant::now() + timeout;
        let mut stream = None;
        for addr in (host, port).to_socket_addrs()? {
            let Some(left) = deadline.checked_duration_since(Instant::now()) else {
                break;
            };
            if let Ok(s) = TcpStream::connect_timeout(&addr, left) {
                stream = Some(s);
                break;
            }
        }
        let stream = stream.context("gateway TCP connection failed")?;
        stream.set_read_timeout(Some(timeout))?;
        stream.set_write_timeout(Some(timeout))?;
        let config = WebSocketConfig {
            max_message_size: Some(4 * 1024 * 1024),
            max_frame_size: Some(4 * 1024 * 1024),
            max_write_buffer_size: 8 * 1024 * 1024,
            ..Default::default()
        };
        let (socket, _) = client_with_config(request, stream, Some(config))
            .map_err(|_| anyhow::anyhow!("gateway WebSocket handshake failed"))?;
        Ok(Self {
            socket,
            id: 0,
            queued: VecDeque::new(),
        })
    }
    fn send(&mut self, mut value: Value) -> Result<u64> {
        self.id = self.id.checked_add(1).context("request id overflow")?;
        ensure!(value.is_object(), "request must be object");
        if value.get("id").is_none() {
            value["id"] = Value::from(self.id);
        }
        let id = value["id"].as_u64().context("request ID must be integer")?;
        self.socket
            .send(Message::Text(serde_json::to_string(&value)?))?;
        Ok(id)
    }
    fn next(&mut self, deadline: Instant) -> Result<Option<Value>> {
        loop {
            if let Some(v) = self.queued.pop_front() {
                return Ok(Some(v));
            }
            let Some(left) = deadline
                .checked_duration_since(Instant::now())
                .filter(|d| !d.is_zero())
            else {
                return Ok(None);
            };
            self.socket.get_mut().set_read_timeout(Some(left))?;
            match self.socket.read() {
                Ok(Message::Text(text)) => {
                    for line in text.lines().filter(|s| !s.trim().is_empty()) {
                        self.queued.push_back(
                            serde_json::from_str(line).context("malformed gateway event")?,
                        );
                    }
                }
                Ok(Message::Ping(_)) => self.socket.flush()?,
                Ok(Message::Close(_))
                | Err(tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed) => {
                    return Ok(None);
                }
                Ok(_) => {}
                Err(tungstenite::Error::Io(e))
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                    ) =>
                {
                    return Ok(None);
                }
                Err(_) => anyhow::bail!("gateway WebSocket read failed"),
            }
        }
    }
}
impl Drop for Gateway {
    fn drop(&mut self) {
        let _ = self
            .socket
            .get_mut()
            .set_write_timeout(Some(Duration::from_millis(200)));
        let _ = self.socket.close(None);
    }
}
pub fn run(args: impl Iterator<Item = String>) -> Result<()> {
    check::run(args)
}
#[cfg(test)]
mod tests;
