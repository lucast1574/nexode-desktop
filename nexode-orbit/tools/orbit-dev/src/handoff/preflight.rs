use super::*;
use crate::release::Github;
const ENVIRONMENT: &str = "native-handoff-signing";
pub const SECRETS: &[&str] = &[
    "APPLE_CERTIFICATE_P12_BASE64",
    "APPLE_CERTIFICATE_PASSWORD",
    "APPLE_SIGNING_IDENTITY",
    "APPLE_NOTARY_KEY_P8_BASE64",
    "APPLE_NOTARY_KEY_ID",
    "APPLE_NOTARY_ISSUER",
];
fn github(api: &mut impl Github, path: &str) -> Result<Vec<Value>> {
    let text = api
        .command(&["api", "--method", "GET", "--paginate", "--slurp", path].map(str::to_string))
        .map_err(|_| {
            anyhow::anyhow!("GitHub CLI unavailable or not readable with current authorization")
        })?;
    let value: Value =
        serde_json::from_str(&text).map_err(|_| anyhow::anyhow!("unexpected GitHub response"))?;
    let pages = value
        .as_array()
        .filter(|p| !p.is_empty() && p.iter().all(Value::is_object))
        .context("unexpected GitHub response")?;
    Ok(pages.clone())
}
fn names(api: &mut impl Github, path: &str) -> Result<std::collections::BTreeSet<String>> {
    let mut names = std::collections::BTreeSet::new();
    for page in github(api, path)? {
        for secret in page["secrets"].as_array().context("missing secret names")? {
            names.insert(
                secret["name"]
                    .as_str()
                    .context("missing secret name")?
                    .to_string(),
            );
        }
    }
    Ok(names)
}
fn assess(env: &Value, policies: &[Value], names: &std::collections::BTreeSet<String>) -> Value {
    let rules = env["protection_rules"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let reviewers = rules.iter().any(|r| {
        r["type"] == "required_reviewers"
            && r["reviewers"].as_array().is_some_and(|r| !r.is_empty())
    });
    let master = env["deployment_branch_policy"]["custom_branch_policies"] == true
        && env["deployment_branch_policy"]["protected_branches"] == false
        && policies.len() == 1
        && policies[0]["name"] == "master"
        && policies[0]["type"] == "branch";
    let mut missing: Vec<_> = SECRETS
        .iter()
        .copied()
        .filter(|s| !names.contains(*s))
        .collect();
    missing.sort();
    json!({"environment":ENVIRONMENT,"required_reviewers":reviewers,"prevent_self_review":rules.iter().any(|r|r["type"]=="required_reviewers" && r["prevent_self_review"]==true),"master_only_deployments":master,"missing_environment_secret_names":missing,"ready":reviewers && master && missing.is_empty()})
}
fn inspect(api: &mut impl Github) -> Value {
    let mut report = json!({"repository":REPOSITORY,"read_only":true,"note":"Secret names only. This cannot validate credential values, Apple access, or certificate validity."});
    match names(
        api,
        &format!("repos/{REPOSITORY}/actions/secrets?per_page=100"),
    ) {
        Ok(names) => {
            report["hosted_arm_key_configured"] = json!(names.contains("JUSTRUST_API_KEY"))
        }
        Err(e) => {
            report["hosted_arm_key_configured"] = Value::Null;
            report["hosted_arm_check"] = json!(e.to_string());
        }
    }
    let signing = (|| -> Result<Value> {
        let base = format!("repos/{REPOSITORY}/environments/{ENVIRONMENT}");
        let env = github(api, &base)?.remove(0);
        let mut policies = vec![];
        for page in github(
            api,
            &format!("{base}/deployment-branch-policies?per_page=100"),
        )? {
            policies.extend(
                page["branch_policies"]
                    .as_array()
                    .context("missing branch policies")?
                    .iter()
                    .cloned(),
            );
        }
        let names = names(api, &format!("{base}/secrets?per_page=100"))?;
        Ok(assess(&env, &policies, &names))
    })();
    report["signing"]=signing.unwrap_or_else(|e|json!({"environment":ENVIRONMENT,"ready":false,"check":e.to_string(),"required_environment_secret_names":SECRETS}));
    report
}
pub fn run(args: impl Iterator<Item = String>) -> Result<()> {
    ensure!(
        args.count() == 0,
        "native-handoff-preflight takes no arguments"
    );
    let report = inspect(&mut crate::release::Gh);
    println!("{}", serde_json::to_string_pretty(&report)?);
    if report["signing"]["ready"] != true {
        std::process::exit(1);
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn env() -> Value {
        json!({"protection_rules":[{"type":"required_reviewers","reviewers":[{"type":"User"}],"prevent_self_review":true}],"deployment_branch_policy":{"custom_branch_policies":true,"protected_branches":false}})
    }
    #[test]
    fn reviewers_secrets_and_exact_master_policy() {
        let names = SECRETS.iter().map(|s| s.to_string()).collect();
        let policy = vec![json!({"name":"master","type":"branch"})];
        let report = assess(&env(), &policy, &names);
        assert_eq!(report["ready"], true);
        assert_eq!(report["prevent_self_review"], true);
        assert_eq!(assess(&env(), &policy, &Default::default())["ready"], false);
        let mut no = env();
        no["protection_rules"] = json!([]);
        assert_eq!(assess(&no, &policy, &names)["ready"], false);
        for policies in [
            vec![json!({"name":"*","type":"branch"})],
            vec![json!({"name":"master","type":"tag"})],
            vec![policy[0].clone(), json!({"name":"release","type":"branch"})],
        ] {
            assert_eq!(assess(&env(), &policies, &names)["ready"], false);
        }
    }
    #[test]
    fn get_only_and_errors_never_echo_values() {
        struct Fake {
            response: &'static str,
            fail: bool,
        }
        impl Github for Fake {
            fn command(&mut self, args: &[String]) -> Result<String> {
                assert_eq!(
                    &args[..5],
                    ["api", "--method", "GET", "--paginate", "--slurp"]
                );
                ensure!(!self.fail, "private diagnostic");
                Ok(self.response.into())
            }
        }
        let mut api = Fake {
            response: r#"[{"secrets":[{"name":"TEST"}]}]"#,
            fail: false,
        };
        assert!(names(&mut api, "o/r").unwrap().contains("TEST"));
        for (response, fail) in [("private invalid JSON", false), ("", true)] {
            let mut api = Fake { response, fail };
            assert!(
                !github(&mut api, "o/r")
                    .unwrap_err()
                    .to_string()
                    .contains("private")
            );
            assert_eq!(inspect(&mut api)["signing"]["ready"], false);
        }
    }
}
