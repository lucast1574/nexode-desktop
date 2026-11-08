use super::*;
fn fixture(lane: &str) -> (tempfile::TempDir, BTreeMap<String, String>) {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    fs::write(root.join("Cargo.lock"), "frozen dependencies").unwrap();
    fs::create_dir(root.join("handoff")).unwrap();
    fs::create_dir_all(root.join("justrust/runs/test-run")).unwrap();
    fs::write(
        root.join("justrust/runs/test-run/meta.json"),
        r#"{"place":"hosted"}"#,
    )
    .unwrap();
    let mut bytes = vec![0u8; 64];
    if lane == "linux-arm64" {
        bytes[..6].copy_from_slice(b"\x7fELF\x02\x01");
        bytes[16] = 2;
        bytes[18] = 183;
    } else {
        bytes[..4].copy_from_slice(b"\xcf\xfa\xed\xfe");
        bytes[4..8].copy_from_slice(&0x0100000cu32.to_le_bytes());
        bytes[12] = 2;
    }
    fs::write(root.join("handoff/nexode"), bytes).unwrap();
    let command: Vec<_> = contract(lane).unwrap()["command"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|a| *a != "--remote" && *a != "--local")
        .cloned()
        .collect();
    fs::write(root.join("handoff/build-report.json"),json!({"id":"test-run","exit":0,"args":command,"remote":if lane=="linux-arm64"{json!({"machine":"hosted"})}else{Value::Null}}).to_string()).unwrap();
    let env = BTreeMap::from([
        ("GITHUB_SHA".into(), "a".repeat(40)),
        ("GITHUB_REPOSITORY".into(), REPOSITORY.into()),
        ("GITHUB_REF".into(), "refs/heads/master".into()),
        ("GITHUB_RUN_ID".into(), "123".into()),
        ("GITHUB_RUN_ATTEMPT".into(), "1".into()),
        (
            "JUSTRUST_HOME".into(),
            root.join("justrust").to_str().unwrap().into(),
        ),
    ]);
    (tmp, env)
}
#[test]
fn both_lanes_bind_source_run_attempt_contract_and_exact_bytes() {
    for lane in ["linux-arm64", "macos-arm64"] {
        let (tmp, env) = fixture(lane);
        let dir = tmp.path().join("handoff");
        let hash = seal(tmp.path(), &dir, lane, &env).unwrap();
        verify(tmp.path(), &dir, lane, &hash, &env).unwrap();
        for key in [
            "GITHUB_SHA",
            "GITHUB_RUN_ID",
            "GITHUB_RUN_ATTEMPT",
            "GITHUB_REPOSITORY",
            "GITHUB_REF",
        ] {
            let mut changed = env.clone();
            changed.insert(
                key.into(),
                if key == "GITHUB_SHA" {
                    "b".repeat(40)
                } else {
                    "other".into()
                },
            );
            assert!(verify(tmp.path(), &dir, lane, &hash, &changed).is_err());
        }
        assert!(verify(tmp.path(), &dir, lane, &"0".repeat(64), &env).is_err());
        fs::write(tmp.path().join("Cargo.lock"), "changed").unwrap();
        assert!(verify(tmp.path(), &dir, lane, &hash, &env).is_err());
    }
}
#[test]
fn manifest_and_receipt_tampering_even_with_updated_hash_rejected() {
    let (tmp, env) = fixture("linux-arm64");
    let dir = tmp.path().join("handoff");
    seal(tmp.path(), &dir, "linux-arm64", &env).unwrap();
    let path = dir.join("provenance.json");
    let original = fs::read(&path).unwrap();
    for key in [
        "toolchain",
        "justrust_revision",
        "contract",
        "features",
        "schema",
    ] {
        let mut v: Value = serde_json::from_slice(&original).unwrap();
        v[key] = json!("untrusted");
        fs::write(&path, v.to_string()).unwrap();
        assert!(
            verify(
                tmp.path(),
                &dir,
                "linux-arm64",
                &digest(&path).unwrap(),
                &env
            )
            .is_err()
        );
    }
    fs::write(&path, original).unwrap();
    let hash = digest(&path).unwrap();
    let report = dir.join("build-report.json");
    fs::write(&report, fs::read_to_string(&report).unwrap() + "\n").unwrap();
    assert!(verify(tmp.path(), &dir, "linux-arm64", &hash, &env).is_err());
    for (key, value) in [
        ("exit", json!(1)),
        ("args", json!(["build"])),
        ("remote", Value::Null),
        ("id", json!("../escape")),
    ] {
        let (tmp, env) = fixture("linux-arm64");
        let dir = tmp.path().join("handoff");
        let path = dir.join("build-report.json");
        let mut v: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        v[key] = value;
        fs::write(path, v.to_string()).unwrap();
        assert!(seal(tmp.path(), &dir, "linux-arm64", &env).is_err());
    }
    let (tmp, env) = fixture("linux-arm64");
    fs::write(
        tmp.path().join("justrust/runs/test-run/meta.json"),
        r#"{"place":"remote"}"#,
    )
    .unwrap();
    assert!(seal(tmp.path(), &tmp.path().join("handoff"), "linux-arm64", &env).is_err());
}
#[test]
fn architecture_truncation_binary_mutation_and_symlinks() {
    let (tmp, env) = fixture("macos-arm64");
    let dir = tmp.path().join("handoff");
    assert!(seal(tmp.path(), &dir, "linux-arm64", &env).is_err());
    fs::write(dir.join("nexode"), b"\x7fELF").unwrap();
    assert!(seal(tmp.path(), &dir, "linux-arm64", &env).is_err());
    let (tmp, env) = fixture("linux-arm64");
    let dir = tmp.path().join("handoff");
    let hash = seal(tmp.path(), &dir, "linux-arm64", &env).unwrap();
    let mut bytes = fs::read(dir.join("nexode")).unwrap();
    bytes.extend_from_slice(b"tampered");
    fs::write(dir.join("nexode"), bytes).unwrap();
    assert!(verify(tmp.path(), &dir, "linux-arm64", &hash, &env).is_err());
    #[cfg(unix)]
    {
        fs::remove_file(dir.join("nexode")).unwrap();
        std::os::unix::fs::symlink(tmp.path().join("Cargo.lock"), dir.join("nexode")).unwrap();
        assert!(seal(tmp.path(), &dir, "linux-arm64", &env).is_err());
        let (tmp, env) = fixture("linux-arm64");
        let dir = tmp.path().join("handoff");
        let hash = seal(tmp.path(), &dir, "linux-arm64", &env).unwrap();
        fs::rename(
            dir.join("provenance.json"),
            tmp.path().join("elsewhere.json"),
        )
        .unwrap();
        std::os::unix::fs::symlink(
            tmp.path().join("elsewhere.json"),
            dir.join("provenance.json"),
        )
        .unwrap();
        assert!(verify(tmp.path(), &dir, "linux-arm64", &hash, &env).is_err());
    }
}
#[cfg(unix)]
#[test]
fn real_smoke_is_credential_free_and_requires_markers() {
    let tmp = tempfile::tempdir().unwrap();
    let binary = tmp.path().join("nexode");
    fs::write(&binary,"#!/bin/sh\n[ -z \"${NEXODE_API_KEY:-}${GITHUB_TOKEN:-}${APPLE_CERTIFICATE_PASSWORD:-}\" ] || exit 99\nprintf 'nexode Usage: fixture\\n'\n").unwrap();
    smoke(tmp.path()).unwrap();
    for body in ["#!/bin/sh\nexit 1\n", "#!/bin/sh\necho incorrect\n"] {
        fs::write(&binary, body).unwrap();
        assert!(smoke(tmp.path()).is_err());
    }
}
