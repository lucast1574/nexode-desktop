use super::*;
pub(super) fn server(samples: &[Sample], peaks: &[Value]) -> Vec<String> {
    let Some(last) = last_attr(samples).filter(|s| nonempty(s.sessions())) else {
        return vec![
            "Need at least one attribution sample before generating optimization hints.".into(),
        ];
    };
    let sessions = last.sessions();
    let total = n(&sessions["total_json_bytes"]);
    let mut hints = vec![];
    let population = stats::population(samples);
    if n(&population["final_live_sessions"]) >= 128 {
        hints.push(format!("Live session population is high ({} sessions, {} connected clients). Reduce or cap session population before optimizing individual transcripts.",n(&population["final_live_sessions"]),n(&population["final_connected_clients"])));
    }
    for (key, threshold, prefix, suffix) in [
        (
            "total_provider_cache_json_bytes",
            0.35,
            "Provider cache is a large share of attributed state (",
            " of total JSON). Prioritize cache compaction, cache invalidation discipline, and avoiding redundant provider-message mirrors.",
        ),
        (
            "total_tool_result_bytes",
            0.25,
            "Tool results are heavy (",
            " of total JSON). Consider truncating stored tool output, summarizing verbose results, or storing large artifacts out-of-line.",
        ),
        (
            "total_large_blob_bytes",
            0.15,
            "Large blobs are materially retained (",
            " of total JSON). Focus on blob thresholds, attachment retention, and aggressive post-use slimming.",
        ),
        (
            "total_payload_text_bytes",
            0.45,
            "Transcript payload text dominates attributed state (",
            " of total JSON). Compaction and transcript summarization will likely pay off.",
        ),
    ] {
        if total > 0 && n(&sessions[key]) as f64 / total as f64 >= threshold {
            hints.push(format!(
                "{prefix}{:.0}%{suffix}",
                n(&sessions[key]) as f64 / total as f64 * 100.0
            ));
        }
    }
    let diagnostics = &samples.last().unwrap().raw["process_diagnostics"];
    let slack = n(&diagnostics["allocator_resident_minus_active_bytes"]);
    let gap = n(&diagnostics["pss_minus_allocator_allocated_bytes"]);
    if slack >= 64 * MB {
        hints.push(format!("Allocator resident slack is high ({} above active). Some memory pressure may be allocator retention rather than live app state.",mb(Some(slack))));
    }
    if gap >= 64 * MB {
        hints.push(format!("PSS is materially above allocator allocated ({} delta), suggesting shared mappings, allocator overhead, or retained pages are worth checking alongside app-owned structures.",mb(Some(gap))));
    }
    let embeddings = samples
        .iter()
        .filter(|s| {
            matches!(
                s.category.as_str(),
                "embedding_loaded" | "embedding_unloaded"
            )
        })
        .count();
    if embeddings > 0 {
        hints.push(format!("Embedding lifecycle events were observed ({embeddings} samples). Compare memory before/after those windows to decide whether local embeddings should unload more aggressively."));
    }
    if let Some(peak) = peaks.first() {
        hints.push(format!("Heaviest observed session was {} at {} attributed JSON. Start optimization work with that session’s transcript and tool-result profile.",s(&peak["session_id"]),mb(int(&peak["peak_json_bytes"]))));
    }
    if hints.is_empty() {
        hints.push("No single dominant culprit stood out yet. Collect more runtime history and compare multiple attribution samples after heavier real usage.".into());
    }
    hints
}
pub(super) fn client(samples: &[Sample], peaks: &[Value]) -> Vec<String> {
    let Some(last) = last_attr(samples).filter(|s| nonempty(s.totals())) else {
        return vec![
            "Need at least one client attribution sample before generating optimization hints."
                .into(),
        ];
    };
    let totals = last.totals();
    let total = n(&totals["total_attributed_bytes"]);
    let mut hints = vec![];
    for (key, threshold, prefix, suffix) in [
        (
            "display_messages_estimate_bytes",
            0.45,
            "Display-message state is a large share of attributed client memory (",
            "). Tighten UI duplication and display-history retention first.",
        ),
        (
            "provider_messages_json_bytes",
            0.20,
            "Resident provider-message copies are a meaningful share of client memory (",
            "). Prefer borrowing or lazy hydration where possible.",
        ),
        (
            "side_panel_estimate_bytes",
            0.15,
            "Side-panel state is materially retained (",
            " of attributed client memory). Focus on page content retention and render-cache discipline.",
        ),
        (
            "remote_state_bytes",
            0.10,
            "Remote session metadata is non-trivial (",
            " of attributed client memory). Review retained model/session lists and remote bootstrap state.",
        ),
    ] {
        if total > 0 && n(&totals[key]) as f64 / total as f64 >= threshold {
            hints.push(format!(
                "{prefix}{:.0}%{suffix}",
                n(&totals[key]) as f64 / total as f64 * 100.0
            ));
        }
    }
    let coverage = stats::coverage(last);
    let pss = n(&coverage["pss_bytes"]);
    let retained = n(&coverage["allocator_retained_resident_estimate_bytes"]);
    let missing = n(&coverage["unattributed_live_heap_bytes"]);
    let live = n(&coverage["allocator_live_bytes"]);
    if pss > 0 && retained as f64 / pss as f64 >= 0.30 {
        hints.push(format!("Allocator retention dominates PSS ({:.0}%, {}). This is freed-but-held heap, not live app state; malloc_trim/purge cadence matters more than estimator coverage here.",retained as f64/pss as f64*100.0,mb(Some(retained))));
    }
    if live > 0 && missing as f64 / live as f64 >= 0.40 {
        hints.push(format!("Estimators miss {:.0}% of live heap ({}). The real estimator gap is tokio/render/runtime structures, not allocator slack.",missing as f64/live as f64*100.0,mb(Some(missing))));
    }
    if let Some(peak) = peaks.first() {
        hints.push(format!("Heaviest observed client session was {} at {} attributed client memory. Start with that session’s display and provider-message layers.",s(&peak["session_id"]),mb(int(&peak["peak_total_attributed_bytes"]))));
    }
    if hints.is_empty() {
        hints.push("No single dominant client-side culprit stood out yet. Collect more client runtime history during heavier UI usage.".into());
    }
    hints
}
