use super::*;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
pub(super) const FILES: [(&str, &str); 2] =
    [("openai", "openai-auth.json"), ("claude", "auth.json")];
pub(super) const CREDENTIALS: &[&str] = &[
    "openai-auth.json",
    "claude-auth.json",
    "auth.json",
    ".credentials.json",
    "openai.env",
    "anthropic.env",
    "google-tokens.json",
    "gemini-auth.json",
];
pub(super) const EXPIRES: u64 = 4102444800000;
pub(super) const MAX_PAYLOAD: usize = 65536;
pub(super) fn digest(b: &[u8]) -> String {
    format!("{:x}", Sha256::digest(b))
}
pub(super) fn token_hashes(b: &[u8]) -> Vec<String> {
    regex::bytes::Regex::new(r"NEXODE_SYNTHETIC_IMPORT_[a-z]+_[0-9a-f]{32}")
        .unwrap()
        .find_iter(b)
        .map(|m| digest(m.as_bytes()))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}
pub(super) fn fixture(provider: &str) -> Result<(Value, Value)> {
    let access = format!("NEXODE_SYNTHETIC_IMPORT_access_{}", unique()?);
    let refresh = format!("NEXODE_SYNTHETIC_IMPORT_refresh_{}", unique()?);
    let (store, credential) = match provider {
        "openai" => {
            let c = json!({"access_token":access,"refresh_token":refresh,"id_token":null,"account_id":null,"expires_at":EXPIRES});
            let mut account = c.clone();
            account["label"] = "openai-otter".into();
            (
                json!({"openai_accounts":[account],"active_openai_account":"openai-otter"}),
                c,
            )
        }
        "claude" => {
            let c = json!({"access":access,"refresh":refresh,"expires":EXPIRES,"scopes":[],"subscription_type":null});
            let mut account = c.clone();
            account["label"] = "claude-otter".into();
            (
                json!({"anthropic_accounts":[account],"active_anthropic_account":"claude-otter"}),
                c,
            )
        }
        _ => anyhow::bail!("Unknown synthetic provider"),
    };
    Ok((
        store,
        json!({"version":1,"provider":provider,"credential":credential}),
    ))
}
pub(super) fn check_payload(bytes: &[u8], provider: &str, expected: &Value) -> Result<()> {
    ensure!(
        !bytes.is_empty() && bytes.len() <= MAX_PAYLOAD,
        "Invalid synthetic payload size"
    );
    let v: Value = serde_json::from_slice(bytes)
        .map_err(|_| anyhow::anyhow!("Malformed synthetic envelope"))?;
    ensure!(
        v.as_object().is_some_and(|o| o.len() == 3)
            && v["version"] == 1
            && v["provider"] == provider,
        "Unexpected transfer envelope"
    );
    let c = &v["credential"];
    let (access, refresh, expiry, allowed) = match provider {
        "openai" => (
            "access_token",
            "refresh_token",
            "expires_at",
            vec![
                "access_token",
                "refresh_token",
                "expires_at",
                "id_token",
                "account_id",
            ],
        ),
        "claude" => (
            "access",
            "refresh",
            "expires",
            vec![
                "access",
                "refresh",
                "expires",
                "scopes",
                "subscription_type",
            ],
        ),
        _ => anyhow::bail!("Unknown synthetic provider"),
    };
    ensure!(
        c.as_object()
            .is_some_and(|o| o.keys().all(|k| allowed.contains(&k.as_str())))
            && c[expiry] == EXPIRES,
        "Unexpected synthetic fields"
    );
    let re = regex::Regex::new(r"^NEXODE_SYNTHETIC_IMPORT_[a-z]+_[0-9a-f]{32}$")?;
    for k in [access, refresh] {
        ensure!(
            c[k].as_str().is_some_and(|s| re.is_match(s)),
            "Refusing non-synthetic credentials"
        );
    }
    for k in ["id_token", "account_id", "subscription_type"] {
        ensure!(c[k].is_null(), "Unexpected credential identity");
    }
    ensure!(
        c.get("scopes").is_none_or(|s| s == &json!([])),
        "Unexpected scopes"
    );
    ensure!(
        json!(token_hashes(bytes)) == *expected,
        "Payload is not selected local synthetic provider"
    );
    Ok(())
}
pub(super) fn scan(root: &Path, allowed: &BTreeSet<String>) -> Result<Value> {
    let mut stores = serde_json::Map::new();
    for p in files(root)? {
        let relative = p.strip_prefix(root)?.to_string_lossy().into_owned();
        let b = fs::read(&p)?;
        let hashes = token_hashes(&b);
        if allowed.contains(&relative) {
            stores.insert(relative,json!({"sha256":digest(&b),"mode":fs::metadata(&p)?.permissions().mode()&0o777,"tokens":hashes}));
        } else {
            ensure!(
                hashes.is_empty(),
                "Synthetic token leaked outside exact credential stores"
            );
        }
    }
    Ok(stores.into())
}
pub(super) fn allowed() -> BTreeSet<String> {
    FILES.iter().map(|(_, f)| format!("nexode/{f}")).collect()
}
pub(super) fn local_clean(root: &Path, needles: &[String]) -> Result<()> {
    for p in files(root)? {
        let relative = p.strip_prefix(root)?;
        ensure!(
            !relative
                .components()
                .any(|c| c.as_os_str() == "pending-login")
                && !CREDENTIALS.contains(&p.file_name().unwrap().to_string_lossy().as_ref()),
            "Remote login created local auth material"
        );
        let b = fs::read(p)?;
        ensure!(
            !needles
                .iter()
                .any(|n| !n.is_empty() && b.windows(n.len()).any(|b| b == n.as_bytes())),
            "Sensitive callback leaked into local file"
        );
    }
    Ok(())
}
pub(super) fn callback_valid(payload: &[u8], states: &[String]) -> Result<()> {
    ensure!(
        !payload.is_empty() && payload.len() <= 16384,
        "Invalid callback size"
    );
    let s =
        std::str::from_utf8(payload).map_err(|_| anyhow::anyhow!("Invalid callback encoding"))?;
    let u = url::Url::parse(s.trim()).map_err(|_| anyhow::anyhow!("Invalid callback URL"))?;
    let query: Vec<_> = u.query_pairs().collect();
    let state: Vec<_> = query
        .iter()
        .filter(|(k, _)| k == "state")
        .map(|(_, v)| v.as_ref())
        .collect();
    let code: Vec<_> = query
        .iter()
        .filter(|(k, _)| k == "code")
        .map(|(_, v)| v.as_ref())
        .collect();
    ensure!(
        u.scheme() == "http"
            && matches!(u.host_str(), Some("localhost" | "127.0.0.1"))
            && u.path() == "/auth/callback"
            && state.len() == 1
            && code.len() == 1
            && code[0].starts_with("NEXODE_SSH_LOGIN_INVALID_")
            && !states.is_empty()
            && !states.iter().any(|s| s == state[0]),
        "Only mismatched synthetic localhost callbacks are permitted"
    );
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn payload_selected_synthetic_and_bounded() {
        for (p, _) in FILES {
            let (_, v) = fixture(p).unwrap();
            let b = serde_json::to_vec(&v).unwrap();
            let e = json!(token_hashes(&b));
            check_payload(&b, p, &e).unwrap();
            assert!(check_payload(&b, p, &json!(["wrong"])).is_err());
            assert!(check_payload(&vec![b' '; MAX_PAYLOAD + 1], p, &e).is_err());
            let mut bad = v;
            let key = if p == "openai" {
                "access_token"
            } else {
                "access"
            };
            bad["credential"][key] = "real-credential-refused".into();
            assert!(check_payload(&serde_json::to_vec(&bad).unwrap(), p, &e).is_err());
        }
    }
    #[test]
    fn exact_store_allowlist() {
        let d = tempfile::tempdir().unwrap();
        init_root(d.path()).unwrap();
        let (s, _) = fixture("openai").unwrap();
        let b = serde_json::to_vec(&s).unwrap();
        private_write(&d.path().join("nexode/openai-auth.json"), &b).unwrap();
        let stores = scan(d.path(), &allowed()).unwrap();
        assert_eq!(stores["nexode/openai-auth.json"]["mode"], 384);
        fs::write(d.path().join("lookalike-openai-auth.json"), b).unwrap();
        assert!(scan(d.path(), &allowed()).is_err());
    }
    #[test]
    fn callback_guard() {
        let states = vec!["remote".into()];
        callback_valid(
            b"http://localhost:1455/auth/callback?code=NEXODE_SSH_LOGIN_INVALID_test&state=other",
            &states,
        )
        .unwrap();
        for b in [
            b"http://localhost:1455/auth/callback?code=NEXODE_SSH_LOGIN_INVALID_test&state=remote"
                .as_slice(),
            b"https://attacker.invalid/auth/callback?code=real&state=other",
            b"http://localhost/auth/callback?code=NEXODE_SSH_LOGIN_INVALID_test&state=a&state=b",
        ] {
            assert!(callback_valid(b, &states).is_err());
        }
    }
}
