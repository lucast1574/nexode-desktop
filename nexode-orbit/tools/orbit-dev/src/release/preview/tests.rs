use super::*;
use serde_json::json;
struct Fake {
    calls: Vec<Vec<String>>,
    directory: PathBuf,
    stale: bool,
    corrupt: &'static str,
    existing: bool,
    changed_tag: bool,
    sunset: usize,
    heads: usize,
    windows: usize,
    runs: Vec<Value>,
    refs: Vec<Value>,
    releases: Vec<Value>,
    failed_ci: bool,
}
impl Fake {
    fn new(path: &Path) -> Self {
        Self {
            calls: vec![],
            directory: path.into(),
            stale: false,
            corrupt: "",
            existing: false,
            changed_tag: false,
            sunset: usize::MAX,
            heads: 0,
            windows: 0,
            runs: vec![],
            refs: vec![],
            releases: vec![],
            failed_ci: false,
        }
    }
    fn edited(&self) -> bool {
        self.calls
            .iter()
            .any(|c| c.starts_with(&["release".into(), "edit".into()]))
    }
    fn dispatched(&self) -> bool {
        self.calls
            .iter()
            .any(|c| c.iter().any(|s| s.ends_with("/dispatches")))
    }
}
fn sha() -> String {
    "a".repeat(40)
}
const TAG: &str = "v1.0.0-preview.12";
impl Github for Fake {
    fn command(&mut self, args: &[String]) -> Result<String> {
        self.calls.push(args.to_vec());
        if args.starts_with(&["release".into(), "view".into()]) {
            let mut assets:Vec<Value>=fs::read_dir(&self.directory)?.map(|p|{let p=p.unwrap();json!({"name":p.file_name().to_str().unwrap(),"state":"uploaded","size":p.metadata().unwrap().len()})}).collect();
            match self.corrupt {
                "missing" => {
                    assets.pop();
                }
                "size" => {
                    assets[0]["size"] = json!(9999);
                }
                "state" => {
                    assets[0]["state"] = json!("new");
                }
                "duplicate" => {
                    assets[1] = assets[0].clone();
                }
                _ => {}
            }
            return Ok(json!({"isDraft":true,"isPrerelease":true,"assets":assets}).to_string());
        }
        Ok("{}".into())
    }
    fn api(&mut self, path: &str, paginated: bool) -> Result<Value> {
        self.calls.push(vec![path.into()]);
        if path.contains("/git/ref/heads/master") {
            self.heads += 1;
            return Ok(
                json!({"object":{"sha":if self.stale && self.heads>1 {"b".repeat(40)}else{sha()}}}),
            );
        }
        if path.contains("/git/ref/tags/") {
            return Ok(json!({"object":{"sha":if self.changed_tag {"b".repeat(40)}else{sha()}}}));
        }
        if path.contains("/git/matching-refs/tags/v1.0.0") {
            assert!(paginated);
            if path.contains("preview.") {
                return Ok(if self.existing {
                    json!([[{"ref":format!("refs/tags/{TAG}")} ]])
                } else {
                    json!([[]])
                });
            }
            self.windows += 1;
            return Ok(if self.windows >= self.sunset {
                json!([[{"ref":"refs/tags/v1.0.0"}]])
            } else {
                json!([[], self.refs])
            });
        }
        if path.contains("/preview.yml/runs?") {
            return Ok(json!([{"workflow_runs":[]},{"workflow_runs":self.runs}]));
        }
        if path.contains("/workflows/ci.yml/runs?") {
            return Ok(json!([{"workflow_runs":[ci_run()]}]));
        }
        if path.contains("/attempts/1/jobs?") {
            return Ok(
                json!([{"jobs":ci::MANDATORY.iter().map(|name|json!({"name":name,"status":"completed","conclusion":"success"})).collect::<Vec<_>>() }]),
            );
        }
        if path.contains("/actions/runs/100") {
            let mut run = ci_run();
            if self.failed_ci {
                run["conclusion"] = json!("cancelled");
            }
            return Ok(run);
        }
        if path.contains("/releases?") {
            return Ok(if self.releases.is_empty() {
                json!([[{"tag_name":"v0.99.1","draft":false,"prerelease":false}]])
            } else {
                json!([self.releases])
            });
        }
        if path.contains("/compare/") {
            return Ok(
                json!([{"total_commits":1,"commits":[{"sha":"b".repeat(40),"commit":{"message":"Fix race"}}]}]),
            );
        }
        anyhow::bail!("unexpected endpoint {path}")
    }
}
fn ci_run() -> Value {
    json!({"id":100,"run_number":10,"run_attempt":1,"head_sha":sha(),"event":"push","head_branch":"master","path":".github/workflows/ci.yml","status":"completed","conclusion":"success"})
}
fn fixture() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    for name in assets() {
        fs::write(tmp.path().join(name), b"archive bytes").unwrap();
    }
    tmp
}
#[test]
fn staging_checksums_and_final_head_gate() {
    let temp = fixture();
    let mut api = Fake::new(temp.path());
    publish(&mut api, REPOSITORY, &sha(), TAG, temp.path()).unwrap();
    let create = api
        .calls
        .iter()
        .find(|c| c.starts_with(&["release".into(), "create".into()]))
        .unwrap();
    for flag in ["--draft", "--prerelease", "--latest=false", "--verify-tag"] {
        assert!(create.iter().any(|s| s == flag));
    }
    let notes = &create[create.iter().position(|s| s == "--notes").unwrap() + 1];
    for text in [
        "## What changed",
        "Existing stable versions do not support",
        "SHA256SUMS",
        "extract the entire archive",
        "full path",
        "update --channel preview",
        "update --channel stable",
    ] {
        assert!(notes.contains(text));
    }
    assert!(api.calls[api.calls.len() - 2][0].ends_with("/git/ref/heads/master"));
    let edit = api.calls.last().unwrap();
    assert_eq!(&edit[..3], ["release", "edit", TAG]);
    assert!(edit.contains(&"--latest=false".into()));
    let checksums = fs::read_to_string(temp.path().join("SHA256SUMS")).unwrap();
    assert_eq!(checksums.lines().count(), 6);
    for line in checksums.lines() {
        let (digest, name) = line.split_once("  ").unwrap();
        assert_eq!(
            digest,
            format!(
                "{:x}",
                Sha256::digest(fs::read(temp.path().join(name)).unwrap())
            )
        );
    }
}
#[test]
fn publication_never_skips_failed_preconditions_or_staging_rechecks() {
    for mode in [
        "missing",
        "empty",
        "extra",
        "ci",
        "existing",
        "stale",
        "tag",
        "sunset-first",
        "sunset-final",
        "remote-missing",
        "remote-size",
        "remote-state",
        "remote-duplicate",
    ] {
        let temp = fixture();
        let mut api = Fake::new(temp.path());
        let asset = temp.path().join(assets().first().unwrap());
        match mode {
            "missing" => fs::remove_file(asset).unwrap(),
            "empty" => fs::write(asset, []).unwrap(),
            "extra" => fs::write(temp.path().join("unexpected"), b"x").unwrap(),
            "ci" => api.failed_ci = true,
            "existing" => api.existing = true,
            "stale" => api.stale = true,
            "tag" => api.changed_tag = true,
            "sunset-first" => api.sunset = 1,
            "sunset-final" => api.sunset = 2,
            _ => api.corrupt = mode.strip_prefix("remote-").unwrap(),
        }
        assert!(
            publish(&mut api, REPOSITORY, &sha(), TAG, temp.path()).is_err(),
            "{mode}"
        );
        assert!(!api.edited(), "{mode}");
        if ["missing", "empty", "extra"].contains(&mode) {
            assert!(api.calls.is_empty());
        }
        if ["ci", "existing", "sunset-first"].contains(&mode) {
            assert!(!api.calls.iter().any(|c| c.contains(&"POST".into())));
        }
    }
    for (repo, s, t) in [
        ("fork/nexode", sha(), TAG),
        (REPOSITORY, "abc".into(), TAG),
        (REPOSITORY, sha(), "v1.0.0"),
        (REPOSITORY, sha(), "v1.0.0-preview.0"),
        (REPOSITORY, sha(), "v1.0.0-preview.1;evil"),
    ] {
        assert!(validate(repo, &s, t).is_err());
    }
}
fn record(status: &str) -> Value {
    json!({"head_sha":sha(),"head_branch":"master","status":status})
}
#[test]
fn recovery_active_budget_ci_head_and_published_guards() {
    for status in ci::PENDING {
        let mut api = Fake::new(Path::new("unused"));
        api.runs = vec![record(status)];
        assert!(!reconcile(&mut api, REPOSITORY).unwrap());
        assert!(!api.dispatched());
    }
    for mode in [
        "limit",
        "stable",
        "published",
        "stale",
        "ci",
        "draft",
        "other-sha",
        "no-runs",
        "failed-preview",
    ] {
        let mut api = Fake::new(Path::new("unused"));
        match mode {
            "limit" => api.runs = vec![record("completed"); 3],
            "stable" => api.sunset = 1,
            "stale" => api.stale = true,
            "ci" => api.failed_ci = true,
            "failed-preview" => api.runs = vec![record("completed")],
            "other-sha" => {
                let mut r = record("completed");
                r["head_sha"] = json!("b".repeat(40));
                api.runs = vec![r; 4];
            }
            "published" | "draft" => {
                api.refs = vec![json!({"ref":format!("refs/tags/{TAG}"),"object":{"sha":sha()}})];
                api.releases = vec![
                    json!({"draft":mode=="draft","prerelease":true,"tag_name":TAG,"target_commitish":"master"}),
                ];
            }
            _ => {}
        }
        let expected = ["draft", "other-sha", "no-runs", "failed-preview"].contains(&mode);
        assert_eq!(reconcile(&mut api, REPOSITORY).unwrap(), expected, "{mode}");
        assert_eq!(api.dispatched(), expected);
        if expected {
            let last = api.calls.last().unwrap();
            assert!(last.ends_with(&[
                "--method".into(),
                "POST".into(),
                "-f".into(),
                "ref=master".into()
            ]));
            assert!(api.calls[api.calls.len() - 2][0].ends_with("/git/ref/heads/master"));
        }
    }
    let mut api = Fake::new(Path::new("unused"));
    assert!(reconcile(&mut api, "fork/nexode").is_err());
    assert!(api.calls.is_empty());
}
