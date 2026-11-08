use super::*;
use serde_json::json;
use std::io::Write;

struct Fixture {
    _temp: tempfile::TempDir,
    home: PathBuf,
    state: PathBuf,
    project: PathBuf,
    other: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let state = home.join(".nexode");
        for name in [
            "active_pids",
            "streaming_pids",
            "sessions",
            "session_leases",
        ] {
            fs::create_dir_all(state.join(name)).unwrap();
        }
        let project = home.join("project");
        fs::create_dir_all(project.join(".git")).unwrap();
        let other = home.join("other");
        fs::create_dir(&other).unwrap();
        Self {
            _temp: temp,
            home,
            state,
            project,
            other,
        }
    }
    fn session(&self, id: &str, path: &Path, pid: i32, busy: bool, extra: Value) {
        fs::write(self.state.join("active_pids").join(id), pid.to_string()).unwrap();
        if busy {
            fs::write(self.state.join("streaming_pids").join(id), pid.to_string()).unwrap();
        }
        let mut meta = extra;
        meta["working_dir"] = json!(path);
        self.write_session(id, &meta);
    }
    fn write_session(&self, id: &str, meta: &Value) {
        fs::write(
            self.state.join("sessions").join(format!("{id}.json")),
            serde_json::to_vec(meta).unwrap(),
        )
        .unwrap();
    }
    fn collect(&self, hint: Option<&str>) -> Result<Snapshot> {
        collect_with(
            &self.project,
            &self.home,
            &self.state,
            hint,
            |pid| pid == 123,
            MAX_ITEMS,
            Duration::from_secs(4),
        )
    }
}
fn count(s: &Snapshot, path: &str) -> (usize, usize) {
    let e = s.entries.iter().find(|e| e.path == path).unwrap();
    (e.agents, e.busy)
}
#[test]
fn shared_pid_counts_sessions_and_workspace_containers() {
    let f = Fixture::new();
    let nested = f.project.join("src");
    fs::create_dir(&nested).unwrap();
    f.session("root", &f.project, 123, true, json!({}));
    f.session(
        "worker.with.dot",
        &nested,
        123,
        true,
        json!({"parent_id":"root"}),
    );
    f.session("idle", &f.other, 123, false, json!({}));
    for container in ["workspaces", "projects", "repos"] {
        fs::create_dir_all(f.home.join(container).join("unused")).unwrap();
    }
    let s = f.collect(None).unwrap();
    assert_eq!(s.current_path.as_deref(), Some("~/project"));
    assert_eq!(s.entries[0].path, "~/project");
    assert_eq!(count(&s, "~/project"), (2, 2));
    assert_eq!(count(&s, "~/other"), (1, 0));
    assert!(!s.entries.iter().any(|e| e.path == "~/project/src"));
    for container in ["workspaces", "projects", "repos"] {
        assert_eq!(count(&s, &format!("~/{container}/unused")), (0, 0));
    }
}
#[test]
fn excludes_dead_debug_canary_and_moved_sessions() {
    let f = Fixture::new();
    f.session("dead", &f.project, 999, true, json!({}));
    f.session("debug", &f.project, 123, true, json!({"is_debug":true}));
    f.session("canary", &f.project, 123, true, json!({"is_canary":true}));
    f.session("away", &f.project, 123, true, json!({}));
    fs::write(
        f.state.join("session_leases/away.json"),
        serde_json::to_vec(&json!({"away_host":"local","repo_root":f.project})).unwrap(),
    )
    .unwrap();
    assert_eq!(count(&f.collect(None).unwrap(), "~/project"), (0, 0));
}
#[test]
fn stale_streaming_marker_is_idle() {
    let f = Fixture::new();
    f.session("idle", &f.project, 123, false, json!({}));
    fs::write(f.state.join("streaming_pids/idle"), "999").unwrap();
    assert_eq!(count(&f.collect(None).unwrap(), "~/project"), (1, 0));
}
#[test]
fn restored_roots_include_empty_siblings() {
    let f = Fixture::new();
    let restored = f._temp.path().join("restored/repo");
    fs::create_dir_all(&restored).unwrap();
    let sibling = restored.parent().unwrap().join("empty");
    fs::create_dir(&sibling).unwrap();
    fs::write(
        f.state.join("session_leases/old.json"),
        serde_json::to_vec(&json!({"away_host":"local","repo_root":restored})).unwrap(),
    )
    .unwrap();
    let s = f.collect(None).unwrap();
    assert_eq!(count(&s, restored.to_str().unwrap()), (0, 0));
    assert_eq!(count(&s, sibling.to_str().unwrap()), (0, 0));
}
#[test]
fn missing_metadata_is_pending_and_corruption_fails() {
    let f = Fixture::new();
    fs::write(f.state.join("active_pids/starting"), "123").unwrap();
    assert_eq!(
        count(&f.collect(None).unwrap(), "(directory pending)"),
        (1, 0)
    );
    assert_eq!(
        count(&f.collect(Some("starting")).unwrap(), "~/project"),
        (1, 0)
    );
    fs::write(f.state.join("sessions/starting.json"), "{").unwrap();
    assert!(f.collect(None).is_err());
}
#[cfg(unix)]
#[test]
fn filenames_and_aliases_are_data() {
    let f = Fixture::new();
    let odd = f.home.join("雲 ' $(not-a-command)\nproject");
    fs::create_dir(&odd).unwrap();
    std::os::unix::fs::symlink(&odd, f.home.join("alias")).unwrap();
    f.session("odd", &odd, 123, false, json!({}));
    let s = f.collect(None).unwrap();
    assert_eq!(count(&s, "~/雲 ' $(not-a-command)\nproject"), (1, 0));
    assert!(!s.entries.iter().any(|e| e.path == "~/alias"));
}
#[test]
fn large_snapshot_skips_transcript_and_nested_metadata() {
    let f = Fixture::new();
    f.session("large", &f.project, 123, false, json!({}));
    let mut file = File::create(f.state.join("sessions/large.json")).unwrap();
    file.write_all(b"{\"messages\":[{\"working_dir\":\"/wrong\",\"is_debug\":true,\"content\":\"")
        .unwrap();
    let chunk = vec![b'x'; 1024 * 1024];
    for _ in 0..33 {
        file.write_all(&chunk).unwrap();
    }
    write!(
        file,
        "\"}}],\"working_dir\":{},\"is_debug\":false}}",
        serde_json::to_string(&f.project).unwrap()
    )
    .unwrap();
    assert_eq!(count(&f.collect(None).unwrap(), "~/project"), (1, 0));
}
#[test]
fn latest_complete_journal_metadata_wins() {
    let f = Fixture::new();
    f.session("journal", &f.project, 123, false, json!({}));
    fs::write(
        f.state.join("sessions/journal.journal.jsonl"),
        format!("{}\n{{\"meta\":", json!({"meta":{"working_dir":f.other}})),
    )
    .unwrap();
    let s = f.collect(None).unwrap();
    assert_eq!(count(&s, "~/other"), (1, 0));
    assert_eq!(count(&s, "~/project"), (0, 0));
}
#[test]
fn nested_fields_cannot_spoof_metadata() {
    let f = Fixture::new();
    f.session("nested", &f.project, 123, false, json!({"messages":[{"working_dir":"/wrong","is_debug":true,"content":"{\"working_dir\":\"/wrong\"} \\\""}]}));
    assert_eq!(count(&f.collect(None).unwrap(), "~/project"), (1, 0));
}
#[test]
fn registry_overflow_and_expired_deadline_fail() {
    let f = Fixture::new();
    f.session("one", &f.project, 123, false, json!({}));
    f.session("two", &f.project, 123, false, json!({}));
    assert!(
        collect_with(
            &f.project,
            &f.home,
            &f.state,
            None,
            |_| true,
            1,
            Duration::from_secs(4)
        )
        .is_err()
    );
    assert!(
        collect_with(
            &f.project,
            &f.home,
            &f.state,
            None,
            |_| true,
            MAX_ITEMS,
            Duration::ZERO
        )
        .is_err()
    );
}
#[test]
fn snapshot_rejects_trailing_garbage() {
    let f = Fixture::new();
    f.session("bad", &f.project, 123, false, json!({}));
    let path = f.state.join("sessions/bad.json");
    let mut s = fs::read_to_string(&path).unwrap();
    s.push_str("bad");
    fs::write(path, s).unwrap();
    assert!(f.collect(None).is_err());
}
