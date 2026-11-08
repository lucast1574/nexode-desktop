use super::*;
struct Fake {
    state: String,
    online: bool,
    started: bool,
    stopped: bool,
    notifications: Vec<String>,
}
impl Backend for Fake {
    fn state(&mut self) -> Result<String> {
        Ok(self.state.clone())
    }
    fn start(&mut self) -> Result<()> {
        self.started = true;
        Ok(())
    }
    fn stop(&mut self) -> Result<()> {
        self.stopped = true;
        Ok(())
    }
    fn online(&mut self) -> Result<bool> {
        Ok(self.online)
    }
    fn pair(&mut self) -> Result<String> {
        Ok("Pairing code: \x1b[1;37m123 456\x1b[0m".into())
    }
    fn notify(&mut self, message: &str) -> Result<()> {
        self.notifications.push(message.into());
        Ok(())
    }
}
fn config() -> Config {
    Config {
        token: "test-token".into(),
        host: "100.64.0.10".into(),
        port: 7643,
        instance: "i-fixture".into(),
    }
}
fn backend() -> Fake {
    Fake {
        state: "running".into(),
        online: true,
        started: false,
        stopped: false,
        notifications: vec![],
    }
}
fn event(action: &str) -> Value {
    json!({"requestContext":{"http":{"method":"POST"}},"headers":{"Authorization":"Bearer test-token"},"body":json!({"action":action}).to_string()})
}
#[test]
fn landing_and_legacy_redirect_do_not_embed_credentials() {
    let mut b = backend();
    let c = config();
    let r = wake(&json!({}), &c, &mut b).unwrap();
    assert_eq!(r["statusCode"], 200);
    assert!(!r["body"].as_str().unwrap().contains("test-token"));
    assert!(
        r["headers"]["Content-Security-Policy"]
            .as_str()
            .unwrap()
            .contains("nonce-")
    );
    let r = wake(
        &json!({"queryStringParameters":{"t":"test-token"}}),
        &c,
        &mut b,
    )
    .unwrap();
    assert_eq!(r["statusCode"], 302);
    assert_eq!(r["headers"]["Location"], "/#t=test-token");
}
#[test]
fn auth_status_pair_and_invalid_requests() {
    let mut b = backend();
    let c = config();
    let mut e = event("status");
    e["headers"] = json!({});
    assert_eq!(wake(&e, &c, &mut b).unwrap()["statusCode"], 403);
    let r = wake(&event("status"), &c, &mut b).unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(r["body"].as_str().unwrap()).unwrap()["healthy"],
        true
    );
    let r = wake(&event("pair"), &c, &mut b).unwrap();
    let v: Value = serde_json::from_str(r["body"].as_str().unwrap()).unwrap();
    assert_eq!(v["code"], "123456");
    assert_eq!(
        v["uri"],
        "nexode://pair?host=100.64.0.10&port=7643&code=123456"
    );
    b.online = false;
    assert_eq!(wake(&event("pair"), &c, &mut b).unwrap()["statusCode"], 503);
    b.state = "stopped".into();
    assert_eq!(wake(&event("pair"), &c, &mut b).unwrap()["statusCode"], 409);
    wake(&event("wake"), &c, &mut b).unwrap();
    assert!(b.started);
    assert_eq!(
        wake(&event("unknown"), &c, &mut b).unwrap()["statusCode"],
        400
    );
    let mut e = event("status");
    e["body"] = json!("not JSON");
    assert_eq!(wake(&e, &c, &mut b).unwrap()["statusCode"], 400);
}
#[test]
fn breaker_stops_running_instance_and_notifies_for_both_states() {
    let mut b = backend();
    let c = config();
    breaker(&c, &mut b).unwrap();
    assert!(b.stopped);
    assert_eq!(b.notifications.len(), 1);
    b.stopped = false;
    b.state = "stopped".into();
    breaker(&c, &mut b).unwrap();
    assert!(!b.stopped);
    assert_eq!(b.notifications.len(), 2);
}

#[test]
fn custom_runtime_returns_response_and_invocation_error() {
    for fail in [false, true] {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let address = server.server_addr().to_string();
        let worker = std::thread::spawn(move || {
            let req = server
                .recv_timeout(Duration::from_secs(5))
                .unwrap()
                .unwrap();
            assert!(req.url().ends_with("/invocation/next"));
            req.respond(
                tiny_http::Response::from_string("{\"fixture\":true}").with_header(
                    tiny_http::Header::from_bytes("Lambda-Runtime-Aws-Request-Id", "request-1")
                        .unwrap(),
                ),
            )
            .unwrap();
            let mut req = server
                .recv_timeout(Duration::from_secs(5))
                .unwrap()
                .unwrap();
            assert!(req.url().ends_with(if fail {
                "/request-1/error"
            } else {
                "/request-1/response"
            }));
            let mut body = String::new();
            req.as_reader().read_to_string(&mut body).unwrap();
            let body: Value = serde_json::from_str(&body).unwrap();
            if fail {
                assert_eq!(body["errorType"], "OrbitPhoneError");
                assert!(!body.to_string().contains("secret"));
            } else {
                assert_eq!(body["ok"], true);
            }
            req.respond(tiny_http::Response::empty(202)).unwrap();
            server
                .recv_timeout(Duration::from_secs(5))
                .unwrap()
                .unwrap()
                .respond(tiny_http::Response::empty(500))
                .unwrap();
        });
        assert!(
            runtime(&address, |v| {
                assert_eq!(v["fixture"], true);
                if fail {
                    anyhow::bail!("secret internal failure");
                }
                Ok(json!({"ok":true}))
            })
            .is_err()
        );
        worker.join().unwrap();
    }
}

#[test]
fn package_contains_only_executable_bootstrap() {
    use std::io::Read;
    let dir = tempfile::tempdir().unwrap();
    let binary = dir.path().join("native");
    let zip = dir.path().join("lambda.zip");
    fs::write(&binary, b"\x7fELFfixture").unwrap();
    package(
        [
            binary.to_str().unwrap().into(),
            zip.to_str().unwrap().into(),
        ]
        .into_iter(),
    )
    .unwrap();
    let mut archive = zip::ZipArchive::new(fs::File::open(zip).unwrap()).unwrap();
    assert_eq!(archive.len(), 1);
    let mut bootstrap = archive.by_name("bootstrap").unwrap();
    assert_eq!(bootstrap.unix_mode().unwrap() & 0o777, 0o755);
    let mut data = vec![];
    bootstrap.read_to_end(&mut data).unwrap();
    assert_eq!(data, b"\x7fELFfixture");
}
