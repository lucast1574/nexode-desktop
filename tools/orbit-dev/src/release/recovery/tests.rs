use super::*;
fn git_ok(root: &Path, args: &[&str]) -> String {
    text(root, args).unwrap()
}
fn setup() -> (tempfile::TempDir, String, String, String, String) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join("crates/nexode-companion/src")).unwrap();
    fs::create_dir_all(root.join("scripts")).unwrap();
    git_ok(root, &["init", "-b", "main"]);
    for (k, v) in [
        ("user.name", "Recovery Fixture"),
        ("user.email", "fixture@example.invalid"),
        ("commit.gpgsign", "false"),
        ("tag.gpgsign", "false"),
    ] {
        git_ok(root, &["config", k, v]);
    }
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname='fixture'\nversion='0.96.0'\n",
    )
    .unwrap();
    let original = format!("fn example() {{\n{ORIGINAL}}}\n");
    fs::write(root.join(SOURCE), &original).unwrap();
    for helper in HELPERS {
        fs::write(root.join(helper), "#!/bin/sh\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(root.join(helper), fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
    git_ok(root, &["add", "."]);
    git_ok(root, &["commit", "-m", "Original"]);
    let tag = git_ok(root, &["rev-parse", "HEAD"]);
    git_ok(root, &["tag", "v0.96.0"]);
    let repaired = original.replace(ORIGINAL, REPLACEMENT);
    fs::write(root.join(SOURCE), &repaired).unwrap();
    git_ok(root, &["add", "."]);
    git_ok(root, &["commit", "-m", "Reviewed repair"]);
    let fix = git_ok(root, &["rev-parse", "HEAD"]);
    git_ok(root, &["checkout", "--detach", "v0.96.0"]);
    (
        temp,
        tag,
        fix,
        hash(original.as_bytes()),
        hash(repaired.as_bytes()),
    )
}
#[test]
fn exact_reviewed_bytes_keep_tag_identity_and_reject_second_application() {
    for crlf in [false, true] {
        let (temp, tag, fix, original, repaired) = setup();
        let root = temp.path();
        let output = tempfile::NamedTempFile::new().unwrap();
        if crlf {
            git_ok(root, &["config", "core.autocrlf", "true"]);
            fs::remove_file(root.join(SOURCE)).unwrap();
            git_ok(root, &["checkout", "--", SOURCE]);
            assert!(
                fs::read_to_string(root.join(SOURCE))
                    .unwrap()
                    .contains("\r\n")
            );
        }
        let policy = Policy {
            original: &original,
            repaired: &repaired,
            commit: &fix,
        };
        let p = apply(
            root,
            "v0.96.0",
            "windows-listener",
            &fix,
            output.path(),
            &policy,
        )
        .unwrap();
        assert_eq!(git_ok(root, &["rev-parse", "HEAD"]), tag);
        assert_eq!(
            git_ok(root, &["rev-parse", "refs/tags/v0.96.0^{commit}"]),
            tag
        );
        assert_eq!(p["fix_commit"], fix);
        assert_eq!(hash(&fs::read(root.join(SOURCE)).unwrap()), repaired);
        for helper in HELPERS {
            assert_eq!(
                p["release_helpers"][*helper],
                hash(&fs::read(root.join(helper)).unwrap())
            );
        }
        assert!(
            apply(
                root,
                "v0.96.0",
                "windows-listener",
                &fix,
                output.path(),
                &policy
            )
            .is_err()
        );
    }
}
#[test]
fn unpatched_and_rejected_repairs_preserve_source() {
    let (temp, tag, fix, original, repaired) = setup();
    let root = temp.path();
    let output = tempfile::NamedTempFile::new().unwrap();
    let policy = Policy {
        original: &original,
        repaired: &repaired,
        commit: &fix,
    };
    let p = apply(root, "v0.96.0", "none", &tag, output.path(), &policy).unwrap();
    assert_eq!(p["eligibility_policy"], "exact-sha-main-ci");
    assert!(git_ok(root, &["status", "--porcelain"]).is_empty());
    for (tag, repair, sha) in [
        ("master", "none", fix.as_str()),
        ("v0.96.0;touch nope", "none", &fix),
        ("v0.96.0", "arbitrary", &fix),
        ("v0.97.0", "windows-listener", &fix),
        ("v0.96.0", "windows-listener", "master"),
        ("v0.96.0", "windows-listener", &tag),
    ] {
        assert!(apply(root, tag, repair, sha, output.path(), &policy).is_err());
        assert!(git_ok(root, &["diff", "--name-only"]).is_empty());
    }
    assert!(
        apply(
            root,
            "v0.96.0",
            "windows-listener",
            &fix,
            output.path(),
            &REVIEWED
        )
        .is_err()
    );
    assert!(git_ok(root, &["diff", "--name-only"]).is_empty());
    let dirty = fs::read_to_string(root.join(SOURCE)).unwrap() + "// unrelated change\n";
    fs::write(root.join(SOURCE), &dirty).unwrap();
    assert!(
        apply(
            root,
            "v0.96.0",
            "windows-listener",
            &fix,
            output.path(),
            &policy
        )
        .is_err()
    );
    assert_eq!(fs::read_to_string(root.join(SOURCE)).unwrap(), dirty);
}
#[test]
fn public_state_refuses_existing_public_newer_or_api_failure() {
    struct Fake {
        draft: String,
        releases: Value,
        fail: bool,
    }
    impl Github for Fake {
        fn command(&mut self, args: &[String]) -> Result<String> {
            ensure!(!self.fail, "API failure");
            Ok(if args[1] == "view" {
                self.draft.clone()
            } else {
                self.releases.to_string()
            })
        }
    }
    for (draft, releases, fail, ok) in [
        ("true", json!([]), false, true),
        ("false", json!([]), false, false),
        ("true", json!([{"tagName":"v0.97.0"}]), false, false),
        (
            "true",
            json!([{"tagName":"v0.95.0"},{"tagName":"v0.96.0"}]),
            false,
            true,
        ),
        ("true", json!([]), true, false),
    ] {
        let mut api = Fake {
            draft: draft.into(),
            releases,
            fail,
        };
        assert_eq!(public_state(&mut api, "v0.96.0").is_ok(), ok);
    }
}
