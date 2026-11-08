use super::*;
use std::io::Write;
#[test]
fn namespace_guard_rejects_host_before_preparing_profile() {
    assert!(isolated(123, "net:[1]", "net:[2]").is_err());
    assert!(isolated(1, "net:[1]", "net:[1]").is_err());
    assert!(isolated(1, "net:[1]", "").is_err());
}
#[test]
fn isolated_profile_preserves_bridge_manifest_and_rejects_path_escape() {
    let t = tempfile::tempdir().unwrap();
    let source = t.path().join("source");
    fs::create_dir(&source).unwrap();
    for name in ["browser", "firefox-agent-bridge-host"] {
        fs::write(source.join(name), "fixture").unwrap();
    }
    let archive = |id: &str| {
        let file = fs::File::create(source.join("browser-agent-bridge.xpi")).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file("manifest.json", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(
            json!({"version":"1.2.3","browser_specific_settings":{"gecko":{"id":id}}})
                .to_string()
                .as_bytes(),
        )
        .unwrap();
        zip.finish().unwrap();
    };
    archive("fixture@example.test");
    let root = t.path().join("isolated");
    assert_eq!(prepare(&root, &source).unwrap()["version"], "1.2.3");
    assert!(
        root.join("home/.mozilla/firefox/acceptance.default/extensions/fixture@example.test.xpi")
            .is_file()
    );
    let host: Value = serde_json::from_slice(
        &fs::read(root.join("home/.mozilla/native-messaging-hosts/firefox_agent_bridge.json"))
            .unwrap(),
    )
    .unwrap();
    assert!(
        host["path"]
            .as_str()
            .unwrap()
            .starts_with(root.to_str().unwrap())
    );
    assert!(environment(&root).iter().all(|(k, _)| !k.contains("TOKEN")));
    archive("../../escape");
    assert!(prepare(&t.path().join("rejected"), &source).is_err());
    assert!(!t.path().join("escape.xpi").exists());
}
#[cfg(unix)]
#[test]
fn acceptance_requires_exact_ready_empty_browser_and_all_exit_codes() {
    use std::os::unix::process::ExitStatusExt;
    let out = |code, text: &str| std::process::Output {
        status: std::process::ExitStatus::from_raw(code << 8),
        stdout: text.as_bytes().to_vec(),
        stderr: vec![],
    };
    let setup = out(0, "Firefox was launched and the browser bridge reconnected");
    let status = out(0, "Built-in browser tool is ready");
    let tabs = out(
        0,
        r#"{"totalTabs":1,"windows":[{"tabs":[{"url":"about:blank"}]}]}"#,
    );
    assert!(passed(&setup, &status, &tabs));
    assert!(!passed(&out(0, "extension install prompt"), &status, &tabs));
    assert!(!passed(
        &setup,
        &out(1, "Built-in browser tool is ready"),
        &tabs
    ));
    assert!(!passed(&setup, &status, &out(0, "{}")));
}
