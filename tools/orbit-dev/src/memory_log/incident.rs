use super::*;
pub(super) fn assess(process: &Value, coverage: &Value, population: &Value) -> Value {
    let pss = n(&process["final_pss_bytes"]);
    let growth = n(&process["net_pss_growth_bytes"]);
    let sessions = n(&population["final_live_sessions"]);
    let session_growth = n(&population["net_live_session_growth"]);
    let connected = int(&population["final_connected_clients"]);
    let detached = int(&population["detached_or_headless_live_sessions"]);
    let live = n(&coverage["allocator_live_bytes"]);
    let attributed = n(&coverage["attributed_live_bytes"]);
    let retained = n(&coverage["allocator_retained_resident_estimate_bytes"]);
    let ratio = coverage["coverage_ratio_live_heap"]
        .as_f64()
        .map(|v| format!("live-heap attribution coverage {:.1}%", v * 100.0))
        .unwrap_or_else(|| "live-heap attribution coverage unavailable".into());
    let runaway = sessions >= 128
        && (session_growth >= 64
            || detached.is_some_and(|n| n >= 128)
            || connected.is_some_and(|n| sessions >= 128.max(n * 16)));
    let (cause, confidence, summary, evidence) = if runaway {
        (
            "runaway_live_session_population",
            "high",
            format!(
                "Live Agent population grew from {} to {sessions} while allocator live memory grew by {}.",
                population["baseline_live_sessions"],
                mb(int(&population["net_allocator_live_growth_bytes"]))
            ),
            vec![
                format!(
                    "{sessions} live sessions vs {} connected clients",
                    connected
                        .map(|n| n.to_string())
                        .unwrap_or_else(|| "unknown".into())
                ),
                format!("{session_growth:+} live sessions during this process lifetime"),
                format!(
                    "{} allocator growth per added session",
                    mb(int(&population["allocator_growth_per_added_session_bytes"]))
                ),
                format!(
                    "allocator live {} vs attributed session JSON {}",
                    mb(Some(live)),
                    mb(Some(attributed))
                ),
            ],
        )
    } else if retained >= 256 * MB && pss > 0 && retained * 4 >= pss {
        (
            "allocator_retention",
            "high",
            "Freed-but-held allocator pages are a material share of current PSS.".into(),
            vec![
                format!("retained resident estimate {}", mb(Some(retained))),
                format!("allocator live {} vs PSS {}", mb(Some(live)), mb(Some(pss))),
            ],
        )
    } else if live > 0 && attributed >= 512 * MB && attributed * 2 >= live {
        ("session_payload_growth","high","Tracked transcript, provider-cache, tool-result, or blob state explains most live heap.".into(),vec![format!("attributed live {}",mb(Some(attributed))),ratio])
    } else if live >= 1024 * MB {
        (
            "unattributed_live_heap",
            "medium",
            "Live allocator bytes are high, but current application attribution is incomplete."
                .into(),
            vec![
                format!("allocator live {}", mb(Some(live))),
                format!("attributed live {}", mb(Some(attributed))),
                ratio,
            ],
        )
    } else if pss >= 1024 * MB {
        (
            "non_heap_or_mapping_growth",
            "medium",
            "PSS is high without matching allocator live bytes; inspect mappings and threads."
                .into(),
            vec![
                format!("PSS {}", mb(Some(pss))),
                format!("allocator live {}", mb(Some(live))),
            ],
        )
    } else {
        (
            "within_normal_operating_range",
            "high",
            "No server memory incident threshold is exceeded in this process lifetime.".into(),
            vec![
                format!("final PSS {}", mb(Some(pss))),
                format!("net PSS growth {:+.1} MB", growth as f64 / MB as f64),
            ],
        )
    };
    let severity = if pss >= 2048 * MB || growth >= 1024 * MB || sessions >= 512 {
        "critical"
    } else if pss >= 1024 * MB || growth >= 256 * MB || sessions >= 128 {
        "warning"
    } else {
        "healthy"
    };
    let actions: Value = serde_json::from_str(include_str!("actions.json"))
        .expect("embedded incident action catalog");
    json!({"severity":severity,"primary_cause":cause,"confidence":confidence,"summary":summary,"evidence":evidence,"recommended_actions":actions[cause],"runbook":"docs/MEMORY_INCIDENT_RUNBOOK.md"})
}
