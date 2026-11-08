use super::*;
#[test]
fn actual_formats_cache_counts_and_final_trace() {
    let c=claude(br#"{"usage":{"input_tokens":100,"output_tokens":20,"cache_read_input_tokens":30,"cache_creation_input_tokens":50},"total_cost_usd":0.01,"duration_ms":123}"#).unwrap();
    assert_eq!(c.total().unwrap(), 200);
    assert_eq!(c.cost, Some(0.01));
    let n=nexode("noise\n[trace] token_usage input=999 output=1 cache_read=0 cache_write=0\n[trace] token_usage input=110 output=20 cache_read=30 cache_write=50").unwrap();
    let r = comparison(&c, &n).unwrap();
    assert_eq!(r["diff"]["total"], 10);
    assert!((r["pct_diff"]["total"].as_f64().unwrap() - 5.0).abs() < 0.0001);
    assert!(nexode("no trace").is_err());
    assert!(nexode("[trace] token_usage input=-1 output=2").is_err());
    assert!(claude(br#"{"is_error":true}"#).is_err());
    assert!(claude(br#"{"usage":{}}"#).is_err());
}
#[test]
fn failures_zero_evidence_and_threshold_are_enforced_in_json_too() {
    let entry = |c, n| json!({"comparison":{"claude":{"total":c},"nexode":{"total":n}}});
    assert!(acceptable(&[entry(100, 150)], 1));
    assert!(!acceptable(&[entry(100, 151)], 1));
    assert!(!acceptable(&[entry(100, 10)], 2));
    assert!(!acceptable(&[entry(0, 0)], 1));
    assert!(!acceptable(&[], 0));
    assert!(acceptable(&[entry(100, 5)], 1));
    let c = Usage::default();
    let n = Usage {
        input: 1,
        ..Default::default()
    };
    assert!(comparison(&c, &n).unwrap()["pct_diff"]["input"].is_null());
}
