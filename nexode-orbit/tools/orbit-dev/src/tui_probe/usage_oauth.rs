use super::*;
use std::{fs, process::Command};
const ENDPOINT: &str = "https://api.anthropic.com/api/oauth/usage";
#[derive(Parser)]
struct Args {
    #[arg(long)]
    debug_socket: Option<PathBuf>,
    #[arg(long)]
    credentials: Option<PathBuf>,
    #[arg(long, default_value = "claude")]
    claude: PathBuf,
    #[arg(long, default_value = "What is 2+2? Reply with just the number.")]
    prompt: String,
    #[arg(long, default_value = "/tmp/oauth-test")]
    working_dir: PathBuf,
    /// Override only for a local HTTP fixture.
    #[arg(long, default_value = ENDPOINT)]
    usage_endpoint: String,
}
fn quota(client: &reqwest::blocking::Client, endpoint: &str, token: &str) -> Result<Value> {
    let response = client
        .get(endpoint)
        .bearer_auth(token)
        .header("anthropic-beta", "oauth-2025-04-20,claude-code-20250219")
        .header("Accept", "application/json")
        .header("User-Agent", "claude-cli/1.0.0")
        .send()
        .map_err(|e| e.without_url())?;
    ensure!(
        response.status().is_success(),
        "quota API returned HTTP {}",
        response.status()
    );
    Ok(response.json()?)
}
fn utilization(value: &Value) -> Result<f64> {
    value["five_hour"]["utilization"]
        .as_f64()
        .context("missing five-hour quota utilization")
}
pub(super) fn run(args: impl Iterator<Item = String>) -> Result<i32> {
    let a = Args::parse_from(std::iter::once("test-oauth-usage".into()).chain(args));
    let endpoint = url::Url::parse(&a.usage_endpoint)?;
    ensure!(
        a.usage_endpoint == ENDPOINT
            || (endpoint.scheme() == "http"
                && matches!(
                    endpoint.host_str(),
                    Some("127.0.0.1" | "localhost" | "[::1]")
                )),
        "quota endpoint must be Anthropic or a loopback fixture"
    );
    ensure!(
        endpoint.username().is_empty() && endpoint.password().is_none(),
        "endpoint must not contain credentials"
    );
    let credentials = a.credentials.unwrap_or(
        dirs::home_dir()
            .context("home unavailable")?
            .join(".claude/.credentials.json"),
    );
    let credentials: Value = serde_json::from_slice(&fs::read(credentials)?)?;
    let token = credentials["claudeAiOauth"]["accessToken"]
        .as_str()
        .filter(|t| !t.is_empty())
        .context("OAuth access token missing")?;
    let http = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let before = quota(&http, &a.usage_endpoint, token)?;
    let start = Instant::now();
    let output = crate::process::capture_interruptible(
        Command::new(&a.claude).args(["-p", &a.prompt, "--output-format", "json"]),
        Duration::from_secs(120),
        || crate::ci_suites::unix::interrupted().is_some(),
    )?;
    let cli_time = start.elapsed().as_secs_f64();
    ensure!(
        output.status.success(),
        "Claude CLI exited with {}",
        output.status
    );
    let cli: Value = serde_json::from_slice(&output.stdout)?;
    ensure!(cli["is_error"] != true, "Claude CLI reported an error");
    let cli_usage = usage(&cli["usage"])?;
    super::super::isolated::pause(1.0)?;
    let after_cli = quota(&http, &a.usage_endpoint, token)?;
    let socket = a.debug_socket.unwrap_or_else(|| {
        PathBuf::from(format!("/run/user/{}/nexode-debug.sock", unsafe {
            libc::getuid()
        }))
    });
    let session = Session {
        id: io::create_session(&socket, &a.working_dir)?,
        socket,
    };
    let state: Value = serde_json::from_str(&session.command("state")?)?;
    let start = Instant::now();
    let response = session.command(&format!("message:{}", a.prompt))?;
    let nexode_time = start.elapsed().as_secs_f64();
    let nexode_usage = usage(&serde_json::from_str::<Value>(&session.command("usage")?)?)?;
    drop(session);
    super::super::isolated::pause(1.0)?;
    let after_nexode = quota(&http, &a.usage_endpoint, token)?;
    let sum = |t: [u64; 4]| t.iter().map(|v| *v as f64).sum::<f64>();
    let total_cli = sum(cli_usage);
    let total_nexode = sum(nexode_usage);
    let report = json!({"prompt":a.prompt,
        "claude_cli":{"response":cli["result"],"usage":metrics(cli_usage),"cost":cli["total_cost_usd"],"model_usage":cli["modelUsage"],"time":cli_time},
        "nexode":{"response":response,"usage":metrics(nexode_usage),"time":nexode_time,"provider":state["provider"],"model":state["model"]},
        "quota":{"before":before,"after_cli":after_cli,"after_nexode":after_nexode,
            "cli_delta":utilization(&after_cli)?-utilization(&before)?,"nexode_delta":utilization(&after_nexode)?-utilization(&after_cli)?},
        "comparison":{"speedup":if nexode_time > 0.0 {Some(cli_time/nexode_time)} else {None},"token_savings_percent":if total_cli>0.0 {Some(100.0*(1.0-total_nexode/total_cli))} else {None}},
        "notes":["Small requests may not visibly change the five-hour quota percentage.","Effective tokens use a diagnostic 10% cache-read weight, not a quota or price guarantee."]});
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(0)
}
