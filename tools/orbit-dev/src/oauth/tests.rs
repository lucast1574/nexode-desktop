use super::*;
#[test]
fn pkce_random_state_private_storage_and_callback_variants() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("private/state.json");
    let first = begin(&path, 100).unwrap();
    let saved: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let verifier = saved["verifier"].as_str().unwrap();
    assert_eq!(verifier.len(), 64);
    assert_eq!(
        saved["challenge"],
        URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
    );
    assert_eq!(saved["state"].as_str().unwrap().len(), 32);
    let url = url::Url::parse(&first).unwrap();
    let pairs: std::collections::BTreeMap<_, _> = url.query_pairs().into_owned().collect();
    assert_eq!(pairs["code_challenge_method"], "S256");
    assert_eq!(pairs["state"], saved["state"].as_str().unwrap());
    assert!(!first.contains(verifier));
    assert_ne!(begin(&path, 100).unwrap(), first);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }
    assert_eq!(
        code(" abc#state ").unwrap(),
        ("abc".into(), Some("state".into()))
    );
    assert_eq!(
        code("https://example.test/callback?code=a%2Bb&state=s").unwrap(),
        ("a+b".into(), Some("s".into()))
    );
    assert_eq!(code("abc").unwrap(), ("abc".into(), None));
    for bad in [
        "",
        "abc#",
        "https://x.test/?state=only",
        "https://x.test/?code=a&code=b",
    ] {
        assert!(code(bad).is_err());
    }
}
#[test]
fn exchange_local_json_contract_private_credentials_and_no_token_output() {
    let tmp = tempfile::tempdir().unwrap();
    let state = tmp.path().join("state.json");
    let creds = tmp.path().join("claude/credentials.json");
    begin(&state, 100).unwrap();
    let pending: Value = serde_json::from_slice(&fs::read(&state).unwrap()).unwrap();
    let expected = pending["state"].as_str().unwrap().to_string();
    private_write(&creds, &json!({"unrelated":"keep"})).unwrap();
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}/token", server.server_addr());
    let worker = std::thread::spawn(move || {
        let mut req = server.recv().unwrap();
        assert_eq!(req.method().as_str(), "POST");
        let body: Value = serde_json::from_reader(req.as_reader()).unwrap();
        assert_eq!(body["code"], "test-code");
        assert_eq!(body["client_id"], CLIENT_ID);
        assert_eq!(body["state"], pending["state"]);
        assert_eq!(body["code_verifier"], pending["verifier"]);
        req.respond(tiny_http::Response::from_string(json!({"access_token":"fixture-access","refresh_token":"fixture-refresh","expires_in":3600}).to_string())).unwrap();
    });
    exchange(
        &state,
        &creds,
        &format!("test-code#{expected}"),
        &endpoint,
        110,
    )
    .unwrap();
    worker.join().unwrap();
    assert!(!state.exists());
    let value: Value = serde_json::from_slice(&fs::read(&creds).unwrap()).unwrap();
    assert_eq!(value["unrelated"], "keep");
    assert_eq!(value["claudeAiOauth"]["expiresAt"], 3_710_000);
    assert_eq!(value["claudeAiOauth"]["accessToken"], "fixture-access");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(creds).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
#[test]
fn rejects_expired_wrong_state_and_http_failure_preserves_pending() {
    let tmp = tempfile::tempdir().unwrap();
    let state = tmp.path().join("state.json");
    let creds = tmp.path().join("credentials.json");
    begin(&state, 100).unwrap();
    for (input, now) in [("code#wrong", 110), ("code", 1001), ("code", 99)] {
        assert!(exchange(&state, &creds, input, "http://127.0.0.1:1", now).is_err());
        assert!(state.exists());
        assert!(!creds.exists());
    }
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}/token", server.server_addr());
    let worker = std::thread::spawn(move || {
        server
            .recv()
            .unwrap()
            .respond(
                tiny_http::Response::from_string("secret response must not be logged")
                    .with_status_code(400),
            )
            .unwrap();
    });
    let error = exchange(&state, &creds, "code", &endpoint, 110)
        .unwrap_err()
        .to_string();
    worker.join().unwrap();
    assert!(error.contains("400"));
    assert!(!error.contains("secret"));
    assert!(state.exists());
    assert!(!creds.exists());
}
