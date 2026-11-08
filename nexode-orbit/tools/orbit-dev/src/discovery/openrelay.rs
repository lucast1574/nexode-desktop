//! Local-only Discovery browse/select fixture; never contacts its listed service.
use anyhow::{Result, ensure};
use clap::Parser;
use serde_json::{Value, json};
use std::{collections::HashMap, fs, io::Write, path::PathBuf};
use tiny_http::{Method, Server};
const CATEGORIES: &[&str] = &[
    "payments",
    "code-review",
    "databases",
    "browser-automation",
    "deployment",
    "observability",
    "authentication",
    "security",
    "storage",
    "analytics",
    "web-search",
    "web-data",
    "financial-data",
    "cloud-infrastructure",
    "compliance-and-privacy",
    "integration-platforms",
    "email-messaging",
    "ai-models",
    "other",
];
fn response(path: &str) -> Result<(u16, Value)> {
    let url = url::Url::parse(&format!("http://localhost{path}"))?;
    if !matches!(url.path(), "/" | "/v1/discovery") {
        return Ok((
            404,
            json!({"error":{"code":"not_found","message":"not found"}}),
        ));
    }
    let q: HashMap<_, _> = url.query_pairs().into_owned().collect();
    let category = q
        .get("category")
        .map(|s| s.trim().to_lowercase())
        .unwrap_or_default();
    let selected = q
        .get("tool")
        .map(|s| s.trim().to_lowercase())
        .unwrap_or_default();
    let event = json!({"event":"discovery_request","phase":if selected.is_empty(){"browse"}else{"select"},"category":category,"tool":if selected.is_empty(){None}else{Some(&selected)},"query":q.get("q").map(|s|s.trim()).unwrap_or(""),"reason":q.get("reason").map(|s|s.trim()).unwrap_or("")});
    println!("{event}");
    std::io::stdout().flush()?;
    if !CATEGORIES.contains(&category.as_str()) {
        return Ok((
            400,
            json!({"error":{"code":"unknown_category","message":"unknown Discovery category"}}),
        ));
    }
    let mut payload = json!({"category":category,"policy":"test discovery fixture: local evaluation only, not a production listing","details":"https://openrelay.xyz"});
    let mut tool = json!({"name":"openrelay-rivet","blurb":"public Ethereum Classic JSON-RPC endpoint operated by OpenRelay","url":"https://rivet.link/networks?via=nexode-discovery-test"});
    if selected.is_empty() {
        payload["tools"] = if category == "cloud-infrastructure" {
            json!([tool])
        } else {
            json!([])
        };
    } else {
        if category != "cloud-infrastructure" || selected != "openrelay-rivet" {
            return Ok((
                404,
                json!({"error":{"code":"unknown_tool","message":format!("no tool '{selected}' in category '{category}'")}}),
            ));
        }
        tool["setup"] = json!(include_str!("openrelay-setup.txt"));
        payload["tool"] = tool;
    }
    Ok((200, payload))
}
#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "127.0.0.1")]
    host: String,
    #[arg(long, default_value_t = 0)]
    port: u16,
    #[arg(long)]
    ready_file: Option<PathBuf>,
}
pub fn run(args: impl Iterator<Item = String>) -> Result<()> {
    let args = Args::parse_from(std::iter::once("openrelay-discovery-server".into()).chain(args));
    let server = Server::http((args.host.as_str(), args.port))
        .map_err(|e| anyhow::anyhow!("discovery bind: {e}"))?;
    let addr = server
        .server_addr()
        .to_ip()
        .ok_or_else(|| anyhow::anyhow!("IP required"))?;
    let ready = json!({"host":addr.ip().to_string(),"port":addr.port(),"endpoint":format!("http://{addr}/v1/discovery")});
    if let Some(file) = args.ready_file {
        fs::write(file, format!("{ready}\n"))?;
    }
    let mut event = ready;
    event["event"] = json!("ready");
    println!("{event}");
    std::io::stdout().flush()?;
    for r in server.incoming_requests() {
        let (code, body) = if r.method() == &Method::Get {
            response(r.url())?
        } else {
            (405, json!({"error":"method not allowed"}))
        };
        super::sponsor::reply(r, code, body)?;
    }
    ensure!(false, "discovery server stopped");
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn browse_select_and_rejections() {
        let (status, b) = response("/v1/discovery?category=cloud-infrastructure").unwrap();
        assert_eq!(status, 200);
        assert!(b["tools"][0].get("setup").is_none());
        let (_, b) =
            response("/v1/discovery?category=cloud-infrastructure&tool=openrelay-rivet").unwrap();
        let setup = b["tool"]["setup"].as_str().unwrap();
        for marker in [
            "Setup is complete with this Discovery response",
            "https://etc.rivet.link/",
            "eth_getBlockByNumber",
            "No account",
        ] {
            assert!(setup.contains(marker));
        }
        assert!(!setup.contains("<access_token>"));
        assert_eq!(
            response("/?category=ai-models").unwrap().1["tools"],
            json!([])
        );
        assert_eq!(response("/?category=bad").unwrap().0, 400);
        assert_eq!(
            response("/?category=ai-models&tool=openrelay-rivet")
                .unwrap()
                .0,
            404
        );
        assert_eq!(response("/bad").unwrap().0, 404);
    }
}
