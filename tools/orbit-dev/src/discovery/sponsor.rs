//! Reference sponsor: signed one-use signup state and immutable SQLite attribution.
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use clap::{Parser, Subcommand};
use hmac::{Hmac, Mac};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tiny_http::{Header, Method, Request, Response, Server};
const SECRET: &str = "mock-sponsor-development-secret";
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn encode(payload: &Value, secret: &[u8]) -> Result<String> {
    let body = URL_SAFE_NO_PAD.encode(serde_json::to_vec(payload)?);
    let mut mac = Hmac::<Sha256>::new_from_slice(secret)?;
    mac.update(body.as_bytes());
    Ok(format!(
        "{body}.{}",
        URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
    ))
}
fn decode(token: &str, secret: &[u8], time: u64) -> Result<Value> {
    let parsed = (|| -> Result<Value> {
        let (body, signature) = token.split_once('.').context("missing signature")?;
        let mut mac = Hmac::<Sha256>::new_from_slice(secret)?;
        mac.update(body.as_bytes());
        mac.verify_slice(&URL_SAFE_NO_PAD.decode(signature)?)?;
        Ok(serde_json::from_slice(&URL_SAFE_NO_PAD.decode(body)?)?)
    })()
    .map_err(|_| anyhow::anyhow!("invalid signup token"))?;
    ensure!(
        parsed["exp"].as_u64().unwrap_or(0) >= time,
        "expired signup token"
    );
    Ok(parsed)
}
fn db(path: &Path) -> Result<Connection> {
    let c = Connection::open(path)?;
    c.busy_timeout(Duration::from_secs(5))?;
    c.execute_batch("CREATE TABLE IF NOT EXISTS accounts (email TEXT PRIMARY KEY, acquisition_source TEXT, created_at INTEGER NOT NULL); CREATE TABLE IF NOT EXISTS consumed_tokens (nonce TEXT PRIMARY KEY, consumed_at INTEGER NOT NULL);")?;
    Ok(c)
}
fn confirm(path: &Path, p: &Value) -> Result<Value> {
    let mut c = db(path)?;
    let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let nonce = p["nonce"].as_str().context("missing nonce")?;
    let consumed: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM consumed_tokens WHERE nonce=?1)",
        [nonce],
        |r| r.get(0),
    )?;
    ensure!(!consumed, "signup token already consumed");
    let email = p["email"].as_str().context("missing email")?;
    tx.execute(
        "INSERT OR IGNORE INTO accounts(email,acquisition_source,created_at) VALUES (?1,?2,?3)",
        params![email, p["via"].as_str(), now()],
    )?;
    tx.execute(
        "INSERT INTO consumed_tokens(nonce,consumed_at) VALUES (?1,?2)",
        params![nonce, now()],
    )?;
    tx.commit()?;
    Ok(json!({"email":email,"confirmed":true}))
}
pub(super) fn reply(request: Request, status: u16, payload: Value) -> Result<()> {
    request.respond(
        Response::from_string(payload.to_string())
            .with_status_code(status)
            .with_header(
                Header::from_bytes("Content-Type", "application/json")
                    .map_err(|_| anyhow::anyhow!("header"))?,
            ),
    )?;
    Ok(())
}
fn value_text(v: &Value) -> String {
    v.as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| v.to_string())
}
fn handle(
    mut r: Request,
    db_path: &Path,
    secret: &[u8],
    base: &str,
    executable: &Path,
) -> Result<()> {
    let url = url::Url::parse(&format!("{base}{}", r.url()))?;
    let query: HashMap<_, _> = url.query_pairs().into_owned().collect();
    let result = (|| -> Result<(u16, Value)> {
        if r.method() == &Method::Post {
            if url.path() != "/v1/signup" {
                return Ok((404, json!({"error":"not found"})));
            }
            let mut bytes = Vec::new();
            r.as_reader().take(65537).read_to_end(&mut bytes)?;
            ensure!(bytes.len() <= 65536, "signup request too large");
            let input: Value = serde_json::from_slice(&bytes)?;
            let email = value_text(input.get("email").context("missing email")?)
                .trim()
                .to_lowercase();
            ensure!(email.contains('@'), "invalid email");
            let source = input
                .get("via")
                .filter(|v| !v.is_null())
                .map(value_text)
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty());
            ensure!(
                source
                    .as_ref()
                    .is_none_or(|s| s.len() <= 100 && s.is_ascii()),
                "invalid attribution source"
            );
            let nonce = format!(
                "{:x}",
                Sha256::digest(format!("{email}:{:?}", SystemTime::now()).as_bytes())
            );
            let token = encode(
                &json!({"email":email,"via":source,"nonce":nonce,"exp":now()+300}),
                secret,
            )?;
            return Ok((
                200,
                json!({"magic_link":format!("{base}/v1/confirm?token={token}")}),
            ));
        }
        if r.method() != &Method::Get {
            return Ok((405, json!({"error":"method not allowed"})));
        }
        match url.path() {
            "/" => Ok((200, json!({"product":"mock-sponsor"}))),
            "/health" => Ok((200, json!({"ok":true}))),
            "/v1/discovery" => {
                let mut listing = json!({"name":"mock-sponsor","blurb":"Reference virtual payment service for attribution validation","url":format!("{base}/?via=nexode-discovery")});
                if query.get("tool").is_some_and(|t| t == "mock-sponsor") {
                    let exe = executable.to_string_lossy().replace('\'', "'\\''");
                    listing["setup"] = json!(format!(
                        "Run `'{exe}' mock-sponsor signup --service {base} --email <email> --via nexode-discovery`, then confirm the returned magic link."
                    ));
                    Ok((200, json!({"tool":listing})))
                } else {
                    Ok((200, json!({"tools":[listing]})))
                }
            }
            "/v1/confirm" => Ok((
                200,
                confirm(
                    db_path,
                    &decode(query.get("token").context("missing token")?, secret, now())?,
                )?,
            )),
            "/v1/account" => {
                let email = query
                    .get("email")
                    .cloned()
                    .unwrap_or_default()
                    .to_lowercase();
                let row:Option<Value>=db(db_path)?.query_row("SELECT email,acquisition_source,created_at FROM accounts WHERE email=?1",[email],|row|Ok(json!({"email":row.get::<_,String>(0)?,"acquisition_source":row.get::<_,Option<String>>(1)?,"created_at":row.get::<_,u64>(2)?}))).optional()?;
                Ok(row
                    .map(|v| (200, v))
                    .unwrap_or((404, json!({"error":"account not found"}))))
            }
            _ => Ok((404, json!({"error":"not found"}))),
        }
    })();
    let (status, body) = result.unwrap_or_else(|e| (400, json!({"error":e.to_string()})));
    reply(r, status, body)
}
#[derive(Parser)]
struct Args {
    #[command(subcommand)]
    command: Action,
}
#[derive(Subcommand)]
enum Action {
    Serve {
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        #[arg(long, default_value_t = 0)]
        port: u16,
        #[arg(long)]
        db: PathBuf,
        #[arg(long,default_value=SECRET)]
        secret: String,
        #[arg(long)]
        ready_file: Option<PathBuf>,
    },
    Signup {
        #[arg(long)]
        service: String,
        #[arg(long)]
        email: String,
        #[arg(long)]
        via: Option<String>,
    },
    Confirm {
        magic_link: String,
    },
    Account {
        #[arg(long)]
        service: String,
        #[arg(long)]
        email: String,
    },
}
fn request(url: &str, body: Option<Value>) -> Result<Value> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()?;
    let response = match body {
        Some(v) => client.post(url).json(&v).send()?,
        None => client.get(url).send()?,
    };
    let status = response.status();
    let value: Value = response.json()?;
    ensure!(status.is_success(), "HTTP {status}: {value}");
    Ok(value)
}
pub fn run(args: impl Iterator<Item = String>) -> Result<()> {
    let args = Args::parse_from(std::iter::once("mock-sponsor".into()).chain(args));
    match args.command {
        Action::Serve {
            host,
            port,
            db,
            secret,
            ready_file,
        } => {
            let server = Server::http((host.as_str(), port))
                .map_err(|e| anyhow::anyhow!("sponsor bind: {e}"))?;
            let base = format!("http://{}", server.server_addr());
            if let Some(path) = ready_file {
                fs::write(path, json!({"url":base}).to_string())?;
            }
            println!("{base}");
            std::io::stdout().flush()?;
            let exe = std::env::current_exe()?;
            for r in server.incoming_requests() {
                if let Err(e) = handle(r, &db, secret.as_bytes(), &base, &exe) {
                    eprintln!("sponsor fixture: {e:#}");
                }
            }
        }
        Action::Signup {
            service,
            email,
            via,
        } => println!(
            "{}",
            request(
                &format!("{}/v1/signup", service.trim_end_matches('/')),
                Some(json!({"email":email,"via":via}))
            )?["magic_link"]
                .as_str()
                .context("missing magic link")?
        ),
        Action::Confirm { magic_link } => println!("{}", request(&magic_link, None)?),
        Action::Account { service, email } => {
            let mut url =
                url::Url::parse(&format!("{}/v1/account", service.trim_end_matches('/')))?;
            url.query_pairs_mut().append_pair("email", &email);
            println!("{}", request(url.as_str(), None)?);
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn signatures_expiry_and_replay_preserve_first_attribution() {
        let token = encode(
            &json!({"email":"a@b","via":"first","nonce":"one","exp":10}),
            b"secret",
        )
        .unwrap();
        let p = decode(&token, b"secret", 10).unwrap();
        assert!(
            decode(&token, b"wrong", 10)
                .unwrap_err()
                .to_string()
                .contains("invalid signup token")
        );
        assert!(
            decode(&token, b"secret", 11)
                .unwrap_err()
                .to_string()
                .contains("expired")
        );
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        confirm(&path, &p).unwrap();
        assert!(
            confirm(&path, &p)
                .unwrap_err()
                .to_string()
                .contains("already consumed")
        );
        confirm(&path, &json!({"email":"a@b","via":"second","nonce":"two"})).unwrap();
        let source: String = db(&path)
            .unwrap()
            .query_row("SELECT acquisition_source FROM accounts", [], |r| r.get(0))
            .unwrap();
        assert_eq!(source, "first");
    }
}
