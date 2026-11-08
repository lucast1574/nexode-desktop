use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
pub(super) fn values(task: &str, phase: &str) -> Vec<&'static str> {
    match (task, phase) {
        ("navigation", "heldout") => vec!["Storage guide", "Backup examples"],
        ("navigation", _) => vec!["Browser guide", "Navigation examples"],
        ("search", "heldout") => vec!["maple", "supplies"],
        ("search", _) => vec!["cedar", "tools"],
        ("form", "heldout") => vec![
            "Robin Sample",
            "robin@example.invalid",
            "Please reserve the green sample.",
        ],
        _ => vec![
            "Alex Example",
            "alex@example.invalid",
            "Please reserve the blue sample.",
        ],
    }
}
pub(super) fn prompt(task: &str, phase: &str) -> String {
    let v = values(task, phase);
    match task {
        "navigation" => format!(
            "Navigate through Browse guides to {}, then {}, then View navigation receipt.",
            v[0], v[1]
        ),
        "search" => format!(
            "Search the catalog for {}, filter Category to {}, submit Search, open the matching item, then Specifications, then View navigation receipt.",
            v[0], v[1]
        ),
        _ => format!(
            "Open Sample request. Fill Name with {:?}, Email with {:?}, and Notes with {:?}. Select Delivery as pickup, check I confirm, then submit Request sample and open View navigation receipt. This is a synthetic localhost form with no external writes.",
            v[0], v[1], v[2]
        ),
    }
}
fn token() -> Result<String> {
    let mut b = [0u8; 12];
    getrandom::fill(&mut b).map_err(|e| anyhow::anyhow!("random token: {e}"))?;
    Ok(b.iter().map(|b| format!("{b:02x}")).collect())
}
pub(super) struct Fixture {
    pub origin: String,
    pub start: Instant,
    states: Arc<Mutex<BTreeMap<String, Value>>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Fixture {
    pub fn new() -> Result<Self> {
        let server = tiny_http::Server::http(("127.0.0.1", 0))
            .map_err(|e| anyhow::anyhow!("fixture bind: {e}"))?;
        let addr = server.server_addr().to_ip().context("fixture address")?;
        let states = Arc::new(Mutex::new(BTreeMap::new()));
        let shared = states.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let start = Instant::now();
        let thread = std::thread::spawn(move || {
            while !stopped.load(Ordering::Relaxed) {
                if let Ok(Some(mut req)) = server.recv_timeout(Duration::from_millis(50)) {
                    let post = req.method() == &tiny_http::Method::Post;
                    let url = url::Url::parse(&format!("http://localhost{}", req.url()));
                    let mut body = String::new();
                    let valid_length = req.body_length().unwrap_or(0) <= 16384;
                    if post && valid_length {
                        use std::io::Read;
                        let _ = req.as_reader().take(16385).read_to_string(&mut body);
                    }
                    let result = url.ok().and_then(|url| {
                        let parts: Vec<_> = url.path().trim_matches('/').split('/').collect();
                        if parts.len() != 2 || !valid_length || body.len() > 16384 {
                            return None;
                        }
                        let mut states = shared.lock().ok()?;
                        let state = states.get_mut(parts[0])?;
                        let fields = form(if post {
                            &body
                        } else {
                            url.query().unwrap_or("")
                        });
                        page(
                            state,
                            parts[0],
                            parts[1],
                            post,
                            fields,
                            start.elapsed().as_secs_f64(),
                        )
                    });
                    let status = if result.is_some() { 200 } else { 404 };
                    let text = format!(
                        "<!doctype html><meta charset=\"utf-8\"><title>Nexode isolated browser fixture</title>{}",
                        result.unwrap_or_else(|| "Not found".into())
                    );
                    let response = tiny_http::Response::from_string(text)
                        .with_status_code(status)
                        .with_header(
                            tiny_http::Header::from_bytes(
                                "Content-Type",
                                "text/html; charset=utf-8",
                            )
                            .unwrap(),
                        )
                        .with_header(
                            tiny_http::Header::from_bytes("Cache-Control", "no-store").unwrap(),
                        );
                    let _ = req.respond(response);
                }
            }
        });
        Ok(Self {
            origin: format!("http://{addr}"),
            start,
            states,
            stop,
            thread: Some(thread),
        })
    }
    pub fn add(&self, task: &str, phase: &str) -> Result<(String, String)> {
        let token = token()?;
        let state = json!({"task":task,"phase":phase,"values":values(task,phase),"search_valid":false,"form_valid":false,"receipt":&self::token()?[..10],"requests":[],"receipt_page_served":false,"confirmation_dom_observed":false});
        self.states.lock().unwrap().insert(token.clone(), state);
        let url = format!("{}/{token}/start", self.origin);
        Ok((token, url))
    }
    pub fn state(&self, token: &str) -> Value {
        self.states.lock().unwrap()[token].clone()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
fn form(text: &str) -> Value {
    let mut fields = serde_json::Map::new();
    for (k, v) in url::form_urlencoded::parse(text.as_bytes()) {
        fields
            .entry(k.into_owned())
            .or_insert(json!([]))
            .as_array_mut()
            .unwrap()
            .push(json!(v));
    }
    fields.into()
}
fn page(
    state: &mut Value,
    token: &str,
    step: &str,
    post: bool,
    fields: Value,
    elapsed: f64,
) -> Option<String> {
    state["requests"]
        .as_array_mut()?
        .push(json!({"step":step,"method":if post{"POST"}else{"GET"},"fields":fields}));
    let task = state["task"].as_str()?.to_string();
    let v: Vec<_> = state["values"]
        .as_array()?
        .iter()
        .map(|s| s.as_str().unwrap().to_string())
        .collect();
    let root = format!("/{token}");
    Some(match (task.as_str(), step, post) {
        ("search", "start", false) => format!(
            r#"<h1>Catalog</h1><form action="{root}/results"><label>Search catalog <input name="q"></label><label>Category <select name="category"><option>all</option><option>tools</option><option>supplies</option></select></label><button>Search</button></form>"#
        ),
        ("search", "results", false) => {
            state["search_valid"] = json!(fields == json!({"q":[v[0]],"category":[v[1]]}));
            if state["search_valid"] == true {
                format!(
                    r#"<h1>Results</h1><a href="{root}/item">{} {}</a>"#,
                    v[0], v[1]
                )
            } else {
                "<h1>No matching results</h1>".into()
            }
        }
        ("search", "item", false) if state["search_valid"] == true => {
            format!(r#"<h1>Item details</h1><a href="{root}/navigation">Specifications</a>"#)
        }
        ("form", "start", false) => {
            format!(r#"<h1>Samples</h1><a href="{root}/request">Sample request</a>"#)
        }
        ("form", "request", false) => format!(
            r#"<h1>Sample request</h1><form method="post" action="{root}/submit"><label>Name <input name="name" required></label><label>Email <input name="email" type="email" required></label><label>Notes <textarea name="notes"></textarea></label><label>Delivery <select name="delivery"><option>mail</option><option>pickup</option></select></label><label><input type="checkbox" name="confirm" value="yes">I confirm</label><button>Request sample</button></form>"#
        ),
        ("form", "submit", true) => {
            state["form_valid"] = json!(
                fields
                    == json!({"name":[v[0]],"email":[v[1]],"notes":[v[2]],"delivery":["pickup"],"confirm":["yes"]})
            );
            if state["form_valid"] == true {
                format!(r#"<a href="{root}/receipt">View navigation receipt</a>"#)
            } else {
                "<h1>Invalid sample request</h1>".into()
            }
        }
        (_, "start", false) => {
            format!(r#"<h1>Nexode documentation</h1><a href="{root}/guides">Browse guides</a>"#)
        }
        (_, "guides", false) => format!(r#"<h1>Guides</h1><a href="{root}/browser">{}</a>"#, v[0]),
        (_, "browser", false) => format!(
            r#"<h1>{}</h1><a href="{root}/navigation">{}</a>"#,
            v[0], v[1]
        ),
        (_, "navigation", false) => format!(
            r#"<h1>Navigation examples</h1><a href="{root}/receipt">View navigation receipt</a>"#
        ),
        (_, "receipt", false)
            if task == "navigation"
                || state["search_valid"] == true
                || state["form_valid"] == true =>
        {
            state["receipt_page_served"] = json!(true);
            format!(
                r##"<h1 id="result">Navigation complete</h1><p>Receipt: {}</p><script>requestAnimationFrame(() => {{if (document.querySelector('#result').textContent === 'Navigation complete') fetch('{root}/visible', {{method:'POST'}});}});addEventListener('pagehide', () => navigator.sendBeacon('{root}/left', ''));</script>"##,
                state["receipt"].as_str()?
            )
        }
        (_, "visible", true) => {
            state["confirmation_dom_observed"] = json!(true);
            state["visible_at"] = json!(elapsed);
            "ok".into()
        }
        (_, "left", true) => {
            state["confirmation_dom_observed"] = json!(false);
            "ok".into()
        }
        _ => return None,
    })
}
pub(super) fn outcome(state: &Value) -> bool {
    let expected: &[&str] = match state["task"].as_str() {
        Some("navigation") => &["start", "guides", "browser", "navigation", "receipt"],
        Some("search") => &["start", "results", "item", "navigation", "receipt"],
        _ => &["start", "request", "submit", "receipt"],
    };
    let mut cursor = 0;
    for req in state["requests"].as_array().into_iter().flatten() {
        if cursor < expected.len() && req["step"] == expected[cursor] {
            cursor += 1
        }
    }
    cursor == expected.len()
        && state["receipt_page_served"] == true
        && (state["task"] == "navigation"
            || state["task"] == "search" && state["search_valid"] == true
            || state["task"] == "form" && state["form_valid"] == true)
}
