use super::*;
fn target(samples: &[Sample], top: usize, minimum: i64) -> Value {
    let target = samples.first().map_or("unknown", |s| s.target.as_str());
    let process = stats::process(samples);
    let population = if target == "server" {
        stats::population(samples)
    } else {
        json!({})
    };
    let attr = last_attr(samples);
    let coverage = attr.map(stats::coverage).unwrap_or(Value::Null);
    let incident = if target == "server" {
        incident::assess(&process, &coverage, &population)
    } else {
        json!({})
    };
    let process_samples = samples
        .iter()
        .filter(|s| s.pss().is_some())
        .collect::<Vec<_>>();
    let mut spikes=process_samples.windows(2).filter_map(|pair|{
        let (a,b)=(pair[0],pair[1]);let delta=b.pss().unwrap()-a.pss().unwrap();
        (delta.abs()>=minimum).then(||json!({"from":a.time,"to":b.time,"delta_pss_bytes":delta,"from_source":a.source,"to_source":b.source,"to_trigger_category":b.category,"to_trigger_reason":b.reason}))
    }).collect::<Vec<_>>();
    spikes.sort_by_key(|v| std::cmp::Reverse(n(&v["delta_pss_bytes"]).abs()));
    spikes.truncate(top);
    let attribution = samples
        .iter()
        .filter(|s| nonempty(s.sessions()))
        .collect::<Vec<_>>();
    let mut deltas = vec![];
    if target == "server" {
        for pair in attribution.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            let mut value = json!({"from":a.time,"to":b.time,"to_trigger_category":b.category,"to_trigger_reason":b.reason});
            let mut magnitude = 0;
            for key in [
                "total_json_bytes",
                "payload_text_bytes",
                "provider_cache_json_bytes",
                "tool_result_bytes",
                "large_blob_bytes",
                "live_count",
                "memory_enabled_session_count",
            ] {
                let source = if matches!(
                    key,
                    "live_count" | "memory_enabled_session_count" | "total_json_bytes"
                ) {
                    key.into()
                } else {
                    format!("total_{key}")
                };
                let delta = n(&b.sessions()[&source]) - n(&a.sessions()[&source]);
                value[format!("delta_{key}")] = json!(delta);
                if key.ends_with("bytes") {
                    magnitude = magnitude.max(delta.abs());
                }
            }
            deltas.push((magnitude, value));
        }
    }
    deltas.sort_by_key(|(m, _)| std::cmp::Reverse(*m));
    let deltas = deltas
        .into_iter()
        .take(top)
        .map(|(_, v)| v)
        .collect::<Vec<_>>();
    let mut sessions = if target == "server" {
        stats::peaks(samples, false)
    } else {
        vec![]
    };
    let mut clients = if target == "client" {
        stats::peaks(samples, true)
    } else {
        vec![]
    };
    sessions.truncate(top);
    clients.truncate(top);
    let mut events = counts(samples.iter().map(|s| s.category.as_str()))
        .as_object()
        .unwrap()
        .clone()
        .into_iter()
        .collect::<Vec<_>>();
    events.sort_by_key(|(_, v)| std::cmp::Reverse(n(v)));
    let events = events.into_iter().collect::<serde_json::Map<_, _>>();
    let hints = if target == "server" {
        hints::server(samples, &sessions)
    } else {
        hints::client(samples, &clients)
    };
    json!({"target":target,"instance_id":samples.last().map(|s|&s.instance),"sample_count":samples.len(),"first_timestamp_ms":samples.first().map(|s|s.time),"last_timestamp_ms":samples.last().map(|s|s.time),"kinds":counts(samples.iter().map(|s|s.kind.as_str())),"process":process,"coverage":coverage,"session_population":population,"incident":incident,"last_attribution":attr.map(|a|json!({"timestamp_ms":a.time,"sessions":a.raw["sessions"].as_object(),"totals":a.raw["totals"].as_object(),"client":if target=="client"{a.raw["client"].clone()}else{Value::Null},"trigger_category":a.category,"trigger_reason":a.reason})),"top_spikes":spikes,"top_attribution_deltas":deltas,"top_sessions":sessions,"top_clients":clients,"event_counts":events,"hints":hints})
}
pub(super) fn summarize(samples: &[Sample], top: usize, minimum: i64) -> Value {
    let targets = samples
        .iter()
        .map(|s| s.target.as_str())
        .collect::<BTreeSet<_>>();
    if targets.len() <= 1 {
        return target(samples, top, minimum);
    }
    let mut values = serde_json::Map::new();
    for name in targets {
        let group = samples
            .iter()
            .filter(|s| s.target == name)
            .cloned()
            .collect::<Vec<_>>();
        values.insert(name.into(), target(&group, top, minimum));
    }
    json!({"targets":values})
}
fn display(key: &str, value: &Value) -> String {
    if key.ends_with("_bytes") {
        mb(int(value))
    } else if key.contains("ratio") {
        value
            .as_f64()
            .map(|n| format!("{:.1}%", n * 100.0))
            .unwrap_or_else(|| "n/a".into())
    } else if key.ends_with("timestamp_ms") || matches!(key, "from" | "to") {
        int(value)
            .and_then(chrono::DateTime::from_timestamp_millis)
            .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
            .unwrap_or_else(|| "n/a".into())
    } else if let Some(s) = value.as_str() {
        s.into()
    } else {
        value.to_string()
    }
}
fn print_value(key: &str, value: &Value, indent: usize) {
    let pad = " ".repeat(indent);
    match value {
        Value::Object(map) => {
            println!("{pad}{}:", key.replace('_', " "));
            for (k, v) in map {
                print_value(k, v, indent + 2);
            }
        }
        Value::Array(values) => {
            println!("{pad}{}:", key.replace('_', " "));
            for (i, v) in values.iter().enumerate() {
                print_value(&format!("{}", i + 1), v, indent + 2);
            }
        }
        _ => println!("{pad}{}: {}", key.replace('_', " "), display(key, value)),
    }
}
pub(super) fn human(summary: &Value) -> Result<()> {
    println!("Runtime Memory Log Analysis\n===========================");
    if let Some(targets) = summary["targets"].as_object() {
        for (name, value) in targets {
            println!("\n[{name}]");
            human(value)?;
        }
    } else if let Some(values) = summary.as_object() {
        for (key, value) in values {
            if key != "files" {
                print_value(key, value, 0);
            }
        }
    }
    if let Some(files) = summary["files"].as_array() {
        println!("files: {}", files.len());
        for file in files {
            println!("  {}", s(file));
        }
    }
    Ok(())
}
