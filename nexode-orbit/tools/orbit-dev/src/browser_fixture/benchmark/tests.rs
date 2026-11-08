use super::*;
pub(super) fn self_test() -> Result<()> {
    let events = [
        json!({"type":"text_delta","text":"I used handoff"}),
        json!({"type":"tool_start","id":"x","name":"browser"}),
        json!({"type":"tool_input","delta":"{\"action\":"}),
        json!({"type":"tool_input","delta":"\"handoff\"}"}),
        json!({"type":"tool_exec","id":"x","name":"browser"}),
        json!({"type":"tool_done","id":"x","name":"browser","error":null,"output":"{\"decision_provider\":\"nexode\",\"status\":\"done\",\"action_trace\":[{\"status\":\"executed\"}]}"}),
    ];
    let tr = trace::extract(
        &events
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n"),
    );
    ensure!(
        tr["browser_calls"][0]["action"] == "handoff"
            && tr["browser_calls"][0]["decision_provider"] == "nexode",
        "trace fragmented input contract"
    );
    ensure!(
        trace::handoff(&tr)["handoff_effective"] == true,
        "handoff not effective"
    );
    for (status, steps, error, effective) in [
        (json!("hand_back"), 0, Value::Null, false),
        (json!("hand_back"), 2, Value::Null, true),
        (json!("done"), 0, Value::Null, true),
        (json!("done"), 2, json!("failed"), false),
        (Value::Null, 0, Value::Null, false),
    ] {
        let mut sample = tr.clone();
        sample["browser_calls"][0]["handoff_status"] = status;
        sample["browser_calls"][0]["handoff_executed_steps"] = json!(steps);
        sample["browser_calls"][0]["error"] = error;
        ensure!(
            trace::handoff(&sample)["handoff_effective"] == effective,
            "handoff effectiveness contract"
        );
    }
    ensure!(
        trace::extract(r#"{"type":"text_delta","text":"handoff"}"#)["browser_calls"] == json!([]),
        "prose counted as browser action"
    );
    ensure!(
        trace::summary(&[])["median_direct_over_handoff_ratio"].is_null(),
        "empty median"
    );
    let mut timings = serde_json::Map::new();
    trace::timing(
        &json!({"action_trace":[{"duration_ms":12}],"timing":{"total_ms":40}}),
        "",
        &mut timings,
    );
    ensure!(
        json!(timings) == json!({"action_trace[0].duration_ms":12,"timing":{"total_ms":40}}),
        "timing units lost"
    );
    let telemetry = trace::extract(
        "{\"type\":\"tokens\",\"input\":23,\"output\":4}\n{\"type\":\"done\",\"text\":\"receipt\",\"usage\":{\"input_tokens\":23},\"model_call_count\":2,\"actual_cost_usd\":0.001}",
    );
    ensure!(
        telemetry["model_call_count"] == 2
            && telemetry["actual_cost_usd"] == 0.001
            && telemetry["usage_tokens"] == json!({"input_tokens":23}),
        "telemetry contract"
    );
    ensure!(
        trace::extract("")["actual_cost_usd"].is_null(),
        "inferred price"
    );
    let normal = json!({"pair":1,"mode":"normal","valid_success":true,"handoff_used":true,"handoff_effective":true,"elapsed_seconds":2.});
    let direct = json!({"pair":1,"mode":"direct","valid_success":true,"handoff_used":false,"elapsed_seconds":3.});
    ensure!(
        trace::summary(&[normal.clone(), direct.clone()])["median_direct_over_handoff_ratio"]
            == 1.5,
        "ratio contract"
    );
    let mut failed = direct.clone();
    failed["valid_success"] = json!(false);
    failed["timed_out"] = json!(true);
    failed["handoff_used"] = json!(true);
    failed["protocol_compliant"] = json!(false);
    let summary = trace::summary(&[normal.clone(), failed]);
    ensure!(
        summary["eligible_speed_pairs"] == 0
            && summary["all_attempt_paired_ratios"] == json!([1.5])
            && summary["arms"]["direct"]["timeouts"] == 1,
        "failed attempts omitted"
    );
    let target: Vec<_> = (0..3)
        .flat_map(|i| {
            let mut j = normal.clone();
            j["mode"] = json!("jev");
            j["pair"] = json!(i);
            j["elapsed_seconds"] = json!(1.);
            let mut d = direct.clone();
            d["pair"] = json!(i);
            [j, d]
        })
        .collect();
    ensure!(
        trace::summary(&target)["target_met"] == true
            && trace::summary(&target)["full_suite_target_met"] == false,
        "paired speed gate"
    );
    ensure!(
        trace::summary(&target[..4])["target_met"] == false,
        "insufficient pairs passed"
    );
    let full: Vec<_> = ["discovery", "heldout"]
        .iter()
        .flat_map(|p| {
            ["navigation", "search", "form"]
                .iter()
                .flat_map(|t| {
                    target
                        .iter()
                        .map(|r| {
                            let mut r = r.clone();
                            r["phase"] = json!(p);
                            r["task"] = json!(t);
                            r
                        })
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>()
        })
        .collect();
    ensure!(
        trace::summary(&full)["full_suite_target_met"] == true,
        "full suite grouping"
    );
    let f = fixture::Fixture::new()?;
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()?;
    for phase in ["discovery", "heldout"] {
        for task in ["navigation", "search", "form"] {
            let (token, url) = f.add(task, phase)?;
            let root = url.rsplit_once('/').unwrap().0;
            let values = fixture::values(task, phase);
            let get = |step: &str| -> Result<String> {
                Ok(client
                    .get(format!("{root}/{step}"))
                    .send()?
                    .error_for_status()?
                    .text()?)
            };
            if task != "navigation" {
                ensure!(
                    client.get(format!("{root}/receipt")).send()?.status() == 404,
                    "premature receipt"
                );
            }
            get("start")?;
            match task {
                "navigation" => {
                    ensure!(get("guides")?.contains(values[0]), "navigation guide");
                    ensure!(get("browser")?.contains(values[1]), "navigation link");
                    get("navigation")?;
                }
                "search" => {
                    ensure!(
                        get("results?q=wrong&category=all")?.contains("No matching"),
                        "invalid search accepted"
                    );
                    let query = url::form_urlencoded::Serializer::new(String::new())
                        .append_pair("q", values[0])
                        .append_pair("category", values[1])
                        .finish();
                    get(&format!("results?{query}"))?;
                    get("item")?;
                    get("navigation")?;
                }
                _ => {
                    get("request")?;
                    let mut form = url::form_urlencoded::Serializer::new(String::new());
                    for (k, v) in ["name", "email", "notes"].iter().zip(&values) {
                        form.append_pair(k, v);
                    }
                    let raw = form.finish();
                    ensure!(
                        client
                            .post(format!("{root}/submit"))
                            .body(raw.clone())
                            .send()?
                            .text()?
                            .contains("Invalid"),
                        "invalid form accepted"
                    );
                    client
                        .post(format!("{root}/submit"))
                        .body(format!("{raw}&delivery=pickup&confirm=yes"))
                        .send()?
                        .error_for_status()?;
                }
            }
            get("receipt")?;
            ensure!(
                fixture::outcome(&f.state(&token)),
                "fixture sequence invalid"
            );
            ensure!(
                f.state(&token)["confirmation_dom_observed"] == false,
                "server request inferred DOM state"
            );
            client
                .post(format!("{root}/visible"))
                .body("")
                .send()?
                .error_for_status()?;
            ensure!(
                f.state(&token)["confirmation_dom_observed"] == true,
                "visible callback"
            );
            client
                .post(format!("{root}/left"))
                .body("")
                .send()?
                .error_for_status()?;
            ensure!(
                f.state(&token)["confirmation_dom_observed"] == false,
                "pagehide callback"
            );
            let mut state = f.state(&token);
            state["requests"] = json!([]);
            ensure!(!fixture::outcome(&state), "empty sequence passed");
        }
    }
    Ok(())
}
#[cfg(test)]
#[test]
fn offline_protocol_and_all_six_http_task_variants() {
    self_test().unwrap();
}
