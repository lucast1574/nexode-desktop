use super::*;
use serde_json::json;
fn write(p: &Path, v: Value) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, serde_json::to_vec(&v).unwrap()).unwrap();
}
#[test]
fn nested_overrides_partial_artifacts_and_logs_are_not_silently_accepted() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let run = root.join("harbor-jobs/task/run");
    write(
        &root.join("campaign.json"),
        json!({"timeout_multiplier":1.0,"attempts_per_task":5}),
    );
    write(
        &run.join("config.json"),
        json!({"timeout_multiplier":1.0,"agent":[{"override_cpus":null,"suppress_override_warnings":true}]}),
    );
    for i in 0..5 {
        let trial = run.join(format!("trial__{i}"));
        write(&trial.join("result.json"), json!({"reward":1}));
        fs::write(trial.join("events.ndjson"), "clean").unwrap();
    }
    assert_eq!(audit(root, 5).unwrap().ready.len(), 1);
    write(
        &run.join("config.json"),
        json!({"timeout_multiplier":1.0,"agent":[{"override_cpus":0}]}),
    );
    let r = audit(root, 5).unwrap();
    assert_eq!(r.failures.len(), 1);
    assert!(r.failures[0].contains("agent[0].override_cpus"));
    assert!(r.ready.is_empty());
    fs::write(run.join("trial__0/result.json"), "broken").unwrap();
    fs::remove_file(run.join("trial__1/events.ndjson")).unwrap();
    fs::write(
        run.join("trial__2/events.ndjson"),
        "Visit TERMINAL-BENCH.ORG",
    )
    .unwrap();
    let r = audit(root, 5).unwrap();
    assert_eq!(r.failures.len(), 3);
    assert_eq!(r.warnings.len(), 1);
    fs::remove_dir_all(run.join("trial__4")).unwrap();
    let r = audit(root, 5).unwrap();
    assert_eq!(r.partial.len(), 1);
    assert!(r.ready.is_empty());
}
#[test]
fn no_jobs_and_invalid_trial_limits_fail() {
    let tmp = tempfile::tempdir().unwrap();
    assert!(audit(tmp.path(), 5).is_err());
    fs::create_dir(tmp.path().join("harbor-jobs")).unwrap();
    assert!(audit(tmp.path(), 0).is_err());
    assert_eq!(audit(tmp.path(), 5).unwrap().warnings.len(), 1);
}
