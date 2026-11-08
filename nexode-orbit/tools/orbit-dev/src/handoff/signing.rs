//! Native guards shared by the macOS signing shell orchestrators.
use super::*;
pub fn team(identity: &str) -> Result<String> {
    let re = regex::Regex::new(r"\ADeveloper ID Application: [^\r\n]+ \(([A-Z0-9]{10})\)\z")?;
    Ok(re
        .captures(identity)
        .context("expected exact Developer ID Application name with 10-character Team ID")?[1]
        .into())
}
fn fingerprint(path: &Path, identity: &str) -> Result<String> {
    let re = regex::Regex::new(r#"\A\s*\d+\)\s+([A-Fa-f0-9]{40})\s+"(.*)"\s*\z"#)?;
    let text = fs::read_to_string(path)?;
    let matches: Vec<_> = text
        .lines()
        .filter_map(|l| re.captures(l))
        .filter(|c| &c[2] == identity)
        .map(|c| c[1].to_string())
        .collect();
    ensure!(
        matches.len() == 1,
        "Expected exactly one matching Developer ID Application identity"
    );
    Ok(matches[0].clone())
}
fn keychains(path: &Path, extra: Option<&str>) -> Result<Vec<String>> {
    let mut args = vec![
        "list-keychains".into(),
        "-d".into(),
        "user".into(),
        "-s".into(),
    ];
    if let Some(extra) = extra {
        args.push(extra.into());
    }
    args.extend(shlex::split(&fs::read_to_string(path)?).context("invalid quoted keychain list")?);
    Ok(args)
}
fn plan(path: &Path) -> Result<()> {
    let p: Value = serde_json::from_slice(&fs::read(path)?)?;
    let commands = p["commands"]
        .as_array()
        .context("missing signing commands")?;
    ensure!(
        p["mode"] == "plan-only"
            && commands.len() == 2
            && commands.iter().all(|c| c[0] == "/usr/bin/codesign"),
        "expected real absolute plan-only signing commands"
    );
    let verify = commands[1]
        .as_array()
        .context("invalid verification command")?;
    ensure!(
        verify.len() >= 2
            && verify[verify.len() - 2]
                .as_str()
                .is_some_and(|s| s.starts_with("=anchor apple generic")),
        "verification must use inline Apple requirement"
    );
    Ok(())
}
fn accepted(path: &Path) -> Result<Value> {
    let value: Value = serde_json::from_slice(&fs::read(path)?)?;
    ensure!(
        value["status"] == "Accepted",
        "Notarization was not accepted"
    );
    Ok(value)
}
fn record(
    directory: &Path,
    pin: &str,
    signed: &str,
    team: &str,
    execution: &Path,
    env: &BTreeMap<String, String>,
) -> Result<Value> {
    let execution: Value = serde_json::from_slice(&fs::read(execution)?)?;
    ensure!(
        execution["mode"] == "signed-and-native-verified",
        "expected completed native signing"
    );
    let mut receipt = json!({"schema":1,"helper_repository":"lucast1574/nexode-orbit","helper_revision":pin,"signed_sha256":signed,"team_id":team,"target":"aarch64-apple-darwin","identifier":"com.nexode.cli","checks":["wrong-hash-no-output","plan-no-output","real-helper-execute","apple-chain-identifier-team","signed-native-execution","unsigned-unchanged","existing-destination-preserved"],"notarized":false});
    for (key, var) in [
        ("repository", "GITHUB_REPOSITORY"),
        ("source_sha", "GITHUB_SHA"),
        ("ref", "GITHUB_REF"),
        ("run_id", "GITHUB_RUN_ID"),
        ("run_attempt", "GITHUB_RUN_ATTEMPT"),
        ("workflow", "GITHUB_WORKFLOW"),
        ("unsigned_sha256", "TRUSTED_SHA256"),
    ] {
        receipt[key] = json!(env.get(var).with_context(|| format!("{var} required"))?);
    }
    fs::write(
        directory.join("receipt.json"),
        serde_json::to_string_pretty(&receipt)? + "\n",
    )?;
    Ok(receipt)
}
pub fn run(args: impl Iterator<Item = String>) -> Result<()> {
    let args: Vec<_> = args.collect();
    let action = args.first().context("signing-check action required")?;
    let rest = &args[1..];
    match (action.as_str(), rest) {
        ("team", []) => println!("{}", team(&std::env::var("APPLE_SIGNING_IDENTITY")?)?),
        ("fingerprint", [path]) => println!(
            "{}",
            fingerprint(Path::new(path), &std::env::var("APPLE_SIGNING_IDENTITY")?)?
        ),
        ("keychains", [path]) | ("keychains", [path, _]) => {
            let args = keychains(Path::new(path), rest.get(1).map(String::as_str))?;
            let output = crate::process::capture(
                Command::new("security").args(args),
                Duration::from_secs(30),
            )?;
            ensure!(
                output.status.success(),
                "keychain search-list update failed"
            );
        }
        ("plan", [path]) => plan(Path::new(path))?,
        ("smoke", [path]) => super::smoke(Path::new(path))?,
        ("notarization", [path]) => {
            accepted(Path::new(path))?;
        }
        ("provenance", [original, signed]) => {
            let original: Value =
                serde_json::from_slice(&fs::read(Path::new(original).join("provenance.json"))?)?;
            let directory = Path::new(signed);
            let record = json!({"unsigned_provenance":original,"signed_sha256":digest(&directory.join("nexode"))?,"notarization":accepted(&directory.join("notarization.json"))?});
            fs::write(
                directory.join("signing-provenance.json"),
                serde_json::to_string_pretty(&record)? + "\n",
            )?;
        }
        ("receipt", [directory, pin, signed, team, execution]) => {
            record(
                Path::new(directory),
                pin,
                signed,
                team,
                Path::new(execution),
                &std::env::vars().collect(),
            )?;
        }
        _ => anyhow::bail!("invalid signing-check action or arguments"),
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_identity_and_fingerprint() {
        let identity = "Developer ID Application: Example (ABCDEFGHIJ)";
        assert_eq!(team(identity).unwrap(), "ABCDEFGHIJ");
        for bad in [
            "-",
            "Example",
            "Developer ID Application: Example (SHORT)",
            &format!("{identity}\n"),
        ] {
            assert!(team(bad).is_err());
        }
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("identities");
        let line = format!("  1) {} \"{identity}\"\n", "A".repeat(40));
        for (text, ok) in [
            (line.clone(), true),
            (line.repeat(2), false),
            (line.replace("Example", "Example Extra"), false),
            ("0 valid identities found".into(), false),
        ] {
            fs::write(&path, text).unwrap();
            assert_eq!(fingerprint(&path, identity).is_ok(), ok);
        }
    }
    #[test]
    fn quoted_keychains_and_guarded_plan() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("keychains");
        fs::write(&path,"  \"/Users/runner/login.keychain-db\"\n \"/Library/Keychains/With Spaces.keychain-db\"\n").unwrap();
        assert_eq!(
            keychains(&path, Some("/private/new")).unwrap(),
            [
                "list-keychains",
                "-d",
                "user",
                "-s",
                "/private/new",
                "/Users/runner/login.keychain-db",
                "/Library/Keychains/With Spaces.keychain-db"
            ]
        );
        let p = json!({"mode":"plan-only","commands":[["/usr/bin/codesign","--sign","A".repeat(40)],["/usr/bin/codesign","-R","=anchor apple generic","artifact"]]});
        fs::write(&path, p.to_string()).unwrap();
        plan(&path).unwrap();
        for mode in 0..3 {
            let mut p = p.clone();
            match mode {
                0 => p["commands"][0][0] = json!("codesign"),
                1 => p["commands"][1][2] = json!("anchor apple generic"),
                _ => p["mode"] = json!("signed-and-native-verified"),
            };
            fs::write(&path, p.to_string()).unwrap();
            assert!(plan(&path).is_err());
        }
    }
    #[test]
    fn receipt_records_only_public_allowlist() {
        let tmp = tempfile::tempdir().unwrap();
        let execution = tmp.path().join("execution.json");
        fs::write(&execution, r#"{"mode":"signed-and-native-verified"}"#).unwrap();
        let env = BTreeMap::from([
            ("GITHUB_REPOSITORY".into(), REPOSITORY.into()),
            ("GITHUB_SHA".into(), "a".repeat(40)),
            ("GITHUB_REF".into(), "refs/heads/master".into()),
            ("GITHUB_RUN_ID".into(), "123".into()),
            ("GITHUB_RUN_ATTEMPT".into(), "2".into()),
            ("GITHUB_WORKFLOW".into(), "validation".into()),
            ("TRUSTED_SHA256".into(), "b".repeat(64)),
            ("APPLE_CERTIFICATE_PASSWORD".into(), "DO-NOT-RECORD".into()),
        ]);
        let r = record(
            tmp.path(),
            &"c".repeat(40),
            &"d".repeat(64),
            "ABCDEFGHIJ",
            &execution,
            &env,
        )
        .unwrap();
        assert!(!r.to_string().contains("DO-NOT-RECORD"));
        assert_eq!(r["run_attempt"], "2");
        assert_eq!(r["notarized"], false);
        fs::write(execution, r#"{"mode":"plan-only"}"#).unwrap();
        assert!(
            record(
                tmp.path(),
                "c",
                "d",
                "TEAM",
                &tmp.path().join("execution.json"),
                &env
            )
            .is_err()
        );
    }
}
