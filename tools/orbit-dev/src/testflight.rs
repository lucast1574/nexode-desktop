//! App Store Connect setup, with explicit account configuration and ES256 JWTs.
use anyhow::{Context, Result, ensure};
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use clap::Parser;
use ring::{
    rand::SystemRandom,
    signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair},
};
use serde_json::{Value, json};
use std::{fs, path::PathBuf, time::Duration};
#[derive(Parser)]
struct Args {
    #[arg(long)]
    key_id: String,
    #[arg(long)]
    issuer: String,
    #[arg(long)]
    key_path: PathBuf,
    #[arg(long, default_value = "com.nexode.mobile")]
    bundle_id: String,
    #[arg(long)]
    tester_email: String,
    #[arg(long)]
    tester_name: String,
    #[arg(long, default_value = "internal")]
    group: String,
}
fn jwt(pem: &str, key_id: &str, issuer: &str, now: i64) -> Result<String> {
    let data = pem
        .lines()
        .filter(|l| !l.starts_with("-----"))
        .collect::<String>();
    let key = EcdsaKeyPair::from_pkcs8(
        &ECDSA_P256_SHA256_FIXED_SIGNING,
        &STANDARD.decode(data)?,
        &SystemRandom::new(),
    )
    .map_err(|_| anyhow::anyhow!("invalid P-256 PKCS8 key"))?;
    let head = URL_SAFE_NO_PAD.encode(serde_json::to_vec(
        &json!({"alg":"ES256","kid":key_id,"typ":"JWT"}),
    )?);
    let body = URL_SAFE_NO_PAD.encode(serde_json::to_vec(
        &json!({"iss":issuer,"iat":now,"exp":now+1200,"aud":"appstoreconnect-v1"}),
    )?);
    let input = format!("{head}.{body}");
    let signature = key
        .sign(&SystemRandom::new(), input.as_bytes())
        .map_err(|_| anyhow::anyhow!("JWT signing failed"))?;
    Ok(format!(
        "{input}.{}",
        URL_SAFE_NO_PAD.encode(signature.as_ref())
    ))
}
trait Api {
    fn request(
        &mut self,
        method: &str,
        path: &str,
        query: &[(&str, &str)],
        body: Option<Value>,
    ) -> Result<Value>;
}
struct Http {
    client: reqwest::blocking::Client,
    key: String,
    key_id: String,
    issuer: String,
}
impl Api for Http {
    fn request(
        &mut self,
        method: &str,
        path: &str,
        query: &[(&str, &str)],
        body: Option<Value>,
    ) -> Result<Value> {
        let mut req = self
            .client
            .request(
                method.parse()?,
                format!("https://api.appstoreconnect.apple.com/v1{path}"),
            )
            .query(query)
            .bearer_auth(jwt(
                &self.key,
                &self.key_id,
                &self.issuer,
                chrono::Utc::now().timestamp(),
            )?);
        if let Some(body) = body {
            req = req.json(&body);
        }
        let response = req.send().map_err(|e| e.without_url())?;
        ensure!(
            response.status().is_success(),
            "App Store Connect {method} {path} returned HTTP {}",
            response.status()
        );
        let text = response.text()?;
        if text.trim().is_empty() {
            Ok(json!({}))
        } else {
            Ok(serde_json::from_str(&text)?)
        }
    }
}
fn first(v: &Value) -> Result<Option<String>> {
    let list = v["data"].as_array().context("API list missing data")?;
    list.first()
        .map(|x| {
            x["id"]
                .as_str()
                .map(str::to_owned)
                .context("API object missing id")
        })
        .transpose()
}
fn id(v: &Value) -> Result<String> {
    Ok(v["data"]["id"]
        .as_str()
        .context("created object missing id")?
        .into())
}
fn setup(api: &mut impl Api, a: &Args) -> Result<Value> {
    let app = first(&api.request("GET", "/apps", &[("filter[bundleId]", &a.bundle_id)], None)?)?
        .context("app not found")?;
    let builds = api.request(
        "GET",
        "/builds",
        &[
            ("filter[app]", &app),
            ("sort", "-uploadedDate"),
            ("limit", "1"),
        ],
        None,
    )?;
    let build = first(&builds)?.context("no builds available")?;
    let group=match first(&api.request("GET","/betaGroups",&[("filter[app]",&app),("filter[name]",&a.group)],None)?)?{
        Some(id)=>id,
        None=>id(&api.request("POST","/betaGroups",&[],Some(json!({"data":{"type":"betaGroups","attributes":{"name":a.group,"isInternalGroup":true},"relationships":{"app":{"data":{"type":"apps","id":app}}}}})))?)?,
    };
    let tester=match first(&api.request("GET","/betaTesters",&[("filter[email]",&a.tester_email)],None)?)?{
        Some(id)=>id,
        None=>id(&api.request("POST","/betaTesters",&[],Some(json!({"data":{"type":"betaTesters","attributes":{"email":a.tester_email,"firstName":a.tester_name},"relationships":{"betaGroups":{"data":[{"type":"betaGroups","id":group}]}}}})))?)?,
    };
    api.request(
        "POST",
        &format!("/betaGroups/{group}/relationships/betaTesters"),
        &[],
        Some(json!({"data":[{"type":"betaTesters","id":tester}]})),
    )?;
    api.request(
        "POST",
        &format!("/betaGroups/{group}/relationships/builds"),
        &[],
        Some(json!({"data":[{"type":"builds","id":build}]})),
    )?;
    Ok(json!({"app":app,"build":build,"group":group,"tester":tester,"assigned":true}))
}
pub fn run(args: impl Iterator<Item = String>) -> Result<()> {
    let a = Args::parse_from(std::iter::once("testflight-setup".into()).chain(args));
    let mut api = Http {
        client: reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .build()?,
        key: fs::read_to_string(&a.key_path)?,
        key_id: a.key_id.clone(),
        issuer: a.issuer.clone(),
    };
    println!("{}", serde_json::to_string_pretty(&setup(&mut api, &a)?)?);
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use ring::signature::{ECDSA_P256_SHA256_FIXED, KeyPair, UnparsedPublicKey};
    #[test]
    fn signed_token_verifies_and_has_bounded_lifetime() {
        let rng = SystemRandom::new();
        let doc = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).unwrap();
        let key =
            EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, doc.as_ref(), &rng).unwrap();
        let pem = format!(
            "-----BEGIN PRIVATE KEY-----\n{}\n-----END PRIVATE KEY-----",
            STANDARD.encode(doc.as_ref())
        );
        let token = jwt(&pem, "fixture", "issuer", 100).unwrap();
        let parts: Vec<_> = token.split('.').collect();
        UnparsedPublicKey::new(&ECDSA_P256_SHA256_FIXED, key.public_key().as_ref())
            .verify(
                format!("{}.{}", parts[0], parts[1]).as_bytes(),
                &URL_SAFE_NO_PAD.decode(parts[2]).unwrap(),
            )
            .unwrap();
        let claims: Value =
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts[1]).unwrap()).unwrap();
        assert_eq!(claims["exp"], 1300);
        assert_eq!(claims["aud"], "appstoreconnect-v1");
        assert!(jwt("bad key", "id", "issuer", 100).is_err());
    }
    struct Fake {
        calls: Vec<String>,
        existing: bool,
        fail: bool,
    }
    impl Api for Fake {
        fn request(
            &mut self,
            method: &str,
            path: &str,
            _: &[(&str, &str)],
            body: Option<Value>,
        ) -> Result<Value> {
            self.calls.push(format!("{method} {path}"));
            if self.fail && path == "/betaGroups" {
                anyhow::bail!("HTTP 403");
            }
            if method == "GET" {
                let found = self.existing || matches!(path, "/apps" | "/builds");
                return Ok(if found {
                    json!({"data":[{"id":path.trim_start_matches('/')} ]})
                } else {
                    json!({"data":[]})
                });
            }
            assert!(body.is_some());
            Ok(json!({"data":{"id":path.trim_start_matches('/')}}))
        }
    }
    #[test]
    fn existing_resources_are_reused_and_lookup_failure_never_creates() {
        let a = Args {
            key_id: "id".into(),
            issuer: "issuer".into(),
            key_path: "unused".into(),
            bundle_id: "app".into(),
            tester_email: "fixture@example.test".into(),
            tester_name: "Fixture".into(),
            group: "internal".into(),
        };
        for existing in [false, true] {
            let mut f = Fake {
                calls: vec![],
                existing,
                fail: false,
            };
            assert_eq!(setup(&mut f, &a).unwrap()["assigned"], true);
            assert_eq!(f.calls.contains(&"POST /betaGroups".into()), !existing);
            assert!(f.calls.last().unwrap().ends_with("relationships/builds"));
        }
        let mut f = Fake {
            calls: vec![],
            existing: false,
            fail: true,
        };
        assert!(setup(&mut f, &a).is_err());
        assert!(f.calls.iter().all(|c| c.starts_with("GET")));
    }
}
