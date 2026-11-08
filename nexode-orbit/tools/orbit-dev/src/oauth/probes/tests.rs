use super::*;
#[test]
fn all_original_variants_preserve_headers_system_and_tools() {
    let h = cases("headers", "model-fixture").unwrap();
    assert_eq!(h.len(), 7);
    assert_eq!(h[0].headers["anthropic-beta"], BETA);
    assert!(!h[1].headers.contains_key("x-app"));
    assert_eq!(h[2].headers["anthropic-beta"], "oauth-2025-04-20");
    assert_eq!(h[4].body["system"].as_array().unwrap().len(), 1);
    assert_eq!(h[5].body["tools"].as_array().unwrap().len(), 6);
    let identity = cases("identity", "fixture").unwrap();
    assert_eq!(identity.len(), 7);
    assert_eq!(identity[3].body["system"].as_array().unwrap().len(), 1);
    assert_eq!(
        identity[0].headers["X-Claude-Code-Session-Id"],
        identity[6].headers["X-Claude-Code-Session-Id"]
    );
    let origin = cases("originator", "fixture").unwrap();
    assert_eq!(origin.len(), 3);
    assert_eq!(origin[0].headers["originator"], "codex_cli_rs");
    assert!(!origin[1].headers.contains_key("originator"));
    assert_eq!(origin[2].headers["originator"], "nexode");
    assert_eq!(origin[0].body["store"], false);
    assert_eq!(origin[0].body["stream"], true);
    for c in h.iter().chain(identity.iter()).chain(origin.iter()) {
        assert!(!c.headers.contains_key("Authorization"));
    }
}
#[test]
fn real_loopback_requests_sse_json_status_and_token_redaction() {
    for mode in ["headers", "originator", "failure"] {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", server.server_addr());
        let worker = std::thread::spawn(move || {
            let mut req = server.recv().unwrap();
            assert!(
                req.headers()
                    .iter()
                    .any(|h| h.field.equiv("Authorization")
                        && h.value.as_str() == "Bearer fake-token")
            );
            let body: Value = serde_json::from_reader(req.as_reader()).unwrap();
            assert_eq!(body["model"], "fixture");
            let response = match mode {
                "originator" => {
                    assert!(
                        req.headers()
                            .iter()
                            .any(|h| h.field.equiv("chatgpt-account-id")
                                && h.value.as_str() == "account-fixture")
                    );
                    "data: {\"type\":\"response.output_text.delta\",\"delta\":\"hello fake-token\"}\n\ndata: [DONE]\n".into()
                }
                "failure" => "fake-token error".into(),
                _ => json!({"content":[{"text":"hello fake-token"}]}).to_string(),
            };
            req.respond(
                tiny_http::Response::from_string(response).with_status_code(if mode == "failure" {
                    403
                } else {
                    200
                }),
            )
            .unwrap();
        });
        let c = cases(
            if mode == "originator" {
                "originator"
            } else {
                "headers"
            },
            "fixture",
        )
        .unwrap()
        .remove(0);
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap();
        let out = send(
            &client,
            &endpoint,
            &c,
            "fake-token",
            Some("account-fixture"),
            mode == "originator",
        )
        .unwrap();
        worker.join().unwrap();
        assert!(!out.contains("fake-token"));
        if mode == "failure" {
            assert!(out.contains("403"));
        } else {
            assert!(out.contains("hello [redacted]"));
        }
    }
}
