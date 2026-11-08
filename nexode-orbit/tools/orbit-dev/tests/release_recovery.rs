#![cfg(unix)]
mod support;
use std::collections::{BTreeMap, BTreeSet};
use support::*;
fn jobs() -> BTreeMap<String, String> {
    let wf = read(".github/workflows/release.yml");
    let mut result = BTreeMap::new();
    let mut name = String::new();
    for line in wf.split_once("\njobs:\n").unwrap().1.lines() {
        if line.starts_with("  ") && !line.starts_with("   ") && line.ends_with(':') {
            name = line.trim().trim_end_matches(':').into();
            result.insert(name.clone(), String::new());
        } else if let Some(body) = result.get_mut(&name) {
            body.push_str(line);
            body.push('\n');
        }
    }
    result
}
fn field<'a>(body: &'a str, key: &str) -> Option<&'a str> {
    body.lines()
        .find_map(|s| s.strip_prefix(&format!("    {key}: ")))
}
fn needs(jobs: &BTreeMap<String, String>) -> BTreeMap<String, Vec<String>> {
    jobs.iter()
        .map(|(n, b)| {
            (
                n.clone(),
                field(b, "needs")
                    .map(|s| {
                        s.trim_matches(['[', ']'])
                            .split(", ")
                            .map(str::to_string)
                            .collect()
                    })
                    .unwrap_or_default(),
            )
        })
        .collect()
}
fn ancestors(job: &str, needs: &BTreeMap<String, Vec<String>>) -> BTreeSet<String> {
    let mut result = BTreeSet::new();
    for parent in &needs[job] {
        result.insert(parent.clone());
        result.extend(ancestors(parent, needs));
    }
    result
}
// Restricted boolean grammar for the checked-in job predicates, no expression execution.
fn expression(condition: &str, results: &BTreeMap<String, &str>) -> bool {
    let text = condition.trim_start_matches("${{ ").trim_end_matches(" }}");
    static PATTERN: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = PATTERN.get_or_init(|| {
        regex::Regex::new(r"always\(\)|needs\.[\w-]+\.result|'[^']*'|==|&&|\|\||[()]").unwrap()
    });
    let tokens: Vec<_> = re.find_iter(text).map(|m| m.as_str()).collect();
    assert_eq!(re.replace_all(text, "").trim(), "");
    fn atom(tokens: &[&str], pos: &mut usize, results: &BTreeMap<String, &str>) -> bool {
        let token = tokens[*pos];
        *pos += 1;
        match token {
            "always()" => true,
            "(" => {
                let value = or(tokens, pos, results);
                assert_eq!(tokens[*pos], ")");
                *pos += 1;
                value
            }
            _ => {
                let name = token
                    .strip_prefix("needs.")
                    .unwrap()
                    .strip_suffix(".result")
                    .unwrap();
                assert_eq!(tokens[*pos], "==");
                *pos += 1;
                let value = tokens[*pos].trim_matches('\'');
                *pos += 1;
                results[name] == value
            }
        }
    }
    fn and(tokens: &[&str], pos: &mut usize, results: &BTreeMap<String, &str>) -> bool {
        let mut value = atom(tokens, pos, results);
        while tokens.get(*pos) == Some(&"&&") {
            *pos += 1;
            value &= atom(tokens, pos, results);
        }
        value
    }
    fn or(tokens: &[&str], pos: &mut usize, results: &BTreeMap<String, &str>) -> bool {
        let mut value = and(tokens, pos, results);
        while tokens.get(*pos) == Some(&"||") {
            *pos += 1;
            value |= and(tokens, pos, results);
        }
        value
    }
    let mut pos = 0;
    let value = or(&tokens, &mut pos, results);
    assert_eq!(pos, tokens.len());
    value
}
#[test]
fn complete_dependency_graph_all_terminal_status_combinations() {
    let jobs = jobs();
    let needs = needs(&jobs);
    let expected = [
        ("create-release", vec![]),
        ("recovery-preflight", vec!["create-release"]),
        (
            "build-linux-macos",
            vec!["create-release", "recovery-preflight"],
        ),
        (
            "build-windows",
            vec!["create-release", "recovery-preflight"],
        ),
        ("publish-windows", vec!["create-release", "build-windows"]),
        (
            "build-freebsd",
            vec!["create-release", "recovery-preflight"],
        ),
        (
            "release",
            vec![
                "create-release",
                "build-linux-macos",
                "publish-windows",
                "build-freebsd",
            ],
        ),
    ];
    assert_eq!(
        needs,
        expected
            .into_iter()
            .map(|(k, v)| (k.into(), v.into_iter().map(str::to_string).collect()))
            .collect()
    );
    assert!(
        field(&jobs["create-release"], "if")
            .unwrap()
            .contains("vars.NEXODE_RELEASE_REPO != ''")
    );
    assert_eq!(
        field(&jobs["recovery-preflight"], "if").unwrap(),
        "github.event_name == 'workflow_dispatch' && inputs.recovery == 'windows-listener'"
    );
    for job in [
        "build-linux-macos",
        "build-windows",
        "build-freebsd",
        "publish-windows",
        "release",
    ] {
        let parents: Vec<_> = ancestors(job, &needs).into_iter().collect();
        let condition = field(&jobs[job], "if").unwrap();
        for mut states in 0..4usize.pow(parents.len() as u32) {
            let results: BTreeMap<_, _> = parents
                .iter()
                .map(|p| {
                    let state = ["success", "skipped", "failure", "cancelled"][states % 4];
                    states /= 4;
                    (p.clone(), state)
                })
                .collect();
            let expected = results["create-release"] == "success"
                && if job.starts_with("build-") {
                    ["success", "skipped"].contains(&results["recovery-preflight"])
                } else if job == "publish-windows" {
                    results["build-windows"] == "success"
                } else {
                    true
                };
            let implicit =
                condition.contains("always()") || results.values().all(|s| *s == "success");
            assert_eq!(
                implicit && expression(condition, &results),
                expected,
                "{job}: {results:?}"
            );
        }
    }
    let results = BTreeMap::from([
        ("create-release".into(), "success"),
        ("build-windows".into(), "success"),
        ("recovery-preflight".into(), "skipped"),
    ]);
    let condition = field(&jobs["publish-windows"], "if").unwrap();
    assert!(expression(condition, &results));
    let without = condition.replace("always() && ", "");
    assert!(!(without.contains("always()") || results.values().all(|s| *s == "success")));
}
#[test]
fn source_snapshot_build_identity_and_final_publication_order() {
    let wf = read(".github/workflows/release.yml");
    assert!(!wf.contains("python"));
    assert!(!wf.contains("GITHUB_REF_NAME"));
    assert_eq!(
        wf.matches("ref: ${{ needs.create-release.outputs.tag_sha }}")
            .count(),
        6
    );
    assert_eq!(
        wf.matches("name: Apply reviewed release source recovery")
            .count(),
        6
    );
    assert_eq!(
        wf.matches("name: Build trusted release maintenance tool")
            .count(),
        7
    );
    assert_eq!(
        wf.matches("git archive \"$WORKFLOW_SHA\" tools/orbit-dev")
            .count(),
        7
    );
    for text in [
        "Require successful exact-commit CI for unpatched source",
        "Check and test exact repaired companion source",
        "Release tag no longer matches the triggering commit",
        "cancel-in-progress: false",
        "sha256sum release-provenance.json >> SHA256SUMS",
        "SHA256SUMS release-provenance.json --clobber",
        "Get-AuthenticodeSignature",
        "Validate complete platform asset set",
        "Keeping the release as a draft until every platform succeeds",
        "if: github.event_name != 'workflow_dispatch'",
    ] {
        assert!(wf.contains(text), "{text}");
    }
    assert_eq!(
        wf.matches("needs: [create-release, recovery-preflight]")
            .count(),
        3
    );
    assert_eq!(
        wf.matches("Verify release build preserved Cargo.lock")
            .count(),
        2
    );
    assert_eq!(
        wf.matches("--check-public-state \"$RELEASE_TAG\"").count(),
        2
    );
    assert_eq!(wf.matches("JUSTRUST_DISABLE: \"1\"").count(), 2);
    let windows = between(&wf, "  build-windows:\n", "  publish-windows:\n");
    assert!(windows.contains("& justrust @cargoArgs"));
    assert!(windows.contains("@(\"build\", \"--local\", \"--locked\", \"--release\""));
    let staged = wf
        .find("      - name: Verify staged assets before publication")
        .unwrap();
    let publish = wf.find("      - name: Publish completed release").unwrap();
    let verify = wf
        .find("      - name: Verify public assets and exact source provenance")
        .unwrap();
    let announce = wf
        .find("      - name: Queue Discord release announcement")
        .unwrap();
    let packages = wf.find("      - name: Update Homebrew formula").unwrap();
    assert!(staged < publish && publish < verify && verify < announce && verify < packages);
    assert!(wf[staged..publish].contains("--allow-draft"));
    assert!(wf[staged..publish].contains("--sha \"$TAG_SHA\""));
    assert!(!wf[staged..publish].contains("continue-on-error"));
    let step = &wf[verify..announce];
    for text in [
        "\"$ORBIT_DEV_BIN\" verify-public-release",
        "--sha \"$TAG_SHA\"",
        "Smoke test public Linux installer in an isolated home",
        "HOME=\"$install_home\"",
        "NEXODE_VERSION=\"$RELEASE_TAG\"",
        "NEXODE_SKIP_SERVER_RELOAD=1 NEXODE_NO_TELEMETRY=1",
        "\"$install_home/.local/bin/nexode\" --version",
    ] {
        assert!(step.contains(text), "{text}");
    }
    assert!(!step.contains("--allow-draft"));
    assert!(!step.contains("continue-on-error"));
    let companion = read(".github/workflows/windows-companion.yml");
    assert!(companion.contains("JUSTRUST_DISABLE: \"1\""));
    assert!(companion.contains("justrust check --local --locked -p nexode-companion"));
}
#[test]
fn validation_workflow_covers_native_tool_changes_and_installer() {
    let wf = read(".github/workflows/release-recovery.yml");
    for section in [
        between(&wf, "  pull_request:\n", "  push:\n"),
        between(&wf, "  push:\n", "  workflow_dispatch:\n"),
    ] {
        for path in [
            ".github/workflows/release.yml",
            ".github/workflows/release-recovery.yml",
            ".github/workflows/windows-companion.yml",
            ".github/scripts/verify_windows_install.ps1",
            "scripts/*release*",
            "tools/orbit-dev/**",
            "scripts/install.sh",
            "scripts/install_justrust_ci.sh",
            "scripts/build_linux_compat.sh",
        ] {
            assert!(section.contains(&format!("- '{path}'")), "{path}");
        }
    }
    assert!(wf.contains("fetch-depth: 0"));
    assert!(wf.contains("cargo test --locked --manifest-path tools/orbit-dev/Cargo.toml"));
    assert!(wf.contains("bash scripts/test_install_release_metadata.sh"));
    assert!(!wf.contains("python"));
}
