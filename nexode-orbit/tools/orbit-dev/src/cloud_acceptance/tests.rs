use super::*;
#[test]
fn failure_reports_preserve_xml_and_json_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let results = vec![
        json!({"name":"pass","seconds":1.0,"status":"passed","artifacts":"one"}),
        json!({"name":"fail","seconds":2.0,"status":"failed","artifacts":"two","error":"outage <&\"","traceback":"details"}),
    ];
    write_report(dir.path(), &results, Path::new("/binary"), "version").unwrap();
    let value: Value =
        serde_json::from_slice(&fs::read(dir.path().join("report.json")).unwrap()).unwrap();
    assert_eq!(value["results"][1]["status"], "failed");
    let text = fs::read_to_string(dir.path().join("junit.xml")).unwrap();
    let doc = roxmltree::Document::parse(&text).unwrap();
    let root = doc.root_element();
    assert_eq!(root.attribute("tests"), Some("2"));
    assert_eq!(root.attribute("failures"), Some("1"));
    assert_eq!(
        root.descendants()
            .find(|n| n.has_tag_name("failure"))
            .unwrap()
            .attribute("message"),
        Some("outage <&\"")
    );
}
#[test]
fn child_failures_and_timeouts_are_failures() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("child.log");
    assert!(
        child(
            Command::new("sh").args(["-c", "echo evidence; exit 7"]),
            &log,
            Duration::from_secs(5)
        )
        .unwrap_err()
        .to_string()
        .contains("exited")
    );
    assert!(fs::read_to_string(&log).unwrap().contains("evidence"));
    assert!(
        child(
            Command::new("sh").args(["-c", "echo before-timeout; sleep 60"]),
            &log,
            Duration::from_millis(50)
        )
        .unwrap_err()
        .to_string()
        .contains("timed out")
    );
    assert!(fs::read_to_string(&log).unwrap().contains("before-timeout"));
}
#[test]
fn attachment_requires_destination_header_and_exact_session() {
    for cloud in [true, false] {
        let destination = if cloud {
            "server: SSH cloud-acceptance"
        } else {
            "server: local"
        };
        let outgoing = if cloud {
            "server: local"
        } else {
            "server: SSH cloud-acceptance"
        };
        let state = json!({"remote_session_id":"moved-session"});
        assert!(!tui::attached(outgoing, &state, "moved-session", cloud));
        assert!(!tui::attached(
            destination,
            &json!({"remote_session_id":"other"}),
            "moved-session",
            cloud
        ));
        assert!(tui::attached(destination, &state, "moved-session", cloud));
    }
}
#[test]
fn only_fresh_agent_turns_count_and_partial_lines_are_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("model.jsonl");
    fs::write(&path,format!("{}\n{}\n{{\"partial\":",json!({"agent":false,"messages":[{"role":"user","content":"needle"}]}),json!({"agent":true,"messages":[{"role":"user","content":"needle"},{"role":"user","content":"new continuation"}]}))).unwrap();
    let records = tui::records(&path).unwrap();
    assert_eq!(records.len(), 1);
    assert!(!tui::fresh_prompt(&records[0], "needle"));
    assert!(tui::fresh_prompt(&records[0], "new continuation"));
}
