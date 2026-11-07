use super::*;
use serde_json::json;
const TAG: &str = "v0.98.1";
fn data() -> Value {
    json!({"tag_name":TAG,"draft":false,"prerelease":false,"assets":required().iter().map(|name|json!({"name":name,"size":1,"state":"uploaded"})).collect::<Vec<_>>()})
}
fn sums(dir: &Path, names: &BTreeSet<String>) {
    let mut sums = String::new();
    for name in names.iter().filter(|n| n.as_str() != "SHA256SUMS") {
        sums.push_str(&format!("{}  {name}\n", hash(&dir.join(name)).unwrap()));
    }
    fs::write(dir.join("SHA256SUMS"), sums).unwrap();
}
fn fixture() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    for name in required() {
        fs::write(tmp.path().join(name), "fixture").unwrap();
    }
    fs::write(tmp.path().join("release-provenance.json"),json!({"tag":TAG,"tag_commit":"a".repeat(40),"repair":"none","eligibility_policy":"exact-sha-main-ci"}).to_string()).unwrap();
    sums(tmp.path(), &required());
    tmp
}
#[test]
fn complete_metadata_and_explicit_draft_only() {
    assert_eq!(metadata(&data(), TAG, false).unwrap(), required());
    let mut draft = data();
    draft["draft"] = json!(true);
    assert!(metadata(&draft, TAG, false).is_err());
    assert_eq!(metadata(&draft, TAG, true).unwrap(), required());
    for (key, value) in [
        ("draft", Value::Null),
        ("draft", json!(0)),
        ("draft", json!("true")),
        ("prerelease", json!(true)),
        ("tag_name", json!("v0.98.0")),
    ] {
        let mut v = data();
        v[key] = value;
        assert!(metadata(&v, TAG, true).is_err());
    }
    for (key, value) in [
        ("name", json!("../unsafe")),
        ("name", json!("..\\unsafe")),
        ("size", json!(0)),
        ("size", json!(true)),
        ("state", json!("new")),
    ] {
        let mut v = data();
        v["assets"][0][key] = value;
        assert!(metadata(&v, TAG, false).is_err());
    }
    let mut v = data();
    v["assets"].as_array_mut().unwrap().pop();
    assert!(metadata(&v, TAG, false).is_err());
    let mut v = data();
    let first = v["assets"][0].clone();
    v["assets"].as_array_mut().unwrap().push(first);
    assert!(metadata(&v, TAG, false).is_err());
}
#[test]
fn downloads_coverage_corruption_provenance_and_symlinks() {
    let tmp = fixture();
    downloads(tmp.path(), &required(), TAG, &"a".repeat(40)).unwrap();
    let path = tmp.path().join("SHA256SUMS");
    let original = fs::read_to_string(&path).unwrap();
    for invalid in [
        String::new(),
        original.lines().next().unwrap().into(),
        format!("{original}{}\n", original.lines().next().unwrap()),
        format!("{original}{}  ../unsafe\n", "a".repeat(64)),
        format!("{original}invalid\n"),
    ] {
        fs::write(&path, invalid).unwrap();
        assert!(downloads(tmp.path(), &required(), TAG, &"a".repeat(40)).is_err());
    }
    fs::write(path, original).unwrap();
    for (key, value) in [
        ("tag", json!("v0.98.0")),
        ("tag_commit", json!("b".repeat(40))),
        ("repair", json!("windows-listener")),
        ("eligibility_policy", json!("bypassed")),
    ] {
        let tmp = fixture();
        let path = tmp.path().join("release-provenance.json");
        let mut p: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        p[key] = value;
        fs::write(path, p.to_string()).unwrap();
        sums(tmp.path(), &required());
        assert!(
            downloads(tmp.path(), &required(), TAG, &"a".repeat(40))
                .unwrap_err()
                .to_string()
                .contains("provenance")
        );
    }
    let path = tmp.path().join("nexode-linux-x86_64.tar.gz");
    fs::write(&path, "corrupted").unwrap();
    assert!(
        downloads(tmp.path(), &required(), TAG, &"a".repeat(40))
            .unwrap_err()
            .to_string()
            .contains("Checksum mismatch")
    );
    fs::remove_file(&path).unwrap();
    assert!(downloads(tmp.path(), &required(), TAG, &"a".repeat(40)).is_err());
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(tmp.path().join("nexode-linux-aarch64.tar.gz"), path).unwrap();
        assert!(downloads(tmp.path(), &required(), TAG, &"a".repeat(40)).is_err());
    }
}
#[test]
fn draft_lookup_pagination_missing_duplicate_and_public_endpoint() {
    struct Fake {
        pages: std::collections::VecDeque<Value>,
        calls: Vec<String>,
    }
    impl Github for Fake {
        fn command(&mut self, _: &[String]) -> Result<String> {
            unreachable!()
        }
        fn api(&mut self, path: &str, _: bool) -> Result<Value> {
            self.calls.push(path.into());
            Ok(self.pages.pop_front().unwrap())
        }
    }
    let first: Vec<_> = (0..100)
        .map(|n| json!({"tag_name":format!("v0.0.{n}")}))
        .collect();
    let mut draft = data();
    draft["draft"] = json!(true);
    let mut api = Fake {
        pages: [json!(first), json!([draft])].into(),
        calls: vec![],
    };
    assert_eq!(fetch(&mut api, "o/r", TAG, true).unwrap(), draft);
    assert_eq!(api.calls.len(), 2);
    assert!(api.calls[1].ends_with("page=2"));
    for pages in [json!([]), json!([draft, draft])] {
        let mut api = Fake {
            pages: [pages].into(),
            calls: vec![],
        };
        assert!(fetch(&mut api, "o/r", TAG, true).is_err());
    }
    let mut api = Fake {
        pages: [data()].into(),
        calls: vec![],
    };
    fetch(&mut api, "o/r", TAG, false).unwrap();
    assert!(api.calls[0].contains("releases/tags/"));
}
#[test]
fn availability_idempotent_truthful_signing_and_checksums() {
    for signed in ["true", "false", ""] {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("release_notes.md");
        fs::write(&path, "Release highlights\n").unwrap();
        for _ in 0..2 {
            availability(&path, |key| {
                Some(
                    if key == "WINDOWS_SIGNING_ENABLED" {
                        signed
                    } else {
                        "true"
                    }
                    .into(),
                )
            })
            .unwrap();
        }
        let body = fs::read_to_string(path).unwrap();
        assert_eq!(body.matches("## Platform availability").count(), 1);
        assert!(body.contains("Release highlights"));
        assert_eq!(
            body.contains("Authenticode signatures verified"),
            signed == "true"
        );
        assert_eq!(body.contains("**unsigned**"), signed != "true");
    }
    let tmp = tempfile::tempdir().unwrap();
    let output = tmp.path().join("SHA256SUMS");
    assert!(checksums(tmp.path(), &output).is_err());
    fs::create_dir(tmp.path().join("platform")).unwrap();
    fs::write(tmp.path().join("platform/test.exe"), b"bytes").unwrap();
    checksums(tmp.path(), &output).unwrap();
    assert_eq!(
        fs::read_to_string(&output).unwrap(),
        format!(
            "{}  test.exe\n",
            hash(&tmp.path().join("platform/test.exe")).unwrap()
        )
    );
    fs::write(tmp.path().join("test.exe"), b"different").unwrap();
    assert!(checksums(tmp.path(), &output).is_err());
}
