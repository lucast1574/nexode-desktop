use super::*;
use std::collections::BTreeMap;
const BILL: &str =
    "x-anthropic-billing-header: cc_version=2.1.280; cc_entrypoint=sdk-cli; cch=33f85;";
const BETA: &str = "claude-code-20250219,oauth-2025-04-20,interleaved-thinking-2025-05-14";
struct Case {
    name: String,
    headers: BTreeMap<String, String>,
    body: Value,
}
fn session() -> Result<String> {
    let mut b = random::<16>()?;
    b[6] = (b[6] & 15) | 64;
    b[8] = (b[8] & 63) | 128;
    let hex: String = b.iter().map(|b| format!("{b:02x}")).collect();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    ))
}
fn cases(mode: &str, model: &str) -> Result<Vec<Case>> {
    let headers = |full: bool| {
        let mut h = BTreeMap::from([(
            "User-Agent".into(),
            if full {
                "claude-cli/2.1.280 (external, sdk-cli)"
            } else {
                "nexode/0.99"
            }
            .into(),
        )]);
        if full {
            for (k, v) in [
                ("x-app", "cli"),
                ("anthropic-dangerous-direct-browser-access", "true"),
                ("X-Stainless-Lang", "js"),
                ("X-Stainless-Runtime", "node"),
                ("X-Stainless-Package-Version", "0.81.0"),
            ] {
                h.insert(k.into(), v.into());
            }
        }
        h
    };
    let anthropic = |name: &str,
                     mut headers: BTreeMap<String, String>,
                     beta: &str,
                     blocks: Vec<String>,
                     tokens: u32,
                     prompt: &str,
                     tools: bool|
     -> Result<Case> {
        headers.insert("anthropic-version".into(), "2023-06-01".into());
        headers.insert("anthropic-beta".into(), beta.into());
        headers.insert("X-Claude-Code-Session-Id".into(), session()?);
        let mut body = json!({"model":model,"max_tokens":tokens,"system":blocks.iter().map(|t|json!({"type":"text","text":t})).collect::<Vec<_>>(),"messages":[{"role":"user","content":prompt}]});
        if tools {
            body["tools"]=json!(["bash","read","edit","agentgrep","swarm","todo"].map(|n|json!({"name":n,"description":"x","input_schema":{"type":"object","properties":{}}})));
        }
        Ok(Case {
            name: name.into(),
            headers,
            body,
        })
    };
    match mode {
        "headers" => {
            let j = "You are Nexode, a coding agent.";
            let mut result = vec![];
            for (name, full, beta, bill, tools) in [
                ("1 baseline (bill + full headers)", true, BETA, true, false),
                (
                    "2 bill, plain UA, no x-app/stainless",
                    false,
                    BETA,
                    true,
                    false,
                ),
                (
                    "3 bill, full headers, beta oauth only",
                    true,
                    "oauth-2025-04-20",
                    true,
                    false,
                ),
                (
                    "4 bill, plain UA, beta oauth only",
                    false,
                    "oauth-2025-04-20",
                    true,
                    false,
                ),
                (
                    "5 no bill, plain UA, beta oauth only",
                    false,
                    "oauth-2025-04-20",
                    false,
                    false,
                ),
                (
                    "6 bill + full headers + nexode tool names",
                    true,
                    BETA,
                    true,
                    true,
                ),
                (
                    "7 bill, plain UA, oauth beta, nexode tools",
                    false,
                    "oauth-2025-04-20",
                    true,
                    true,
                ),
            ] {
                let blocks = if bill {
                    vec![BILL.into(), j.into()]
                } else {
                    vec![j.into()]
                };
                result.push(anthropic(
                    name,
                    headers(full),
                    beta,
                    blocks,
                    30,
                    "Say hi.",
                    tools,
                )?);
            }
            Ok(result)
        }
        "identity" => {
            let sdk = "You are a Claude agent, built on Anthropic's Claude Agent SDK.";
            let cc = "You are Claude Code, Anthropic's official CLI for Claude.";
            let jc = "You are nexode, powered by Claude Code. You are a third-party CLI, not the official Claude Code CLI.";
            let p="You are Nexode, an open-source coding agent. Use tools to edit files and run builds. ".repeat(20);
            let sid = session()?;
            let mut result = vec![];
            for (name, blocks) in [
                ("A current (bill+SDK+nexode)", vec![BILL, sdk, &p]),
                ("B no identity (bill+nexode)", vec![BILL, &p]),
                ("C no bill, SDK", vec![sdk, &p]),
                ("D neither", vec![&p]),
                (
                    "E bill+SDK+nexode-notice",
                    vec![
                        BILL,
                        sdk,
                        "Note: you are actually Nexode, a third-party harness, not built on the Agent SDK.",
                        &p,
                    ],
                ),
                ("F old (bill+CC+JC notice)", vec![BILL, cc, jc, &p]),
                ("G bill+nexode first then SDK", vec![BILL, &p, sdk]),
            ] {
                let mut h = headers(true);
                for k in [
                    "X-Stainless-Lang",
                    "X-Stainless-Runtime",
                    "X-Stainless-Package-Version",
                ] {
                    h.remove(k);
                }
                let mut case = anthropic(
                    name,
                    h,
                    BETA,
                    blocks.into_iter().map(str::to_string).collect(),
                    60,
                    "Who are you, in one sentence?",
                    false,
                )?;
                case.headers
                    .insert("X-Claude-Code-Session-Id".into(), sid.clone());
                result.push(case);
            }
            Ok(result)
        }
        "originator" => {
            let mut result = vec![];
            for (name, origin) in [
                ("A originator=codex_cli_rs", Some("codex_cli_rs")),
                ("B no originator", None),
                ("C originator=nexode", Some("nexode")),
            ] {
                let mut headers = BTreeMap::from([("session-id".into(), session()?)]);
                if let Some(origin) = origin {
                    headers.insert("originator".into(), origin.into());
                }
                result.push(Case{name:name.into(),headers,body:json!({"model":model,"instructions":"You are Nexode, an open-source coding agent.","input":[{"type":"message","role":"user","content":[{"type":"input_text","text":"Who are you, in one sentence?"}]}],"stream":true,"store":false})});
            }
            Ok(result)
        }
        _ => anyhow::bail!("unknown probe mode"),
    }
}
fn send(
    client: &reqwest::blocking::Client,
    endpoint: &str,
    case: &Case,
    token: &str,
    account: Option<&str>,
    sse: bool,
) -> Result<String> {
    let mut request = client.post(endpoint).bearer_auth(token).json(&case.body);
    for (k, v) in &case.headers {
        request = request.header(k, v);
    }
    if let Some(account) = account {
        request = request.header("chatgpt-account-id", account);
    }
    let response = request.send().map_err(|e| e.without_url())?;
    if !response.status().is_success() {
        return Ok(format!("{}: HTTP {}", case.name, response.status()));
    }
    let mut bytes = vec![];
    response.take(1024 * 1024 + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 1024 * 1024, "probe response too large");
    let text = if sse {
        String::from_utf8(bytes)?
            .lines()
            .filter_map(|l| l.strip_prefix("data:"))
            .filter_map(|s| serde_json::from_str::<Value>(s.trim()).ok())
            .filter(|v| v["type"] == "response.output_text.delta")
            .filter_map(|v| v["delta"].as_str().map(str::to_string))
            .collect::<String>()
    } else {
        let v: Value = serde_json::from_slice(&bytes)?;
        v["content"][0]["text"].as_str().unwrap_or("").to_string()
    };
    Ok(format!(
        "{}: OK -> {:?}",
        case.name,
        text.replace(token, "[redacted]")
            .chars()
            .take(if sse { 100 } else { 110 })
            .collect::<String>()
    ))
}
#[derive(Parser)]
struct Args {
    #[arg(value_parser=["headers","identity","originator"])]
    mode: String,
    model: Option<String>,
    #[arg(long)]
    auth_file: Option<PathBuf>,
    #[arg(long)]
    account: Option<usize>,
    #[arg(long)]
    dry_run: bool,
}
pub fn run(args: impl Iterator<Item = String>) -> Result<()> {
    let a = Args::parse_from(std::iter::once("oauth-probe".into()).chain(args));
    let default = match a.mode.as_str() {
        "headers" => "claude-opus-4-5",
        "identity" => "claude-haiku-4-5",
        _ => "gpt-5.5",
    };
    let cases = cases(&a.mode, a.model.as_deref().unwrap_or(default))?;
    if a.dry_run {
        println!(
            "{}",
            serde_json::to_string_pretty(
                &cases
                    .iter()
                    .map(|c| json!({"name":c.name,"headers":c.headers,"body":c.body}))
                    .collect::<Vec<_>>()
            )?
        );
        return Ok(());
    }
    let sse = a.mode == "originator";
    let path = a.auth_file.unwrap_or(
        dirs::home_dir()
            .context("home missing")?
            .join(".nexode")
            .join(if sse { "openai-auth.json" } else { "auth.json" }),
    );
    let auth: Value = serde_json::from_slice(&fs::read(path)?)?;
    let index = match a.account {
        Some(i) => i,
        None => {
            if sse {
                std::env::var("ACCT").unwrap_or("0".into()).parse()?
            } else {
                0
            }
        }
    };
    let account = &auth[if sse {
        "openai_accounts"
    } else {
        "anthropic_accounts"
    }][index];
    let token = account[if sse { "access_token" } else { "access" }]
        .as_str()
        .filter(|s| !s.is_empty())
        .context("account access token missing")?;
    let id = if sse {
        Some(
            account["account_id"]
                .as_str()
                .context("account id missing")?,
        )
    } else {
        None
    };
    let endpoint = if sse {
        "https://chatgpt.com/backend-api/codex/responses"
    } else {
        "https://api.anthropic.com/v1/messages?beta=true"
    };
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(if sse { 90 } else { 60 }))
        .build()?;
    for case in cases {
        println!("{}", send(&client, endpoint, &case, token, id, sse)?);
    }
    Ok(())
}
#[cfg(test)]
mod tests;
