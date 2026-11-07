//! Minimal native AWS Query/JSON clients. Lambda supplies rotating role credentials.
use super::*;
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn hmac(key: &[u8], body: &[u8]) -> Vec<u8> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(body);
    mac.finalize().into_bytes().to_vec()
}
fn signed_headers(
    service: &str,
    region: &str,
    body: &str,
    content_type: &str,
    target: Option<&str>,
    date: &str,
    credentials: (&str, &str, Option<&str>),
) -> BTreeMap<String, String> {
    let (access, secret, token) = credentials;
    let mut headers = BTreeMap::from([
        ("host".into(), format!("{service}.{region}.amazonaws.com")),
        ("content-type".into(), content_type.into()),
        ("x-amz-date".into(), date.into()),
    ]);
    if let Some(t) = target {
        headers.insert("x-amz-target".into(), t.into());
    }
    if let Some(t) = token {
        headers.insert("x-amz-security-token".into(), t.into());
    }
    let names = headers.keys().cloned().collect::<Vec<_>>().join(";");
    let canonical_headers = headers
        .iter()
        .map(|(k, v)| {
            format!(
                "{k}:{}\n",
                v.split_whitespace().collect::<Vec<_>>().join(" ")
            )
        })
        .collect::<String>();
    let canonical = format!(
        "POST\n/\n\n{canonical_headers}\n{names}\n{}",
        hex(&Sha256::digest(body.as_bytes()))
    );
    let day = &date[..8];
    let scope = format!("{day}/{region}/{service}/aws4_request");
    let string = format!(
        "AWS4-HMAC-SHA256\n{date}\n{scope}\n{}",
        hex(&Sha256::digest(canonical.as_bytes()))
    );
    let key = hmac(format!("AWS4{secret}").as_bytes(), day.as_bytes());
    let key = hmac(&key, region.as_bytes());
    let key = hmac(&key, service.as_bytes());
    let key = hmac(&key, b"aws4_request");
    headers.insert(
        "authorization".into(),
        format!(
            "AWS4-HMAC-SHA256 Credential={access}/{scope}, SignedHeaders={names}, Signature={}",
            hex(&hmac(&key, string.as_bytes()))
        ),
    );
    headers
}
pub(super) struct Aws {
    pub region: String,
    pub instance: String,
    pub host: String,
    pub command: String,
    pub topic: Option<String>,
    pub client: reqwest::blocking::Client,
}
impl Aws {
    fn call(&self, service: &str, body: String, target: Option<&str>) -> Result<String> {
        let access =
            std::env::var("AWS_ACCESS_KEY_ID").context("AWS role access key unavailable")?;
        let secret =
            std::env::var("AWS_SECRET_ACCESS_KEY").context("AWS role secret unavailable")?;
        let token = std::env::var("AWS_SESSION_TOKEN").ok();
        let date = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
        let content = if service == "ssm" {
            "application/x-amz-json-1.1"
        } else {
            "application/x-www-form-urlencoded; charset=utf-8"
        };
        let headers = signed_headers(
            service,
            &self.region,
            &body,
            content,
            target,
            &date,
            (&access, &secret, token.as_deref()),
        );
        let mut req = self
            .client
            .post(format!("https://{service}.{}.amazonaws.com/", self.region));
        for (k, v) in headers {
            req = req.header(k, v);
        }
        let response = req.body(body).send().map_err(|e| e.without_url())?;
        let status = response.status();
        let text = response.text()?;
        if !status.is_success() {
            let code = serde_json::from_str::<Value>(&text)
                .ok()
                .and_then(|v| v["__type"].as_str().map(str::to_owned))
                .unwrap_or_default();
            ensure!(false, "AWS {service} HTTP {status} {code}");
        }
        Ok(text)
    }
    fn query(
        &self,
        service: &str,
        action: &str,
        version: &str,
        params: &[(&str, &str)],
    ) -> Result<String> {
        let mut form = url::form_urlencoded::Serializer::new(String::new());
        form.append_pair("Action", action)
            .append_pair("Version", version);
        for (k, v) in params {
            form.append_pair(k, v);
        }
        self.call(service, form.finish(), None)
    }
    fn ssm(&self, action: &str, body: Value) -> Result<Value> {
        Ok(serde_json::from_str(&self.call(
            "ssm",
            body.to_string(),
            Some(&format!("AmazonSSM.{action}")),
        )?)?)
    }
}
impl Backend for Aws {
    fn state(&mut self) -> Result<String> {
        let text = self.query(
            "ec2",
            "DescribeInstances",
            "2016-11-15",
            &[("InstanceId.1", &self.instance)],
        )?;
        let doc = roxmltree::Document::parse(&text)?;
        let state = doc
            .descendants()
            .find(|n| n.has_tag_name("instanceState"))
            .context("EC2 response missing state")?;
        Ok(state
            .children()
            .find(|n| n.has_tag_name("name"))
            .and_then(|n| n.text())
            .context("EC2 state name missing")?
            .into())
    }
    fn start(&mut self) -> Result<()> {
        self.query(
            "ec2",
            "StartInstances",
            "2016-11-15",
            &[("InstanceId.1", &self.instance)],
        )?;
        Ok(())
    }
    fn stop(&mut self) -> Result<()> {
        self.query(
            "ec2",
            "StopInstances",
            "2016-11-15",
            &[("InstanceId.1", &self.instance)],
        )?;
        Ok(())
    }
    fn online(&mut self) -> Result<bool> {
        let v = self.ssm(
            "DescribeInstanceInformation",
            json!({"Filters":[{"Key":"InstanceIds","Values":[self.instance]}],"MaxResults":5}),
        )?;
        Ok(v["InstanceInformationList"]
            .as_array()
            .context("SSM information missing")?
            .iter()
            .any(|x| x["PingStatus"] == "Online"))
    }
    fn pair(&mut self) -> Result<String> {
        let command = format!(
            "sudo -iu ec2-user env NEXODE_GATEWAY_HOST={} {}",
            shlex::try_quote(&self.host)?,
            self.command
        );
        let v=self.ssm("SendCommand",json!({"InstanceIds":[self.instance],"DocumentName":"AWS-RunShellScript","Parameters":{"commands":[command]},"TimeoutSeconds":35}))?;
        let id = v["Command"]["CommandId"]
            .as_str()
            .context("SSM command id missing")?;
        let deadline = std::time::Instant::now() + Duration::from_secs(35);
        while std::time::Instant::now() < deadline {
            let v = match self.ssm(
                "GetCommandInvocation",
                json!({"CommandId":id,"InstanceId":self.instance}),
            ) {
                Ok(v) => v,
                Err(e) if e.to_string().contains("InvocationDoesNotExist") => {
                    std::thread::sleep(Duration::from_secs(1));
                    continue;
                }
                Err(e) => return Err(e),
            };
            let status = v["Status"].as_str().context("SSM status missing")?;
            if status == "Success" {
                return Ok(format!(
                    "{}{}",
                    v["StandardOutputContent"].as_str().unwrap_or(""),
                    v["StandardErrorContent"].as_str().unwrap_or("")
                ));
            }
            ensure!(
                !matches!(status, "Failed" | "Cancelled" | "TimedOut"),
                "pair command {status}"
            );
            std::thread::sleep(Duration::from_secs(1));
        }
        anyhow::bail!("pair command timed out")
    }
    fn notify(&mut self, message: &str) -> Result<()> {
        self.query(
            "sns",
            "Publish",
            "2010-03-31",
            &[
                (
                    "TopicArn",
                    self.topic
                        .as_deref()
                        .context("SNS_TOPIC_ARN required for breaker")?,
                ),
                ("Subject", "nexode circuit breaker fired"),
                ("Message", message),
            ],
        )?;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn signature_binds_payload_region_target_and_temporary_token() {
        let sign = |body, token| {
            signed_headers(
                "ssm",
                "us-east-1",
                body,
                "application/x-amz-json-1.1",
                Some("AmazonSSM.SendCommand"),
                "20260101T000000Z",
                ("AKID", "secret", token),
            )
        };
        let a = sign("{}", Some("session"));
        let b = sign("{\"x\":1}", Some("session"));
        assert_ne!(a["authorization"], b["authorization"]);
        assert_ne!(a["authorization"], sign("{}", None)["authorization"]);
        assert!(a["authorization"].contains("20260101/us-east-1/ssm/aws4_request"));
        assert!(a["authorization"].contains("x-amz-security-token"));
        assert!(a["authorization"].contains("x-amz-target"));
        assert_eq!(
            hex(&hmac(&[0x0b; 20], b"Hi There")),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
    }
}
