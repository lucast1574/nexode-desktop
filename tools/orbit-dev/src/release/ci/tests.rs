use super::*;
use serde_json::json;
fn run() -> Value {
    json!({"id":100,"run_number":10,"run_attempt":1,"head_sha":"a".repeat(40),"event":"push","head_branch":"master","path":".github/workflows/ci.yml","status":"completed","conclusion":"success"})
}
fn jobs() -> Vec<Value> {
    MANDATORY
        .iter()
        .map(|n| json!({"name":n,"status":"completed","conclusion":"success","run_attempt":1}))
        .collect()
}
struct Fake {
    runs: Value,
    detail: Value,
    jobs: Value,
    confirmation: Value,
    calls: Vec<String>,
    details: usize,
}
impl Default for Fake {
    fn default() -> Self {
        Self {
            runs: json!([{"workflow_runs":[run()]}]),
            detail: run(),
            jobs: json!([{"jobs":jobs()}]),
            confirmation: run(),
            calls: vec![],
            details: 0,
        }
    }
}
impl Github for Fake {
    fn command(&mut self, _: &[String]) -> Result<String> {
        unreachable!()
    }
    fn api(&mut self, path: &str, page: bool) -> Result<Value> {
        self.calls.push(path.into());
        if path.contains("/workflows/") {
            assert!(page);
            Ok(self.runs.clone())
        } else if path.contains("/jobs?") {
            assert!(page);
            Ok(self.jobs.clone())
        } else {
            self.details += 1;
            Ok(if self.details > 1 {
                self.confirmation.clone()
            } else {
                self.detail.clone()
            })
        }
    }
}
fn check(api: &mut Fake, preview: bool) -> Result<Option<String>> {
    check_once("owner/repo", &"a".repeat(40), api, preview)
}
fn advisory_api(step: &str) -> Fake {
    let mut api = Fake::default();
    api.detail["conclusion"] = json!("failure");
    api.confirmation = api.detail.clone();
    api.jobs[0]["jobs"][0]["conclusion"] = json!("failure");
    api.jobs[0]["jobs"][0]["steps"] = json!([{"name":"Run clippy","status":"completed","conclusion":"success"},{"name":step,"status":"completed","conclusion":"failure"}]);
    api
}
#[test]
fn exact_identity_latest_attempt_and_race() {
    let mut api = Fake::default();
    assert_eq!(check(&mut api, false).unwrap(), None);
    assert!(api.calls[0].contains(&format!("head_sha={}&event=push", "a".repeat(40))));
    assert!(api.calls[2].contains("/attempts/1/jobs?"));
    for (key, v) in [
        ("head_sha", json!("b".repeat(40))),
        ("head_branch", json!("feature")),
        ("event", json!("pull_request")),
    ] {
        let mut api = Fake::default();
        api.runs[0]["workflow_runs"][0][key] = v;
        assert!(
            check(&mut api, false)
                .unwrap()
                .unwrap()
                .contains("missing exact-SHA")
        );
    }
    for (key, v) in [
        ("head_sha", json!("b".repeat(40))),
        ("id", json!(101)),
        ("event", json!("pull_request")),
        ("path", json!("other.yml")),
        ("run_attempt", json!(0)),
        ("run_number", json!(true)),
    ] {
        let mut api = Fake::default();
        api.detail[key] = v;
        assert!(check(&mut api, false).is_err());
    }
    let mut api = Fake::default();
    api.confirmation["run_attempt"] = json!(2);
    assert!(
        check(&mut api, false)
            .unwrap()
            .unwrap()
            .contains("attempt changed")
    );
    let mut api = Fake::default();
    let mut newer = run();
    newer["id"] = json!(101);
    newer["run_number"] = json!(11);
    newer["conclusion"] = json!("failure");
    api.runs[0]["workflow_runs"]
        .as_array_mut()
        .unwrap()
        .push(newer.clone());
    api.detail = newer;
    assert!(check(&mut api, false).is_err());
    assert!(api.calls[1].ends_with("/runs/101"));
    let mut api = Fake::default();
    api.detail["run_attempt"] = json!(2);
    api.confirmation = api.detail.clone();
    for job in api.jobs[0]["jobs"].as_array_mut().unwrap() {
        job["run_attempt"] = json!(2);
    }
    assert_eq!(check(&mut api, false).unwrap(), None);
    assert!(api.calls[2].contains("/attempts/2/"));
}
#[test]
fn all_mandatory_jobs_and_states_fail_closed() {
    for index in 0..MANDATORY.len() {
        let mut api = Fake::default();
        api.jobs[0]["jobs"].as_array_mut().unwrap().remove(index);
        assert!(check(&mut api, false).is_err());
        for conclusion in ["failure", "cancelled", "skipped", "neutral", "timed_out"] {
            let mut api = Fake::default();
            api.jobs[0]["jobs"][index]["conclusion"] = json!(conclusion);
            assert!(check(&mut api, false).is_err());
        }
        let mut api = Fake::default();
        api.jobs[0]["jobs"][index]["status"] = json!("queued");
        api.jobs[0]["jobs"][index]["conclusion"] = Value::Null;
        assert!(check(&mut api, false).unwrap().is_some());
        for (field, value) in [
            ("run_attempt", json!(2)),
            ("run_id", json!(101)),
            ("head_sha", json!("b".repeat(40))),
        ] {
            let mut api = Fake::default();
            api.jobs[0]["jobs"][index][field] = value;
            assert!(check(&mut api, false).is_err());
        }
    }
    let mut api = Fake::default();
    let duplicate = api.jobs[0]["jobs"][0].clone();
    api.jobs[0]["jobs"].as_array_mut().unwrap().push(duplicate);
    assert!(check(&mut api, false).is_err());
    let mut api = Fake::default();
    for job in api.jobs[0]["jobs"].as_array_mut().unwrap() {
        job.as_object_mut().unwrap().remove("run_attempt");
        job["run_id"] = json!(100);
        job["head_sha"] = json!("a".repeat(40));
    }
    api.detail["path"] = json!(".github/workflows/ci.yml@master");
    assert_eq!(check(&mut api, false).unwrap(), None);
    for conclusion in [
        "failure",
        "cancelled",
        "timed_out",
        "action_required",
        "skipped",
        "neutral",
        "stale",
        "error",
    ] {
        let mut api = Fake::default();
        api.detail["conclusion"] = json!(conclusion);
        assert!(check(&mut api, false).is_err());
        assert_eq!(api.calls.len(), 2);
    }
    let mut api = Fake::default();
    api.detail["status"] = json!("in_progress");
    api.detail["conclusion"] = Value::Null;
    api.jobs = json!([{"jobs":[]}]);
    assert!(
        check(&mut api, false)
            .unwrap()
            .unwrap()
            .contains("missing mandatory job")
    );
}
#[test]
fn preview_advisory_only_with_complete_correctness_evidence() {
    for step in ADVISORY {
        let mut api = advisory_api(step);
        assert!(check(&mut api, true).unwrap().is_none());
        assert!(check(&mut advisory_api(step), false).is_err());
    }
    for step in [
        "Check all targets and all features",
        "Run clippy with warnings denied",
        "Enforce Cargo.lock is up to date",
        "Enforce panic-prone usage ratchet",
        "Enforce swallowed-error usage ratchet",
        "unknown",
    ] {
        assert!(check(&mut advisory_api(step), true).is_err());
    }
    for evidence in [
        Value::Null,
        json!([]),
        json!([null]),
        json!([{"name":"Check formatting","status":"completed","conclusion":"success"}]),
    ] {
        let mut api = advisory_api("Check formatting");
        api.jobs[0]["jobs"][0]["steps"] = evidence;
        assert!(check(&mut api, true).is_err());
    }
    for conclusion in ["skipped", "cancelled", "timed_out"] {
        let mut api = advisory_api("Check formatting");
        api.jobs[0]["jobs"][0]["steps"]
            .as_array_mut()
            .unwrap()
            .push(json!({"name":"Run tests","status":"completed","conclusion":conclusion}));
        assert!(check(&mut api, true).is_err());
    }
    for name in MANDATORY {
        if advisory(name).is_empty() {
            continue;
        }
        let mut job = json!({"name":name,"status":"completed","conclusion":"failure","steps":[{"name":advisory(name)[0],"status":"completed","conclusion":"failure"}]});
        for step in optional(name) {
            job["steps"]
                .as_array_mut()
                .unwrap()
                .push(json!({"name":step,"status":"completed","conclusion":"skipped"}));
        }
        assert!(job_status(&job, true).unwrap().is_none());
        job["steps"]
            .as_array_mut()
            .unwrap()
            .push(json!({"name":"Run tests","status":"completed","conclusion":"skipped"}));
        assert!(job_status(&job, true).is_err());
    }
    let mut api = Fake::default();
    api.detail["conclusion"] = json!("failure");
    assert!(check(&mut api, true).is_err());
    let mut api = advisory_api("Check formatting");
    api.jobs[0]["jobs"]
        .as_array_mut()
        .unwrap()
        .push(json!({"name":"Security","status":"completed","conclusion":"failure"}));
    assert!(check(&mut api, true).is_err());
    let job = json!({"name":"Build & Test (ubuntu-latest)","status":"completed","conclusion":"failure","steps":[{"name":"Enforce warning budget (Linux, macOS)","status":"completed","conclusion":"failure"},{"name":"Security preflight (Linux)","status":"completed","conclusion":"skipped"}]});
    assert!(job_status(&job, true).is_err());
}
#[test]
fn malformed_inputs_pages_timeout_and_cli() {
    for value in [
        Value::Null,
        json!({}),
        json!([]),
        json!([{"workflow_runs":null}]),
        json!([{"workflow_runs":[null]}]),
    ] {
        let mut api = Fake {
            runs: value,
            ..Default::default()
        };
        assert!(check(&mut api, false).is_err());
    }
    for repo in [
        "../repo",
        "owner/repo?token=x",
        "owner/repo/more",
        "-owner/repo",
    ] {
        assert!(validate(repo, &"a".repeat(40), 0.0, 0.0).is_err());
    }
    for sha in [
        "a".repeat(39),
        "g".repeat(40),
        "main".into(),
        format!("{}\n", "a".repeat(40)),
    ] {
        assert!(validate("o/r", &sha, 0.0, 0.0).is_err());
    }
    for invalid in [-1.0, f64::INFINITY, f64::NAN] {
        assert!(validate("o/r", &"a".repeat(40), invalid, 0.0).is_err());
        assert!(validate("o/r", &"a".repeat(40), 0.0, invalid).is_err());
    }
    assert!(validate("o/r", &"A".repeat(40), 0.0, 0.0).is_ok());
    let mut api = Fake {
        runs: json!([{"workflow_runs":[]}]),
        ..Default::default()
    };
    assert!(
        authorize("o/r", &"a".repeat(40), 0.0, 30.0, &mut api, false)
            .unwrap_err()
            .is::<WaitExpired>()
    );
    let start = Instant::now();
    assert!(
        authorize("o/r", &"a".repeat(40), 0.02, 30.0, &mut api, false)
            .unwrap_err()
            .is::<WaitExpired>()
    );
    assert!(start.elapsed() < Duration::from_secs(1));
    assert!(Args::try_parse_from(["ci", "--repo", "o/r"]).is_err());
    assert!(Args::try_parse_from(["ci", "sha", "--sha", "sha", "--repo", "o/r"]).is_err());
    assert!(
        Args::try_parse_from(["ci", "sha", "--repo", "o/r", "--preview"])
            .unwrap()
            .preview
    );
}
