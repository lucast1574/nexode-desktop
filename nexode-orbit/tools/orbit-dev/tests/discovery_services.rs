mod support;
use serde_json::{Value, json};
use std::{
    fs,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
struct Service {
    child: Child,
    _dir: tempfile::TempDir,
    url: String,
}
impl Drop for Service {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
fn service(sponsor: bool) -> Service {
    let dir = tempfile::tempdir().unwrap();
    let ready = dir.path().join("ready.json");
    let mut c = Command::new(support::binary("orbit-dev"));
    c.env_clear().env("PATH", "");
    if sponsor {
        c.args(["mock-sponsor", "serve", "--db"])
            .arg(dir.path().join("sponsor.db"));
    } else {
        c.arg("openrelay-discovery-server");
    }
    c.arg("--ready-file")
        .arg(&ready)
        .stdout(Stdio::null())
        .stderr(Stdio::inherit());
    let mut service = Service {
        child: c.spawn().unwrap(),
        _dir: dir,
        url: String::new(),
    };
    let start = Instant::now();
    loop {
        if let Ok(bytes) = fs::read(&ready)
            && let Ok(v) = serde_json::from_slice::<Value>(&bytes)
        {
            service.url = v[if sponsor { "url" } else { "endpoint" }]
                .as_str()
                .unwrap()
                .into();
            break;
        }
        assert!(service.child.try_wait().unwrap().is_none(), "server exited");
        assert!(start.elapsed() < Duration::from_secs(5), "ready timeout");
        std::thread::sleep(Duration::from_millis(10));
    }
    service
}
fn cli(args: &[&str]) -> std::process::Output {
    Command::new(support::binary("orbit-dev"))
        .env_clear()
        .env("PATH", "")
        .arg("mock-sponsor")
        .args(args)
        .output()
        .unwrap()
}
fn signup(service: &Service, email: &str, via: Option<&str>) -> String {
    let mut args = vec!["signup", "--service", &service.url, "--email", email];
    if let Some(via) = via {
        args.extend(["--via", via]);
    }
    support::success(cli(&args)).trim().into()
}
fn account(service: &Service, email: &str) -> Value {
    serde_json::from_str(&support::success(cli(&[
        "account",
        "--service",
        &service.url,
        "--email",
        email,
    ])))
    .unwrap()
}
#[test]
fn sponsor_cli_signed_roundtrip_immutability_replay_and_validation() {
    let service = service(true);
    let link = signup(
        &service,
        "AtTributed@example.test",
        Some("nexode-discovery"),
    );
    assert!(!link.contains("nexode-discovery"));
    assert!(support::success(cli(&["confirm", &link])).contains("true"));
    let repeat = cli(&["confirm", &link]);
    assert!(!repeat.status.success());
    assert!(String::from_utf8_lossy(&repeat.stderr).contains("already consumed"));
    assert_eq!(
        account(&service, "attributed@example.test")["acquisition_source"],
        "nexode-discovery"
    );
    let other = signup(&service, "attributed@example.test", Some("other-partner"));
    support::success(cli(&["confirm", &other]));
    assert_eq!(
        account(&service, "attributed@example.test")["acquisition_source"],
        "nexode-discovery"
    );
    let organic = signup(&service, "organic@example.test", None);
    support::success(cli(&["confirm", &organic]));
    assert!(account(&service, "organic@example.test")["acquisition_source"].is_null());
    let link = signup(&service, "tamper@example.test", Some("nexode-discovery"));
    let mut parsed = url::Url::parse(&link).unwrap();
    let token = parsed
        .query_pairs()
        .find(|(k, _)| k == "token")
        .unwrap()
        .1
        .into_owned();
    let changed = format!(
        "{}{}",
        if token.starts_with('A') { "B" } else { "A" },
        &token[1..]
    );
    parsed.set_query(None);
    parsed.query_pairs_mut().append_pair("token", &changed);
    let bad = cli(&["confirm", parsed.as_str()]);
    assert!(!bad.status.success());
    assert!(String::from_utf8_lossy(&bad.stderr).contains("invalid signup token"));
    for (email, via) in [("not-email", "ok"), ("a@b", "é"), ("a@b", &"x".repeat(101))] {
        assert!(
            !cli(&[
                "signup",
                "--service",
                &service.url,
                "--email",
                email,
                "--via",
                via
            ])
            .status
            .success()
        );
    }
}
#[test]
fn live_attribution_benchmark_and_exact_discovery_command() {
    let service = service(true);
    let root = tempfile::tempdir().unwrap();
    let sponsors = root.path().join("sponsors.json");
    let report = root.path().join("report.json");
    fs::write(&sponsors,json!({"version":1,"default_marker":"via=nexode-discovery","sponsors":[{"tool":"mock-sponsor","category":"payments","mechanism":"cli-flag","marker":"--via nexode-discovery","listing_marker":"via=nexode-discovery"}]}).to_string()).unwrap();
    let out = support::success(
        Command::new(support::binary("orbit-dev"))
            .env_clear()
            .env("PATH", "")
            .args([
                "benchmark-attribution",
                "--live",
                "--live-web",
                "--endpoint",
            ])
            .arg(format!("{}/v1/discovery", service.url))
            .arg("--sponsors")
            .arg(&sponsors)
            .arg("--output")
            .arg(&report)
            .output()
            .unwrap(),
    );
    assert!(out.contains("1/1 sponsors credit agent-driven CLI signups to nexode"));
    let report: Value = serde_json::from_slice(&fs::read(report).unwrap()).unwrap();
    assert_eq!(report["sponsors"][0]["score"], 100);
    assert_eq!(report["sponsors"][0]["cli_attribution"], "attributed");
    let body: Value =
        reqwest::blocking::get(format!("{}/v1/discovery?tool=mock-sponsor", service.url))
            .unwrap()
            .json()
            .unwrap();
    let setup = body["tool"]["setup"].as_str().unwrap();
    #[cfg(unix)]
    {
        let cmd = setup
            .split('`')
            .nth(1)
            .unwrap()
            .replace("<email>", "served-command@example.test");
        let link = support::success(
            Command::new("/bin/sh")
                .args(["-c", &cmd])
                .env_clear()
                .env("PATH", "")
                .output()
                .unwrap(),
        );
        support::success(cli(&["confirm", link.trim()]));
        assert_eq!(
            account(&service, "served-command@example.test")["acquisition_source"],
            "nexode-discovery"
        );
    }
    #[cfg(not(unix))]
    assert!(setup.contains("mock-sponsor signup"));
}
#[test]
fn openrelay_http_browse_select_unknown_routes() {
    let service = service(false);
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap();
    let get = |query: &str| {
        let r = client
            .get(format!("{}?{query}", service.url))
            .send()
            .unwrap();
        let status = r.status().as_u16();
        (status, r.json::<Value>().unwrap())
    };
    let (status, b) = get("category=cloud-infrastructure");
    assert_eq!(status, 200);
    assert_eq!(b["tools"][0]["name"], "openrelay-rivet");
    assert!(b["tools"][0].get("setup").is_none());
    let (_, b) = get("category=cloud-infrastructure&tool=openrelay-rivet");
    assert!(
        b["tool"]["setup"]
            .as_str()
            .unwrap()
            .contains("eth_getBlockByNumber")
    );
    assert_eq!(get("category=ai-models").1["tools"], json!([]));
    assert_eq!(get("category=bogus").0, 400);
    assert_eq!(get("category=ai-models&tool=openrelay-rivet").0, 404);
}
