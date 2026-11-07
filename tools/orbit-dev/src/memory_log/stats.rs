use super::*;
pub(super) fn process(samples: &[Sample]) -> Value {
    let samples = samples
        .iter()
        .filter(|s| s.pss().is_some())
        .collect::<Vec<_>>();
    let Some(first) = samples.first() else {
        return json!({});
    };
    let last = samples.last().unwrap();
    let peak = samples.iter().fold(*first, |a, b| {
        if b.pss().filter(|n| *n != 0).unwrap_or(-1) > a.pss().filter(|n| *n != 0).unwrap_or(-1) {
            b
        } else {
            a
        }
    });
    let mut values = samples.iter().map(|s| s.pss().unwrap()).collect::<Vec<_>>();
    values.sort();
    let middle = values.len() / 2;
    let median = if values.len() % 2 == 1 {
        values[middle]
    } else {
        (values[middle - 1] + values[middle]) / 2
    };
    json!({"sample_count":samples.len(),"first_timestamp_ms":first.time,"last_timestamp_ms":last.time,"duration_ms":(last.time-first.time).max(0),"baseline_pss_bytes":first.pss(),"final_pss_bytes":last.pss(),"net_pss_growth_bytes":last.pss().unwrap()-first.pss().unwrap(),"peak_pss_bytes":peak.pss(),"peak_growth_vs_baseline_bytes":peak.pss().unwrap()-first.pss().unwrap(),"median_pss_bytes":median,"peak_timestamp_ms":peak.time,"peak_trigger_category":peak.category,"peak_trigger_reason":peak.reason,"allocator_allocated_bytes":last.allocator("allocated_bytes"),"allocator_resident_bytes":last.allocator("resident_bytes"),"allocator_retained_bytes":last.allocator("retained_bytes")})
}
pub(super) fn population(samples: &[Sample]) -> Value {
    let samples = samples
        .iter()
        .filter(|s| int(&s.sessions()["live_count"]).is_some())
        .collect::<Vec<_>>();
    let Some(first) = samples.first() else {
        return json!({});
    };
    let last = samples.last().unwrap();
    // Keep the first maximum, matching historical reports when counts tie.
    let peak = samples.iter().fold(*first, |a, b| {
        if n(&b.sessions()["live_count"]) > n(&a.sessions()["live_count"]) {
            b
        } else {
            a
        }
    });
    let baseline = n(&first.sessions()["live_count"]);
    let final_live = n(&last.sessions()["live_count"]);
    let growth = final_live - baseline;
    let connected = super::first(
        &last.raw["clients"],
        &["connected_count", "connected_clients"],
    );
    let allocated_growth = last
        .allocator("allocated_bytes")
        .zip(first.allocator("allocated_bytes"))
        .map(|(a, b)| a - b);
    json!({"baseline_timestamp_ms":first.time,"final_timestamp_ms":last.time,"baseline_live_sessions":baseline,"final_live_sessions":final_live,"peak_live_sessions":n(&peak.sessions()["live_count"]),"peak_timestamp_ms":peak.time,"net_live_session_growth":growth,"final_connected_clients":connected,"detached_or_headless_live_sessions":connected.map(|c|(final_live-c).max(0)),"baseline_allocator_live_bytes":first.allocator("allocated_bytes"),"final_allocator_live_bytes":last.allocator("allocated_bytes"),"net_allocator_live_growth_bytes":allocated_growth,"allocator_growth_per_added_session_bytes":if growth>0{allocated_growth.map(|g|g.max(0)/growth)}else{None}})
}
pub(super) fn coverage(sample: &Sample) -> Value {
    let pss = sample.pss();
    let attributed = sample.attributed();
    let live = sample.allocator("allocated_bytes");
    let retained = sample.allocator("retained_bytes");
    let process = &sample.raw["process"];
    let os = &process["os"];
    let diagnostics = &sample.raw["process_diagnostics"];
    let stack = first(process, &["main_stack_bytes"])
        .filter(|v| *v != 0)
        .or_else(|| first(os, &["main_stack_bytes", "stack_bytes", "vm_stk_bytes"]));
    let threads = first(process, &["thread_count"])
        .filter(|v| *v != 0)
        .or_else(|| first(os, &["thread_count", "threads"]));
    let resident = first(
        diagnostics,
        &[
            "allocator_retained_resident_estimate_bytes",
            "allocator_retained_bytes",
        ],
    )
    .or(retained);
    let stack_estimate = first(diagnostics, &["thread_stack_estimate_bytes"]).or(stack);
    let mut report = json!({"timestamp_ms":sample.time,"pss_bytes":pss,"pss_anon_bytes":int(&os["pss_anon_bytes"]),"pss_file_bytes":int(&os["pss_file_bytes"]),"pss_shmem_bytes":int(&os["pss_shmem_bytes"]),"anon_huge_pages_bytes":int(&os["anon_huge_pages_bytes"]),"rss_file_bytes":int(&os["rss_file_bytes"]),"main_stack_bytes":stack,"thread_stack_estimate_bytes":stack_estimate,"thread_count":threads,"attributed_live_bytes":attributed,"allocator_live_bytes":live,"allocator_retained_bytes":retained,"allocator_retained_resident_estimate_bytes":resident});
    if let Some((attributed, live)) = attributed.zip(live) {
        report["unattributed_live_heap_bytes"] = json!((live - attributed).max(0));
        report["coverage_ratio_live_heap"] = json!(if live == 0 {
            0.0
        } else {
            rounded(attributed as f64 / live as f64)
        });
    }
    if let Some((attributed, pss)) = attributed.zip(pss) {
        report["coverage_ratio_pss"] = json!(if pss == 0 {
            0.0
        } else {
            rounded(attributed as f64 / pss as f64)
        });
    }
    if let Some(pss) = pss {
        let explained = [
            "attributed_live_bytes",
            "allocator_retained_resident_estimate_bytes",
            "pss_file_bytes",
            "thread_stack_estimate_bytes",
        ]
        .iter()
        .map(|k| n(&report[*k]))
        .sum::<i64>();
        report["explained_pss_bytes"] = json!(explained);
        report["unexplained_pss_bytes"] = json!((pss - explained).max(0));
        report["explained_ratio"] = json!(if pss == 0 {
            0.0
        } else {
            rounded(explained as f64 / pss as f64)
        });
    }
    report
}
pub(super) fn peaks(samples: &[Sample], client: bool) -> Vec<Value> {
    let mut values = serde_json::Map::new();
    for sample in samples {
        let entries = if client {
            if !nonempty(sample.totals()) {
                continue;
            }
            vec![&sample.raw["client"]]
        } else {
            sample.sessions()["top_by_json_bytes"]
                .as_array()
                .map(|a| a.iter().collect())
                .unwrap_or_default()
        };
        for entry in entries {
            let id = s(&entry["session_id"]);
            if id.is_empty() {
                continue;
            }
            let field = if client {
                "peak_total_attributed_bytes"
            } else {
                "peak_json_bytes"
            };
            let bytes = if client {
                n(&sample.totals()["total_attributed_bytes"])
            } else {
                n(&entry["json_bytes"])
            };
            if values.get(id).is_some_and(|v| n(&v[field]) >= bytes) {
                continue;
            }
            let mut value = json!({"session_id":id,"provider":entry["provider"],"model":entry["model"],"last_seen_timestamp_ms":sample.time});
            value[field] = json!(bytes);
            if client {
                value["client_instance_id"] = entry["client_instance_id"].clone();
                value["is_remote"] = json!(entry["is_remote"] == true);
                for key in [
                    "display_messages_estimate_bytes",
                    "provider_messages_json_bytes",
                    "side_panel_estimate_bytes",
                    "remote_state_bytes",
                ] {
                    value[format!("peak_{key}")] = json!(n(&sample.totals()[key]));
                }
            } else {
                value["memory_enabled"] = json!(entry["memory_enabled"] == true);
                value["message_count"] = json!(n(&entry["message_count"]));
                for key in [
                    "payload_text_bytes",
                    "provider_cache_json_bytes",
                    "tool_result_bytes",
                    "large_blob_bytes",
                ] {
                    value[format!("peak_{key}")] = json!(n(&entry[key]));
                }
            }
            values.insert(id.into(), value);
        }
    }
    let mut values = values.into_values().collect::<Vec<_>>();
    let field = if client {
        "peak_total_attributed_bytes"
    } else {
        "peak_json_bytes"
    };
    values.sort_by_key(|v| std::cmp::Reverse(n(&v[field])));
    values
}
