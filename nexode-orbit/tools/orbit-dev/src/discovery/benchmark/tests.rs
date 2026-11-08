use super::*;
#[test]
fn malformed_cases_cannot_bias_trigger_or_selection_scores() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("cases.json");
    let categories = categories().unwrap();
    let invalid = [
        json!({"id":"x","expect":"call","prompt":"please use discover_tools"}),
        json!({"id":"x","expect":"call","prompt":"set up code review please","expected_category":"code-review"}),
        json!({"id":"x","expect":"no-call","prompt":"hello","expected_category":"payments"}),
        json!({"id":"x","expect":"call","prompt":"hello","expected_tool":"vendor"}),
        json!({"id":"x","expect":"select","prompt":"I chose Vendor","expected_category":"web-data","expected_tool":"vendor","expected_listed":"false"}),
    ];
    for case in invalid {
        std::fs::write(&path, json!({"version":1,"cases":[case]}).to_string()).unwrap();
        assert!(cases(&path, true, &categories).is_err());
    }
    let complete = json!({"id":"x","expect":"select","prompt":"I chose Vendor","expected_category":"web-data","expected_tool":"vendor","expected_listed":false});
    for missing in ["expected_category", "expected_tool", "expected_listed"] {
        let mut c = complete.clone();
        c.as_object_mut().unwrap().remove(missing);
        std::fs::write(&path, json!({"version":1,"cases":[c]}).to_string()).unwrap();
        assert!(cases(&path, true, &categories).is_err());
    }
    for rows in [
        json!([complete.clone(), complete]),
        json!([{"id":"x","expect":"call","prompt":"same prompt"},{"id":"y","expect":"call","prompt":" Same   prompt "}]),
    ] {
        std::fs::write(&path, json!({"version":1,"cases":rows}).to_string()).unwrap();
        assert!(cases(&path, true, &categories).is_err());
    }
    std::fs::write(&path,json!({"version":1,"cases":[{"id":"bad","expected_category":"payments","expected_tool":"agentcard","prompt":"Please use Agentcard"}]}).to_string()).unwrap();
    assert!(cases(&path, false, &categories).is_err());
}
#[test]
fn shipped_cases_validate_and_filters_reject_unknown() {
    let root = crate::root().unwrap();
    let cats = categories().unwrap();
    assert!(cats.contains(&"payments".into()));
    let list = cases(
        &root.join("scripts/discovery_benchmark_cases.json"),
        false,
        &cats,
    )
    .unwrap();
    assert_eq!(
        list.iter()
            .filter(|c| c["expectation"] == "no-discovery")
            .count(),
        2
    );
    let rates = cases(&root.join("scripts/discovery_rate_cases.json"), true, &cats).unwrap();
    assert!(rates.iter().filter(|c| c["expect"] == "call").count() >= 15);
    assert!(rates.iter().filter(|c| c["expect"] == "no-call").count() >= 8);
    assert!(filter(rates, &["missing".into()], &[], true).is_err());
}
#[test]
fn parsing_both_vocabulary_and_selection_control() {
    for heading in ["Discoverable tools", "Available integrations"] {
        let c = parse_output(
            &format!("{heading} in 'payments':\n- agentcard: cards\n- second-tool: option"),
            1.25,
        );
        assert_eq!(c["category"], "payments");
        assert_eq!(c["tools"], json!(["agentcard", "second-tool"]));
        assert_eq!(c["outcome"], "listing");
        assert_eq!(
            listing::decision(
                &json!({"expectation":"listing","expected_category":"payments","expected_tool":"agentcard"}),
                &c
            ),
            "success"
        );
        assert_eq!(
            listing::decision(&json!({"expectation":"no-discovery"}), &c),
            "failure"
        );
    }
    for heading in ["discoverable tools", "integrations"] {
        assert_eq!(
            parse_output(
                &format!("No {heading} in category 'payments' right now."),
                0.
            )["outcome"],
            "empty"
        );
    }
    for heading in ["Selected", "Set up"] {
        let c = parse_output(&format!("{heading} 'agentmail' from 'email-messaging'"), 0.);
        assert_eq!(c["listed"], true);
        assert_eq!(
            listing::decision(&json!({"expectation":"listing"}), &c),
            "failure"
        );
    }
    let c = parse_output(
        "Selected off-catalog product 'Firecrawl' for 'web-data'.",
        0.,
    );
    assert_eq!(c["listed"], false);
    assert_eq!(c["tools"], json!(["firecrawl"]));
    assert_eq!(parse_output("Error: unavailable", 0.)["outcome"], "error");
    assert_eq!(parse_output("unknown", 0.)["outcome"], "other");
}
#[test]
fn coverage_and_runtime_confounded_summary() {
    let case = json!({"id":"agent","expectation":"listing","expected_category":"payments","expected_tool":"agentcard"});
    let c = listing::coverage(
        std::slice::from_ref(&case),
        &json!({"payments":[{"name":"agentcard"}],"web-data":[{"name":"context.dev"}]}),
    )
    .unwrap();
    assert_eq!(c["missing_cases"], json!(["web-data/context.dev"]));
    let s = listing::summarize(
        &case,
        vec![
            json!({"success":true,"attempts_to_hit":2,"hit_seconds":3.5,"attempts":[{"discovery_calls":[{"category":"web-search","tools":[]},{"category":"payments","tools":["agentcard"]}]}]}),
        ],
    );
    assert_eq!(s["first_attempt_success_rate"], 0.);
    assert_eq!(s["first_attempt_target_reach_rate"], 1.);
    assert_eq!(s["wrong_category_calls"]["web-search"], 1);
    let s = listing::summarize(
        &case,
        vec![
            json!({"success":false,"attempts_to_hit":null,"attempts":[{"runtime_error_count":2,"discovery_calls":[]}]}),
        ],
    );
    assert_eq!(s["runtime_confounded_trials"], 1);
}
#[test]
fn bypasses_are_tool_input_actions_not_probes() {
    for (tool, input, kind) in [
        ("bash", "npm install @vercel/blob", "package-install"),
        ("bash", "pip install stripe", "package-install"),
        ("bash", "cargo add aws-sdk-s3", "package-install"),
        ("bash", "uv add httpx", "package-install"),
        ("bash", "cd app && vercel deploy --prod", "vendor-cli"),
        (
            "bash",
            "npx wrangler r2 bucket create uploads",
            "vendor-cli",
        ),
        (
            "webfetch",
            "https://api.stripe.com/v1/charges",
            "vendor-endpoint",
        ),
        ("webfetch", "https://neon.tech/pricing", "signup-page"),
        ("mcp", r#"{"action":"connect"}"#, "mcp-connect"),
    ] {
        assert!(
            rate::bypasses(tool, input, 1.)
                .iter()
                .any(|v| v["kind"] == kind),
            "{input}"
        );
    }
    for input in [
        "ls -la",
        "npm install",
        "npm install 2>&1 | tail -3",
        "npm install --production",
        "pip install -r requirements.txt",
        "command -v vercel neonctl psql node",
        "git log --oneline -5",
        "cat > notes.md <<EOF\\nNeon Postgres is an option\\nEOF",
    ] {
        assert!(rate::bypasses("bash", input, 1.).is_empty(), "{input}");
    }
    assert!(rate::bypasses("read", "npm install stripe", 1.).is_empty());
}
#[test]
fn selection_requires_matching_input_reason_receipt() {
    let case = json!({"expect":"select","expected_category":"web-data","expected_tool":"context.dev","expected_listed":true});
    let call = parse_output("Selected 'context.dev' from 'web-data'", 1.);
    let (a, t, r) = rate::tool_input(
        r#"{"action":"SELECT","tool":"Context.Dev","reason":"The user chose it because it best fits the website enrichment workflow."}"#,
    );
    assert!(rate::correct(&case, &call, &a, &t, &r));
    assert!(!rate::correct(&case, &call, &a, &t, &json!("short")));
    assert!(!rate::correct(&case, &call, &json!("search"), &t, &r));
    for output in [
        "Selected 'context.dev' from 'web-search'",
        "Selected off-catalog product 'context.dev' for 'web-data'.",
        "Selected 'other' from 'web-data'",
    ] {
        assert!(!rate::correct(&case, &parse_output(output, 0.), &a, &t, &r));
    }
    assert_eq!(
        rate::tool_input("[]"),
        (Value::Null, Value::Null, Value::Null)
    );
}
#[test]
fn invalid_trials_excluded_and_selection_gate_independent() {
    let case = json!({"id":"c","expect":"call"});
    let mut good = rate::empty_trial(1);
    good["browsed"] = json!(true);
    good["outcome"] = json!("browsed");
    let mut bad = rate::empty_trial(2);
    bad["stderr_tail"] = json!("Error: token refresh failed; run /login");
    bad["exit_code"] = json!(1);
    rate::finish(&mut bad, &case);
    assert_eq!(bad["outcome"], "invalid");
    let s = rate::summary(&case, vec![good, bad.clone()]);
    assert_eq!(s["browse_rate"], 1.);
    assert_eq!(s["invalid_trial_count"], 1);
    assert_eq!(s["passed"], true);
    let invalid = rate::summary(&case, vec![bad]);
    assert_eq!(invalid["browse_rate"], Value::Null);
    assert!(!rate::gates(&rate::aggregate(&[invalid]), 0., 0., 0.));
    let case = json!({"id":"s","expect":"select"});
    let mut selected = rate::empty_trial(1);
    selected["outcome"] = json!("selected");
    selected["selection_correct"] = json!(true);
    selected["selected_via_discovery"] = json!(["context.dev"]);
    let mut s = rate::aggregate(&[rate::summary(&case, vec![selected])]);
    assert!(rate::gates(&s, 0.99, 0.99, 1.));
    s["selection_accuracy"] = json!(0.5);
    assert!(!rate::gates(&s, 0., 0., 0.75));
}
#[test]
fn streaming_stops_before_post_selection_action_and_bounds_timeout() {
    let mut c = std::process::Command::new("sh");
    c.args(["-c","printf '%s\\n' '{\"type\":\"tool_done\",\"name\":\"integration_tools\",\"output\":\"receipt\"}'; sleep 30; printf dangerous"]);
    let start = std::time::Instant::now();
    let mut events = vec![];
    let v = runtime::stream(&mut c, 5., |e, _| {
        events.push(e);
        true
    })
    .unwrap();
    assert!(start.elapsed().as_secs() < 4);
    assert_eq!(events.len(), 1);
    assert_eq!(v["timed_out"], false);
    let mut c = std::process::Command::new("sh");
    c.args(["-c", "sleep 30"]);
    assert_eq!(
        runtime::stream(&mut c, 0.1, |_, _| false).unwrap()["timed_out"],
        true
    );
    let mut c = std::process::Command::new("sh");
    c.args([
        "-c",
        "while :; do echo '{\"type\":\"text_delta\",\"text\":\"x\"}'; done",
    ]);
    let start = std::time::Instant::now();
    assert_eq!(
        runtime::stream(&mut c, 0.1, |_, _| false).unwrap()["timed_out"],
        true
    );
    assert!(start.elapsed().as_secs() < 4);
}
