use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    io::{Read, Write},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};
use tiny_http::{Header, Request, Response, Server};

#[derive(Default)]
pub(super) struct State {
    pub events: Vec<Value>,
    pub errors: Vec<String>,
    registration: Option<Value>,
    authorization: Option<HashMap<String, String>>,
    streams: HashMap<String, mpsc::Sender<Vec<u8>>>,
}
pub(super) struct Fixture {
    pub base: String,
    pub token: String,
    pub static_token: String,
    code: String,
    pub state: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}
fn secret() -> Result<String> {
    let mut bytes = [0; 32];
    getrandom::fill(&mut bytes).map_err(|_| anyhow::anyhow!("fixture random generator failed"))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}
impl Fixture {
    pub fn new() -> Result<Self> {
        let server =
            Server::http("127.0.0.1:0").map_err(|_| anyhow::anyhow!("cannot bind MCP fixture"))?;
        let base = format!("http://{}", server.server_addr());
        let mut fixture = Self {
            base,
            token: secret()?,
            static_token: secret()?,
            code: secret()?,
            state: Arc::new(Mutex::new(State::default())),
            stop: Arc::new(AtomicBool::new(false)),
            worker: None,
        };
        let handler = Arc::new(Self {
            base: fixture.base.clone(),
            token: fixture.token.clone(),
            static_token: fixture.static_token.clone(),
            code: fixture.code.clone(),
            state: fixture.state.clone(),
            stop: fixture.stop.clone(),
            worker: None,
        });
        fixture.worker = Some(thread::spawn(move || {
            let mut handlers = vec![];
            while !handler.stop.load(Ordering::SeqCst) {
                match server.recv_timeout(Duration::from_millis(50)) {
                    Ok(Some(request)) => {
                        let fixture = handler.clone();
                        handlers.push(thread::spawn(move || {
                            if fixture.dispatch(request).is_err() {
                                fixture
                                    .state
                                    .lock()
                                    .unwrap()
                                    .errors
                                    .push("HTTP fixture rejected request".into());
                            }
                        }));
                    }
                    Ok(None) => {}
                    Err(_) => break,
                }
            }
            for worker in handlers {
                let _ = worker.join();
            }
        }));
        Ok(fixture)
    }
    pub fn events(&self) -> Vec<Value> {
        self.state.lock().unwrap().events.clone()
    }
    pub fn errors(&self) -> Vec<String> {
        self.state.lock().unwrap().errors.clone()
    }
    fn reply(
        &self,
        request: Request,
        index: usize,
        status: u16,
        value: Option<Value>,
        headers: &[(&str, String)],
    ) -> Result<()> {
        self.state.lock().unwrap().events[index]["status"] = json!(status);
        let body = value.map(|v| v.to_string()).unwrap_or_default();
        let mut response = Response::from_string(body)
            .with_status_code(status)
            .with_header(Header::from_bytes("Content-Type", "application/json").unwrap());
        for (key, value) in headers {
            response.add_header(
                Header::from_bytes(*key, value.as_str())
                    .map_err(|_| anyhow::anyhow!("invalid fixture header"))?,
            );
        }
        request.respond(response)?;
        Ok(())
    }
    fn dispatch(&self, mut request: Request) -> Result<()> {
        let url = url::Url::parse(&format!("{}{}", self.base, request.url()))?;
        let path = url.path().to_owned();
        let verb = request.method().as_str().to_owned();
        let header = |name: &str| {
            request
                .headers()
                .iter()
                .find(|h| h.field.as_str().as_str().eq_ignore_ascii_case(name))
                .map(|h| h.value.as_str().to_owned())
        };
        let auth = header("Authorization");
        let kind = if auth.as_deref() == Some(&format!("Bearer {}", self.token)) {
            "oauth"
        } else if auth.as_deref() == Some(&format!("Bearer {}", self.static_token)) {
            "static"
        } else if auth.is_none() {
            "absent"
        } else {
            "unexpected"
        };
        let content_type = header("Content-Type").unwrap_or_default();
        let mut event = json!({"verb":verb,"path":path,"rpc":null,"auth":kind,"session":header("Mcp-Session-Id"),"protocol":header("Mcp-Protocol-Version"),"accept":header("Accept")});
        let mut raw = vec![];
        request
            .as_reader()
            .take(1024 * 1024 + 1)
            .read_to_end(&mut raw)?;
        ensure!(raw.len() <= 1024 * 1024, "fixture body too large");
        let data: Value = if raw.is_empty() {
            json!({})
        } else if content_type.contains("application/json") {
            serde_json::from_slice(&raw)?
        } else {
            json!(
                url::form_urlencoded::parse(&raw)
                    .into_owned()
                    .collect::<HashMap<_, _>>()
            )
        };
        event["rpc"] = data["method"].clone();
        let index = {
            let mut state = self.state.lock().unwrap();
            let index = state.events.len();
            state.events.push(event);
            index
        };
        let resource = format!("{}/mcp", self.base);
        match path.as_str() {
            "/resource-metadata" => self.reply(request, index, 200, Some(json!({"resource":resource,"authorization_servers":[self.base],"scopes_supported":["tools:read"]})), &[]),
            "/.well-known/oauth-authorization-server" => self.reply(request,index,200,Some(json!({"issuer":self.base,"authorization_endpoint":format!("{}/authorize",self.base),"token_endpoint":format!("{}/token",self.base),"registration_endpoint":format!("{}/register",self.base),"response_types_supported":["code"],"grant_types_supported":["authorization_code"],"code_challenge_methods_supported":["S256"],"token_endpoint_auth_methods_supported":["none"],"authorization_response_iss_parameter_supported":true})),&[]),
            "/register" => {
                ensure!(data["token_endpoint_auth_method"] == "none", "DCR must request a public client");
                let redirect = url::Url::parse(data["redirect_uris"][0].as_str().context("missing callback")?)?;
                ensure!(redirect.host_str() == Some("127.0.0.1") && redirect.path() == "/callback", "DCR callback must be loopback");
                self.state.lock().unwrap().registration = Some(data);
                self.reply(request,index,201,Some(json!({"client_id":"fixture-client","token_endpoint_auth_method":"none"})),&[])
            }
            "/authorize" => {
                let mut query = HashMap::new();
                for (key,value) in url.query_pairs() { ensure!(query.insert(key.into_owned(), value.into_owned()).is_none(), "duplicate authorize parameter"); }
                let q = |k: &str| query.get(k).map(String::as_str).unwrap_or("");
                ensure!(q("response_type") == "code" && q("client_id") == "fixture-client", "authorize client/response mismatch");
                ensure!(q("code_challenge_method") == "S256", "PKCE S256 required");
                ensure!(q("resource") == resource && q("scope") == "tools:read", "authorize resource/scope mismatch");
                let mut state = self.state.lock().unwrap();
                ensure!(state.registration.as_ref().and_then(|r| r["redirect_uris"].as_array()).is_some_and(|r| r.iter().any(|v| v == q("redirect_uri"))), "unregistered callback");
                let mut callback = url::Url::parse(q("redirect_uri"))?;
                callback.query_pairs_mut().append_pair("code", &self.code).append_pair("state", q("state")).append_pair("iss", &self.base);
                state.authorization = Some(query);
                drop(state);
                self.reply(request,index,302,None,&[("Location",callback.into())])
            }
            "/token" => {
                let state = self.state.lock().unwrap();
                let query = state.authorization.as_ref().context("token before authorize")?;
                let verifier = data["code_verifier"].as_str().unwrap_or("");
                ensure!(Some(&URL_SAFE_NO_PAD.encode(Sha256::digest(verifier))) == query.get("code_challenge"), "PKCE verifier mismatch");
                ensure!(data["grant_type"] == "authorization_code" && data["code"] == self.code, "token grant/code mismatch");
                ensure!(data["redirect_uri"].as_str() == query.get("redirect_uri").map(String::as_str) && data["client_id"] == "fixture-client", "token callback/client mismatch");
                ensure!(data["resource"] == resource, "token resource mismatch");
                drop(state);
                self.reply(request,index,200,Some(json!({"access_token":self.token,"token_type":"Bearer","expires_in":3600,"refresh_token":secret()?,"scope":"tools:read"})),&[])
            }
            "/mcp" | "/legacy" | "/fallback" | "/messages/legacy" | "/messages/fallback" => {
                if !matches!(kind,"oauth"|"static") { return self.reply(request,index,401,None,&[("WWW-Authenticate",format!("Bearer resource_metadata=\"{}/resource-metadata\", scope=\"tools:read\"",self.base))]); }
                if path == "/fallback" && verb == "POST" { return self.reply(request,index,405,None,&[]); }
                if matches!(path.as_str(),"/legacy"|"/fallback") && verb == "GET" {
                    let (tx,rx) = mpsc::channel();
                    tx.send(format!("event: endpoint\ndata: /messages{path}\n\n").into_bytes())?;
                    { let mut state = self.state.lock().unwrap(); state.streams.insert(path,tx); state.events[index]["status"] = json!(200); }
                    let mut writer = request.into_writer();
                    writer.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n")?;
                    writer.flush()?;
                    while !self.stop.load(Ordering::SeqCst) {
                        let data = match rx.recv_timeout(Duration::from_millis(100)) {
                            Ok(data) => data,
                            Err(mpsc::RecvTimeoutError::Timeout) => b": heartbeat\n\n".to_vec(),
                            Err(_) => break,
                        };
                        if writer.write_all(&data).and_then(|_| writer.flush()).is_err() { break; }
                    }
                    return Ok(());
                }
                if verb == "DELETE" { return self.reply(request,index,204,None,&[]); }
                if verb == "GET" { return self.reply(request,index,405,None,&[]); }
                let result = match data["method"].as_str() {
                    Some("initialize") => json!({"protocolVersion":data["params"]["protocolVersion"],"capabilities":{"tools":{}},"serverInfo":{"name":"acceptance-fixture","version":"1"}}),
                    Some("tools/list") => json!({"tools":[{"name":"zeta_fixture","description":"fixture tool","inputSchema":{"type":"object"}},{"name":"alpha_fixture","description":"fixture tool","inputSchema":{"type":"object"}}]}),
                    Some("notifications/initialized") => return self.reply(request,index,202,None,&[]),
                    _ => return self.reply(request,index,400,None,&[]),
                };
                let response = json!({"jsonrpc":"2.0","id":data["id"],"result":result});
                if let Some(stream) = path.strip_prefix("/messages") {
                    self.state.lock().unwrap().streams.get(stream).context("SSE stream missing")?.send(format!("event: message\ndata: {response}\n\n").into_bytes())?;
                    self.reply(request,index,202,None,&[])
                } else {
                    let headers = if data["method"] == "initialize" { vec![("Mcp-Session-Id","fixture-session".into())] } else { vec![] };
                    self.reply(request,index,200,Some(response),&headers)
                }
            }
            _ => self.reply(request,index,404,None,&[]),
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(worker) = self.worker.take() {
            self.stop.store(true, Ordering::SeqCst);
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unauthorized_requests_record_only_auth_classification() {
        let fixture = Fixture::new().unwrap();
        let client = reqwest::blocking::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(3))
            .build()
            .unwrap();
        let response = client
            .post(format!("{}/mcp", fixture.base))
            .bearer_auth("never-record-this-token")
            .json(&json!({"method":"initialize"}))
            .send()
            .unwrap();
        assert_eq!(response.status().as_u16(), 401);
        let events = fixture.events();
        assert_eq!(events[0]["auth"], "unexpected");
        assert!(
            !serde_json::to_string(&events)
                .unwrap()
                .contains("never-record-this-token")
        );
        assert!(fixture.errors().is_empty());
    }
    #[test]
    fn registration_rejects_non_loopback_callback_without_persisting_details() {
        let fixture = Fixture::new().unwrap();
        let client = reqwest::blocking::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(3))
            .build()
            .unwrap();
        let response = client.post(format!("{}/register",fixture.base)).json(&json!({"token_endpoint_auth_method":"none","redirect_uris":["https://example.invalid/callback?secret=never-record"]})).send().unwrap();
        assert_eq!(response.status().as_u16(), 500);
        assert_eq!(fixture.errors(), vec!["HTTP fixture rejected request"]);
        assert!(
            !serde_json::to_string(&fixture.events())
                .unwrap()
                .contains("never-record")
        );
    }
}
