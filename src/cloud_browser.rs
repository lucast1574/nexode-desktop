//! Nexode browser approval: PKCE binds an authorization to this desktop.
//! Google authentication stays on the existing trusted Nexode browser origin.
use super::{Outcome, account, client, connector_secret};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use zeroize::Zeroizing;

pub struct BrowserLogin {
    pub verification_url: String,
    pub user_code: String,
    pub committing: bool,
    pub(super) device_code: Zeroizing<String>,
    pub(super) verifier: Zeroizing<String>,
    pub(super) expires_at: Instant,
    pub(super) next_poll: Instant,
    pub(super) opened: bool,
}
fn challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}
pub(super) fn begin() -> Result<BrowserLogin, String> {
    let mut entropy = Zeroizing::new([0_u8; 32]);
    getrandom::fill(entropy.as_mut())
        .map_err(|_| "No se pudo generar una autorización segura.".to_string())?;
    let verifier = Zeroizing::new(URL_SAFE_NO_PAD.encode(entropy.as_ref()));
    let data = match request(
        "mutation($challenge:String!){startDesktopConnection(challenge:$challenge){deviceCode userCode verificationUrl expiresAtMs intervalSeconds}}",
        json!({"challenge":challenge(&verifier)}),
    )? {
        Reply::Data(data) => data,
        Reply::Retry(_) => {
            return Err("Hay demasiados intentos. Espera un minuto y vuelve a conectar.".into());
        }
    };
    let started = &data["startDesktopConnection"];
    let url = started["verificationUrl"]
        .as_str()
        .filter(|url| valid_url(url))
        .ok_or("Nexode devolvió una página de autorización no válida.")?;
    let code = started["userCode"]
        .as_str()
        .filter(|code| valid_code(code))
        .ok_or("Código de autorización inválido.")?;
    let device = started["deviceCode"]
        .as_str()
        .filter(|value| valid_device_code(value))
        .ok_or("Autorización de dispositivo inválida.")?;
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as f64;
    let remaining_ms = started["expiresAtMs"]
        .as_f64()
        .filter(|value| value.is_finite())
        .ok_or("Autorización sin fecha de vencimiento.")?
        - now_ms;
    if !(1.0..=600_000.0).contains(&remaining_ms) {
        return Err("La autorización venció o el reloj del equipo no está sincronizado.".into());
    }
    Ok(BrowserLogin {
        verification_url: url.into(),
        user_code: code.into(),
        device_code: Zeroizing::new(device.into()),
        verifier,
        expires_at: Instant::now() + Duration::from_millis(remaining_ms.min(300_000.0) as u64),
        next_poll: Instant::now() + Duration::from_secs(5),
        opened: false,
        committing: false,
    })
}
pub(super) fn exchange(device: &str, verifier: &str) -> Result<Outcome, String> {
    let data = match request(
        "mutation($deviceCode:String!,$verifier:String!){exchangeDesktopConnection(deviceCode:$deviceCode,verifier:$verifier){status connector}}",
        json!({"deviceCode":device,"verifier":verifier}),
    )? {
        Reply::Data(data) => data,
        Reply::Retry(delay) => return Ok(Outcome::BrowserPending(delay)),
    };
    let reply = &data["exchangeDesktopConnection"];
    match reply["status"].as_str() {
        Some("pending") => Ok(Outcome::BrowserPending(Duration::from_secs(5))),
        Some("approved") => {
            let key = connector_secret(
                reply["connector"]
                    .as_str()
                    .ok_or("Conector de cuenta no disponible.")?,
            )?;
            let info = account(&key)?;
            Ok(Outcome::BrowserApproved(info, key))
        }
        _ => Err("Nexode no confirmó la autorización del dispositivo.".into()),
    }
}
enum Reply {
    Data(serde_json::Value),
    Retry(Duration),
}
fn retry_delay(value: Option<&str>) -> Duration {
    Duration::from_secs(
        value
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(10)
            .clamp(5, 60),
    )
}
fn request(query: &str, variables: serde_json::Value) -> Result<Reply, String> {
    let response = client(false)?
        .post(super::GRAPHQL)
        .json(&json!({"query":query,"variables":variables}))
        .send()
        .map_err(|_| "No se pudo contactar con Nexode.".to_string())?;
    let delay = [
        "retry-after",
        "retry-after-auth-tight",
        "retry-after-sdk-public",
    ]
    .into_iter()
    .filter_map(|name| {
        response
            .headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
    })
    .map(|value| retry_delay(Some(value)))
    .max()
    .unwrap_or_else(|| retry_delay(None));
    if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return Ok(Reply::Retry(delay));
    }
    let value = super::response(response)?;
    if let Some(errors) = value["errors"].as_array() {
        if errors.iter().any(is_rate_limited) {
            return Ok(Reply::Retry(delay));
        }
        return Err("No se pudo completar la autorización. Comprueba el código y vuelve a iniciar si venció.".into());
    }
    value
        .get("data")
        .cloned()
        .map(Reply::Data)
        .ok_or("Respuesta de autorización incompleta.".into())
}
fn is_rate_limited(error: &serde_json::Value) -> bool {
    error.get("code").and_then(serde_json::Value::as_str) == Some("Too Many Requests")
        || error.get("message").and_then(serde_json::Value::as_str)
            == Some("ThrottlerException: Too Many Requests")
        || error
            .pointer("/extensions/originalError/statusCode")
            .and_then(serde_json::Value::as_u64)
            == Some(429)
        || error
            .pointer("/extensions/response/statusCode")
            .and_then(serde_json::Value::as_u64)
            == Some(429)
}
fn valid_device_code(value: &str) -> bool {
    value.len() == 43
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}
fn valid_code(value: &str) -> bool {
    value.len() == 11
        && value.bytes().enumerate().all(|(index, byte)| {
            if index == 5 {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit() && !byte.is_ascii_lowercase()
            }
        })
}
fn valid_url(value: &str) -> bool {
    reqwest::Url::parse(value).is_ok_and(|url| {
        let pairs: Vec<_> = url.query_pairs().collect();
        url.scheme() == "https"
            && url.host_str() == Some("cloud.nexode.app")
            && url.port_or_known_default() == Some(443)
            && url.username().is_empty()
            && url.password().is_none()
            && url.path() == "/desktop/connect"
            && url.fragment().is_none()
            && pairs.len() == 1
            && pairs[0].0 == "request"
            && pairs[0].1.len() == 24
            && pairs[0]
                .1
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pkce_matches_rfc7636_and_checks_only_the_official_public_request_url() {
        assert_eq!(retry_delay(Some("999999")).as_secs(), 60);
        assert_eq!(retry_delay(Some("0")).as_secs(), 5);
        assert_eq!(retry_delay(None).as_secs(), 10);
        assert_eq!(
            challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
        assert!(valid_url(
            "https://cloud.nexode.app/desktop/connect?request=abcdefghijklmnopqrstuvwx"
        ));
        for value in [
            "http://cloud.nexode.app/desktop/connect?request=abcdefghijklmnopqrstuvwx",
            "https://cloud.nexode.app.evil/desktop/connect?request=abcdefghijklmnopqrstuvwx",
            "https://cloud.nexode.app/desktop/connect?request=abcdefghijklmnopqrstuvwx&key=secret",
            "https://cloud.nexode.app/desktop/connect?request=short",
            "https://cloud.nexode.app/desktop/connect?request=abcdefghijklmnopqrstuvwx#token",
        ] {
            assert!(!valid_url(value));
        }
        assert!(valid_code("ABCDE-12345"));
        assert!(!valid_code("ABCDE-1234\n"));
    }
    #[test]
    fn retries_both_formatted_and_legacy_graphql_throttling_without_confusing_auth_failure() {
        assert!(is_rate_limited(
            &json!({"code":"Too Many Requests","message":"Too many desktop authorization requests."})
        ));
        assert!(is_rate_limited(
            &json!({"message":"ThrottlerException: Too Many Requests","extensions":{"code":"INTERNAL_SERVER_ERROR"}})
        ));
        assert!(is_rate_limited(
            &json!({"extensions":{"originalError":{"statusCode":429}}})
        ));
        assert!(!is_rate_limited(
            &json!({"code":"Unauthorized","message":"Invalid account"})
        ));
    }
}
