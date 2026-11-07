//! Optional legacy local pairing endpoint. No remote host or AWS account is embedded.
use anyhow::{Context, Result, ensure};
use clap::Parser;
use serde_json::{Value, json};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};
use subtle::ConstantTimeEq;
#[derive(Parser)]
struct Args {
    #[arg(long)]
    host: Option<String>,
    #[arg(long, default_value_t = 7643)]
    port: u16,
    #[arg(long, default_value = "127.0.0.1:7644")]
    bind: String,
    #[arg(long, default_value = "/etc/nexode-pair-token")]
    token_file: PathBuf,
    #[arg(long, default_value = "nexode")]
    binary: PathBuf,
    #[arg(long)]
    run_as: Option<String>,
}
fn read_token(path: &Path) -> Result<String> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(path).context("open pairing token file")?;
    let meta = file.metadata()?;
    ensure!(
        meta.is_file() && meta.len() <= 16384,
        "token must be a bounded regular file"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        ensure!(
            meta.permissions().mode() & 0o077 == 0,
            "pairing token file must be owner-only (0600)"
        );
    }
    let mut token = String::new();
    file.take(16385).read_to_string(&mut token)?;
    ensure!(
        token.len() <= 16384 && !token.trim().is_empty(),
        "empty or oversized token"
    );
    Ok(token.trim().into())
}
fn authorize(target: &str, bearer: Option<&str>, expected: &str) -> bool {
    let candidate = if let Some(bearer) = bearer {
        bearer.strip_prefix("Bearer ").map(str::to_owned)
    } else {
        url::Url::parse(&format!("http://localhost{target}"))
            .ok()
            .and_then(|u| {
                let mut values = u
                    .query_pairs()
                    .filter(|(k, _)| k == "t")
                    .map(|(_, v)| v.into_owned());
                let first = values.next();
                if values.next().is_some() { None } else { first }
            })
    };
    candidate.is_some_and(|v| bool::from(v.as_bytes().ct_eq(expected.as_bytes())))
}
fn pairing(text: &str, host: &str, port: u16) -> Result<Value> {
    let clean = regex::Regex::new(r"\x1b\[[0-9;]*m")?.replace_all(text, "");
    let re = regex::Regex::new(r"Pairing code:\s*([0-9]{3})\s*([0-9]{3})\b")?;
    let capture = re
        .captures(&clean)
        .context("no pairing code in command output")?;
    let code = format!("{}{}", &capture[1], &capture[2]);
    let mut uri = url::Url::parse("nexode://pair")?;
    uri.query_pairs_mut().extend_pairs([
        ("host", host),
        ("port", &port.to_string()),
        ("code", &code),
    ]);
    Ok(json!({"code":code,"host":host,"port":port,"uri":uri.as_str(),"expires_in":300}))
}
fn generate(a: &Args, host: &str) -> Result<Value> {
    let mut command = if let Some(user) = &a.run_as {
        ensure!(
            !user.is_empty() && !user.starts_with('-'),
            "invalid run-as user"
        );
        let mut c = Command::new("sudo");
        c.args(["-n", "-u", user, "-i", "--"]).arg(&a.binary);
        c
    } else {
        Command::new(&a.binary)
    };
    command.arg("pair").env("NEXODE_GATEWAY_HOST", host);
    let result =
        crate::process::capture_interruptible(&mut command, Duration::from_secs(30), interrupted)?;
    ensure!(result.status.success(), "pair command failed");
    pairing(
        &(String::from_utf8_lossy(&result.stdout).into_owned()
            + &String::from_utf8_lossy(&result.stderr)),
        host,
        a.port,
    )
}
fn interrupted() -> bool {
    #[cfg(unix)]
    {
        crate::ci_suites::unix::interrupted().is_some()
    }
    #[cfg(not(unix))]
    {
        false
    }
}
fn handle(
    method: &str,
    target: &str,
    bearer: Option<&str>,
    token: &str,
    generate: impl FnOnce() -> Result<Value>,
) -> (u16, Value) {
    if !authorize(target, bearer, token) {
        return (403, json!({"error":"forbidden"}));
    }
    if method != "GET" {
        return (405, json!({"error":"method not allowed"}));
    }
    let path = target.split('?').next().unwrap_or("");
    if path != "/pair-code" {
        return (404, json!({"error":"not found"}));
    }
    match generate() {
        Ok(v) => (200, v),
        Err(_) => (500, json!({"error":"pairing code generation failed"})),
    }
}
pub fn run(args: impl Iterator<Item = String>) -> Result<()> {
    let a = Args::parse_from(std::iter::once("pair-service".into()).chain(args));
    let host = a
        .host
        .clone()
        .or_else(|| std::env::var("NEXODE_GATEWAY_HOST").ok())
        .context("--host or NEXODE_GATEWAY_HOST required")?;
    ensure!(
        !host.is_empty() && !host.chars().any(char::is_whitespace) && a.port > 0,
        "invalid advertised host/port"
    );
    let token = read_token(&a.token_file)?;
    let server =
        tiny_http::Server::http(&a.bind).map_err(|e| anyhow::anyhow!("pair service bind: {e}"))?;
    #[cfg(unix)]
    let _signals = crate::ci_suites::unix::Signals::install()?;
    eprintln!("pair service listening on {}", server.server_addr());
    while !interrupted() {
        if let Some(req) = server.recv_timeout(Duration::from_millis(200))? {
            let bearer = req
                .headers()
                .iter()
                .find(|h| h.field.equiv("Authorization"))
                .map(|h| h.value.as_str());
            let (status, body) = handle(req.method().as_str(), req.url(), bearer, &token, || {
                generate(&a, &host)
            });
            let response = tiny_http::Response::from_string(body.to_string())
                .with_status_code(status)
                .with_header(
                    tiny_http::Header::from_bytes("Content-Type", "application/json").unwrap(),
                )
                .with_header(tiny_http::Header::from_bytes("Cache-Control", "no-store").unwrap());
            let _ = req.respond(response);
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn denied_or_wrong_routes_never_generate_and_errors_hide_process_output() {
        let forbidden = || -> Result<Value> { panic!("must not generate") };
        assert_eq!(
            handle("GET", "/pair-code?t=wrong", None, "secret", forbidden).0,
            403
        );
        assert_eq!(
            handle("GET", "/other?t=secret", None, "secret", forbidden).0,
            404
        );
        assert_eq!(
            handle(
                "POST",
                "/pair-code",
                Some("Bearer secret"),
                "secret",
                forbidden
            )
            .0,
            405
        );
        assert!(!authorize("/pair-code?t=secret&t=secret", None, "secret"));
        let (status, v) = handle("GET", "/pair-code", Some("Bearer secret"), "secret", || {
            anyhow::bail!("sensitive process output")
        });
        assert_eq!(status, 500);
        assert!(!v.to_string().contains("sensitive"));
    }
    #[test]
    fn ansi_code_and_encoded_uri_preserve_pairing_contract() {
        let v = pairing("\x1b[32mPairing code: 123 456\x1b[0m", "host.example", 7643).unwrap();
        assert_eq!(v["code"], "123456");
        let u = url::Url::parse(v["uri"].as_str().unwrap()).unwrap();
        assert!(
            u.query_pairs()
                .any(|(k, v)| k == "host" && v == "host.example")
        );
        assert_eq!(v["expires_in"], 300);
        assert!(pairing("Pairing code: 1234567", "localhost", 7643).is_err());
    }
    #[test]
    fn empty_or_public_token_is_rejected() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("token");
        fs::write(&p, "").unwrap();
        assert!(read_token(&p).is_err());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::write(&p, "test-secret").unwrap();
            fs::set_permissions(&p, fs::Permissions::from_mode(0o600)).unwrap();
            assert_eq!(read_token(&p).unwrap(), "test-secret");
            fs::set_permissions(&p, fs::Permissions::from_mode(0o644)).unwrap();
            assert!(read_token(&p).is_err());
        }
    }
}
