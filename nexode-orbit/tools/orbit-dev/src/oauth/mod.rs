//! Manual OAuth completion and explicit provider diagnostics, with no Python.
mod probes;
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use clap::Parser;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};
const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const AUTHORIZE: &str = "https://claude.ai/oauth/authorize";
const TOKEN: &str = "https://console.anthropic.com/v1/oauth/token";
const REDIRECT: &str = "https://console.anthropic.com/oauth/code/callback";
fn random<const N: usize>() -> Result<[u8; N]> {
    let mut bytes = [0; N];
    getrandom::fill(&mut bytes).map_err(|_| anyhow::anyhow!("OS randomness unavailable"))?;
    Ok(bytes)
}
pub(crate) fn private_write(path: &Path, value: &Value) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    if !parent.exists() {
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(parent)?;
    }
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.write_all(serde_json::to_string_pretty(value)?.as_bytes())?;
    file.write_all(b"\n")?;
    file.flush()?;
    file.persist(path).map_err(|e| e.error)?;
    Ok(())
}
fn begin(path: &Path, now: i64) -> Result<String> {
    let verifier = URL_SAFE_NO_PAD.encode(random::<48>()?);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let state = random::<16>()?
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let mut url = url::Url::parse(AUTHORIZE)?;
    url.query_pairs_mut().extend_pairs([
        ("response_type", "code"),
        ("client_id", CLIENT_ID),
        ("redirect_uri", REDIRECT),
        ("scope", "org:create_api_key user:profile user:inference"),
        ("code_challenge", &challenge),
        ("code_challenge_method", "S256"),
        ("state", &state),
    ]);
    private_write(
        path,
        &json!({"verifier":verifier,"challenge":challenge,"state":state,"auth_url":url.as_str(),"created_at":now}),
    )?;
    Ok(url.into())
}
fn code(input: &str) -> Result<(String, Option<String>)> {
    let input = input.trim();
    ensure!(!input.is_empty(), "authorization code required");
    if let Ok(url) = url::Url::parse(input)
        && matches!(url.scheme(), "http" | "https")
    {
        let mut code = None;
        let mut state = None;
        for (k, v) in url.query_pairs() {
            if k == "code" {
                ensure!(code.is_none(), "duplicate callback code");
                code = Some(v.into_owned());
            } else if k == "state" {
                ensure!(state.is_none(), "duplicate callback state");
                state = Some(v.into_owned());
            }
        }
        return Ok((
            code.filter(|s| !s.is_empty())
                .context("callback has no code")?,
            state,
        ));
    }
    if let Some((code, state)) = input.split_once('#') {
        ensure!(!code.is_empty() && !state.is_empty(), "invalid code#state");
        return Ok((code.into(), Some(state.into())));
    }
    Ok((input.into(), None))
}
fn exchange(
    state_path: &Path,
    credentials: &Path,
    input: &str,
    endpoint: &str,
    now: i64,
) -> Result<()> {
    let meta = fs::symlink_metadata(state_path)?;
    ensure!(
        meta.is_file() && !meta.is_symlink(),
        "OAuth state must be regular file"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        ensure!(
            meta.permissions().mode() & 0o077 == 0 && meta.uid() == unsafe { libc::geteuid() },
            "OAuth state must be private and owned by this user"
        );
    }
    let saved: Value = serde_json::from_slice(&fs::read(state_path)?)?;
    let created = saved["created_at"]
        .as_i64()
        .context("start a new OAuth flow")?;
    ensure!(
        created <= now && now - created <= 900,
        "OAuth state expired; start again"
    );
    let expected = saved["state"].as_str().context("state missing")?;
    let verifier = saved["verifier"].as_str().context("verifier missing")?;
    let (code, state) = code(input)?;
    ensure!(
        state.as_deref().is_none_or(|s| s == expected),
        "OAuth state mismatch; use latest callback"
    );
    let mut creds = if credentials.exists() {
        let value: Value = serde_json::from_slice(&fs::read(credentials)?)?;
        ensure!(
            value.is_object(),
            "existing credential file must be JSON object"
        );
        value
    } else {
        json!({})
    };
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()?;
    let response=client.post(endpoint).json(&json!({"grant_type":"authorization_code","client_id":CLIENT_ID,"code":code,"code_verifier":verifier,"redirect_uri":REDIRECT,"state":expected})).send().map_err(|e|e.without_url())?;
    ensure!(
        response.status().is_success(),
        "OAuth exchange failed: HTTP {}",
        response.status()
    );
    let mut bytes = vec![];
    response.take(1024 * 1024 + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 1024 * 1024, "OAuth response too large");
    let tokens: Value = serde_json::from_slice(&bytes).context("invalid OAuth token response")?;
    let access = tokens["access_token"]
        .as_str()
        .filter(|s| !s.is_empty())
        .context("missing access token")?;
    let refresh = tokens["refresh_token"]
        .as_str()
        .filter(|s| !s.is_empty())
        .context("missing refresh token")?;
    let duration = tokens["expires_in"]
        .as_i64()
        .filter(|n| *n > 0)
        .context("invalid token lifetime")?;
    let expires = now
        .checked_add(duration)
        .and_then(|n| n.checked_mul(1000))
        .context("token expiration overflow")?;
    creds["claudeAiOauth"] =
        json!({"accessToken":access,"refreshToken":refresh,"expiresAt":expires});
    private_write(credentials, &creds)?;
    fs::remove_file(state_path)?;
    Ok(())
}
#[derive(Parser)]
struct Args {
    #[arg(long)]
    exchange: bool,
    code: Option<String>,
    #[arg(long)]
    state_file: Option<PathBuf>,
    #[arg(long)]
    credentials: Option<PathBuf>,
}
pub fn run(command: &str, args: impl Iterator<Item = String>) -> Result<()> {
    if command == "oauth-probe" {
        return probes::run(args);
    }
    let a = Args::parse_from(std::iter::once("oauth-helper".into()).chain(args));
    let home = dirs::home_dir().context("home directory unavailable")?;
    let nexode = std::env::var_os("NEXODE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".nexode"));
    let path = a
        .state_file
        .unwrap_or_else(|| nexode.join("oauth-helper.json"));
    let now = chrono::Utc::now().timestamp();
    if a.exchange {
        let input = if let Some(code) = a.code {
            code
        } else {
            eprint!("Enter authorization code or callback URL: ");
            std::io::stderr().flush()?;
            let mut line = String::new();
            std::io::stdin().read_line(&mut line)?;
            line
        };
        let credentials = a
            .credentials
            .unwrap_or_else(|| home.join(".claude/.credentials.json"));
        exchange(&path, &credentials, &input, TOKEN, now)?;
        println!("Credentials saved to {}", credentials.display());
    } else {
        ensure!(a.code.is_none(), "code requires --exchange");
        println!("Auth URL: {}", begin(&path, now)?);
        println!("Private pending state saved to {}", path.display());
    }
    Ok(())
}
#[cfg(test)]
mod tests;
