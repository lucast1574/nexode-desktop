//! Native Lambda wake/pair and circuit-breaker handlers; no AWS CLI dependency.
mod aws;
use anyhow::{Context, Result, ensure};
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use serde_json::{Value, json};
use std::{fs, path::PathBuf, time::Duration};
use subtle::ConstantTimeEq;
trait Backend {
    fn state(&mut self) -> Result<String>;
    fn start(&mut self) -> Result<()>;
    fn stop(&mut self) -> Result<()>;
    fn online(&mut self) -> Result<bool>;
    fn pair(&mut self) -> Result<String>;
    fn notify(&mut self, message: &str) -> Result<()>;
}
struct Config {
    token: String,
    host: String,
    port: u16,
    instance: String,
}
fn response(status: u16, body: String, content_type: &str) -> Value {
    json!({"statusCode":status,"body":body,"headers":{"Cache-Control":"no-store","Content-Type":content_type,"Referrer-Policy":"no-referrer","X-Content-Type-Options":"nosniff","X-Frame-Options":"DENY"}})
}
fn json_response(status: u16, value: Value) -> Value {
    response(status, value.to_string(), "application/json")
}
fn equal(a: &str, b: &str) -> bool {
    bool::from(a.as_bytes().ct_eq(b.as_bytes()))
}
fn pairing(text: &str, c: &Config) -> Result<Value> {
    let clean = regex::Regex::new(r"\x1b\[[0-9;]*m")?.replace_all(text, "");
    let regex = regex::Regex::new(r"Pairing code:\s+(\d{3})\s+(\d{3})")?;
    let m = regex
        .captures(&clean)
        .context("no pairing code in command output")?;
    let code = format!("{}{}", &m[1], &m[2]);
    let mut uri = url::Url::parse("nexode://pair")?;
    uri.query_pairs_mut()
        .append_pair("host", &c.host)
        .append_pair("port", &c.port.to_string())
        .append_pair("code", &code);
    Ok(json!({"code":code,"host":c.host,"port":c.port,"uri":uri.as_str(),"expires_in":300}))
}
fn landing() -> Result<Value> {
    let mut bytes = [0u8; 18];
    getrandom::fill(&mut bytes).map_err(|_| anyhow::anyhow!("nonce generation failed"))?;
    let nonce = URL_SAFE_NO_PAD.encode(bytes);
    let mut out = response(
        200,
        include_str!("landing.html").replace("__NONCE__", &nonce),
        "text/html; charset=utf-8",
    );
    out["headers"]["Content-Security-Policy"] = json!(format!(
        "default-src 'none'; style-src 'nonce-{nonce}'; script-src 'nonce-{nonce}'; connect-src 'self'; base-uri 'none'; frame-ancestors 'none'"
    ));
    Ok(out)
}
fn wake(event: &Value, c: &Config, backend: &mut impl Backend) -> Result<Value> {
    let method = event["requestContext"]["http"]["method"]
        .as_str()
        .or_else(|| event["httpMethod"].as_str())
        .unwrap_or("GET")
        .to_uppercase();
    if method == "GET" {
        if event["queryStringParameters"]["t"]
            .as_str()
            .is_some_and(|t| equal(t, &c.token))
        {
            let mut out = response(302, "".into(), "application/json");
            let encoded =
                url::form_urlencoded::byte_serialize(c.token.as_bytes()).collect::<String>();
            out["headers"]["Location"] = json!(format!("/#t={encoded}"));
            return Ok(out);
        }
        return landing();
    }
    if method != "POST" {
        return Ok(json_response(405, json!({"error":"method not allowed"})));
    }
    let auth = event["headers"]
        .as_object()
        .and_then(|h| {
            h.iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("authorization"))
        })
        .and_then(|(_, v)| v.as_str())
        .unwrap_or("");
    if !equal(auth, &format!("Bearer {}", c.token)) {
        return Ok(json_response(403, json!({"error":"forbidden"})));
    }
    let raw = event["body"].as_str().unwrap_or("{}");
    let raw = if event["isBase64Encoded"] == true {
        match STANDARD.decode(raw) {
            Ok(v) => v,
            Err(_) => return Ok(json_response(400, json!({"error":"invalid body"}))),
        }
    } else {
        raw.as_bytes().to_vec()
    };
    let body: Value = match serde_json::from_slice(&raw) {
        Ok(v) => v,
        Err(_) => return Ok(json_response(400, json!({"error":"invalid JSON"}))),
    };
    let action = body["action"].as_str().unwrap_or("");
    if !matches!(action, "wake" | "status" | "pair") {
        return Ok(json_response(400, json!({"error":"unknown action"})));
    }
    let state = backend.state()?;
    match action {
        "wake" => {
            let started = state == "stopped";
            if started {
                backend.start()?;
            }
            Ok(json_response(200, json!({"state":state,"started":started})))
        }
        "status" => Ok(json_response(
            200,
            json!({"state":state,"healthy":state=="running"&&backend.online()?}),
        )),
        _ => {
            if state != "running" {
                return Ok(json_response(
                    409,
                    json!({"error":format!("instance {state}, wake it first")}),
                ));
            }
            if !backend.online()? {
                return Ok(json_response(
                    503,
                    json!({"error":"instance management agent is not ready"}),
                ));
            }
            match backend.pair().and_then(|s| pairing(&s, c)) {
                Ok(v) => Ok(json_response(200, v)),
                Err(_) => Ok(json_response(500, json!({"error":"pair command failed"}))),
            }
        }
    }
}
fn breaker(c: &Config, backend: &mut impl Backend) -> Result<Value> {
    let state = backend.state()?;
    let msg = if state == "running" {
        backend.stop()?;
        format!("Circuit breaker: stopped {} (was {state})", c.instance)
    } else {
        format!("Circuit breaker: {} already {state}", c.instance)
    };
    backend.notify(&msg)?;
    Ok(json!({"message":msg}))
}
fn runtime(endpoint: &str, mut handler: impl FnMut(Value) -> Result<Value>) -> Result<()> {
    ensure!(
        !endpoint.contains(['/', '?', '#', '@']),
        "invalid Lambda runtime authority"
    );
    let client = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let root = format!("http://{endpoint}/2018-06-01/runtime");
    loop {
        let response = client
            .get(format!("{root}/invocation/next"))
            .send()?
            .error_for_status()?;
        let id = response
            .headers()
            .get("lambda-runtime-aws-request-id")
            .context("Lambda request id missing")?
            .to_str()?
            .to_owned();
        ensure!(
            !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'),
            "invalid Lambda request id"
        );
        let result = response
            .json()
            .map_err(anyhow::Error::from)
            .and_then(&mut handler);
        let (route, body) = match result {
            Ok(v) => ("response", v),
            Err(_) => (
                "error",
                json!({"errorMessage":"Native phone handler failed; inspect service permissions and configuration","errorType":"OrbitPhoneError"}),
            ),
        };
        client
            .post(format!("{root}/invocation/{id}/{route}"))
            .timeout(Duration::from_secs(10))
            .json(&body)
            .send()?
            .error_for_status()?;
    }
}
pub fn run(args: impl Iterator<Item = String>) -> Result<()> {
    use clap::Parser;
    #[derive(Parser)]
    struct Args {
        #[arg(value_parser=["wake","breaker"])]
        mode: String,
        #[arg(long)]
        event: Option<PathBuf>,
    }
    let a = Args::parse_from(std::iter::once("phone-lambda".into()).chain(args));
    let env = |k: &str| std::env::var(k).with_context(|| format!("{k} is required"));
    let token = if a.mode == "wake" {
        env("WAKE_TOKEN")?
    } else {
        String::new()
    };
    ensure!(
        a.mode != "wake" || (!token.is_empty() && token != "REPLACE_WITH_WAKE_TOKEN"),
        "configure a nonempty wake token"
    );
    let c = Config {
        token,
        host: std::env::var("NEXODE_GATEWAY_HOST").unwrap_or_default(),
        port: std::env::var("NEXODE_GATEWAY_PORT")
            .unwrap_or_else(|_| "7643".into())
            .parse()?,
        instance: env("INSTANCE_ID")?,
    };
    if a.mode == "wake" {
        ensure!(
            !c.host.is_empty() && c.port > 0,
            "gateway host and port required"
        );
    }
    let region = env("AWS_REGION")?;
    ensure!(
        region
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-'),
        "invalid AWS region"
    );
    let mut backend = aws::Aws {
        region,
        instance: c.instance.clone(),
        host: c.host.clone(),
        command: std::env::var("NEXODE_PAIR_COMMAND")
            .unwrap_or_else(|_| "/home/ec2-user/.local/bin/nexode pair".into()),
        topic: if a.mode == "breaker" {
            Some(env("SNS_TOPIC_ARN")?)
        } else {
            None
        },
        client: reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .build()?,
    };
    let handler = |event| {
        if a.mode == "wake" {
            wake(&event, &c, &mut backend)
        } else {
            breaker(&c, &mut backend)
        }
    };
    if let Some(path) = a.event {
        let mut handler = handler;
        println!("{}", handler(serde_json::from_slice(&fs::read(path)?)?)?);
        Ok(())
    } else {
        runtime(&env("AWS_LAMBDA_RUNTIME_API")?, handler)
    }
}
#[cfg(test)]
mod tests;

pub fn package(args: impl Iterator<Item = String>) -> Result<()> {
    use std::io::Write;
    let args: Vec<_> = args.collect();
    ensure!(
        args.len() == 2,
        "usage: package-phone-lambda <static-linux-binary> <zip>"
    );
    let bytes = fs::read(&args[0])?;
    ensure!(
        bytes.starts_with(b"\x7fELF"),
        "Lambda requires a Linux ELF binary"
    );
    let mut archive = zip::ZipWriter::new(fs::File::create(&args[1])?);
    archive.start_file(
        "bootstrap",
        zip::write::SimpleFileOptions::default()
            .unix_permissions(0o755)
            .compression_method(zip::CompressionMethod::Deflated),
    )?;
    archive.write_all(&bytes)?;
    archive.finish()?;
    Ok(())
}
