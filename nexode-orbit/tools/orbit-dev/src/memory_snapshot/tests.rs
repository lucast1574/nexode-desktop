use super::*;
#[test]
fn proc_fixture_roles_totals_disappearing_processes_and_spaced_paths() {
    let tmp = tempfile::tempdir().unwrap();
    for (pid, argv) in [
        (
            10,
            vec!["/build/nexode", "--socket", "/run/main socket", "serve"],
        ),
        (
            20,
            vec![
                "nexode",
                "--resume",
                "session_a",
                "--socket=/run/main socket",
            ],
        ),
        (30, vec!["nexode", "--socket", "/other", "serve"]),
        (40, vec!["sh", "nexode serve"]),
        (50, vec!["nexode", "login"]),
        (60, vec!["nexode", "--fresh-spawn"]),
    ] {
        let dir = tmp.path().join(pid.to_string());
        fs::create_dir(&dir).unwrap();
        fs::write(dir.join("cmdline"), argv.join("\0")).unwrap();
        fs::write(dir.join("smaps_rollup"),"header\nRss: 2048 kB\nPss: 1024 kB\nAnonymous: 512 kB\nPrivate_Dirty: 256 kB\nSwap: 0 kB\n").unwrap();
    }
    fs::create_dir(tmp.path().join("70")).unwrap();
    let r = snapshot(tmp.path(), "/run/main socket", false).unwrap();
    assert_eq!(r["totals"]["primary"]["count"], 3);
    assert_eq!(r["totals"]["primary"]["rss_mb"], 6.0);
    assert_eq!(r["totals"]["primary"]["pss_mb"], 3.0);
    assert_eq!(r["clients"][0]["session_id"], "session_a");
    assert_eq!(r["server"][0]["socket_path"], "/run/main socket");
    assert_eq!(r["auxiliary"], json!([]));
    let r = snapshot(tmp.path(), "/run/main socket", true).unwrap();
    assert_eq!(r["auxiliary"][0]["role"], "server_aux");
    assert_eq!(r["totals"]["auxiliary"]["pss_mb"], 1.0);
    assert_eq!(r["totals"]["primary"]["pss_mb"], 3.0);
}
#[test]
fn bad_smaps_fails_instead_of_fabricating_measurements() {
    assert!(smaps("Rss: invalid kB").is_err());
    assert!(smaps("Pss: 20 MB").is_err());
    assert_eq!(smaps("Pss: 128 kB").unwrap()["pss_mb"], 0.1);
}
