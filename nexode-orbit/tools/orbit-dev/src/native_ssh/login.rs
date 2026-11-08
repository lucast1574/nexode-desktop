use super::*;
use super::{bridge::Bridge, tui::Tui};
use std::time::Instant;
const CANCEL: &str = "Pending authorization was removed on the remote host.";
fn inspect(c: &Config, legacy: &Value, needles: &[String]) -> Result<Value> {
    let s = remote::call(c, "inspect", json!({"needles":needles}))?;
    ensure!(
        s["leaks"] == json!([]) && s["credentials"] == json!([]),
        "Sensitive input or unexpected credentials in remote state"
    );
    ensure!(
        s["legacy_sha256"] == *legacy,
        "Changed another invocation's pending state"
    );
    Ok(s)
}
fn wait_call(c: &Config, t: &mut Tui, operation: &str, provider: Option<&str>) -> Result<Value> {
    let end = Instant::now() + TIMEOUT;
    loop {
        t.pump(Duration::from_millis(200))?;
        let s = remote::call(c, "inspect", json!({"needles":[]}))?;
        if s["calls"]
            .as_array()
            .context("calls")?
            .iter()
            .any(|v| v["operation"] == operation && provider.is_none_or(|p| v["provider"] == p))
        {
            return Ok(s);
        }
        ensure!(
            t.alive()? && Instant::now() < end,
            "Remote login subprocess did not record expected operation"
        );
    }
}
fn verify_url(p: &Value, c: &Value) -> Result<String> {
    ensure!(
        p["mode"] == 384
            && p["relative_path"]
                == format!(
                    "pending-login/flows/{}/openai.json",
                    c["flow_id"].as_str().context("flow")?
                ),
        "Pending login must be private and flow-scoped"
    );
    let s = c["auth_url"].as_str().context("auth URL")?;
    let u = url::Url::parse(s).map_err(|_| anyhow::anyhow!("Invalid authorization URL"))?;
    ensure!(
        u.scheme() == "https" && u.host_str() == Some("auth.openai.com"),
        "Not a real OpenAI auth URL"
    );
    let q: Vec<_> = u.query_pairs().collect();
    for (key, expected) in [
        ("state", p["state"].as_str().context("state")?),
        (
            "code_challenge",
            p["challenge"].as_str().context("challenge")?,
        ),
        (
            "redirect_uri",
            p["redirect_uri"].as_str().context("redirect")?,
        ),
        ("code_challenge_method", "S256"),
    ] {
        let got: Vec<_> = q
            .iter()
            .filter(|(k, _)| k == key)
            .map(|(_, v)| v.as_ref())
            .collect();
        ensure!(
            got == [expected],
            "Remote URL differs from pending state/PKCE"
        );
    }
    Ok(s.into())
}
fn start_tui(
    c: &Config,
    e: &BTreeMap<String, String>,
    root: &Path,
    id: &str,
    sentinel: &str,
) -> Result<Tui> {
    let mut t = Tui::new(c, e, root, id, false)?;
    t.wait(&format!("SSH {}", c.host), 0)?;
    t.wait(sentinel, 0)?;
    Ok(t)
}
fn scenarios(c: &Config, legacy: &Value, root: &Path) -> Result<()> {
    let e = env(root, true);
    let (h, id, sentinel) = bridge::seed(c, "NEXODE_SSH_LOGIN_CONTEXT_")?;
    let mut needles = vec![];
    for accept in [true, false] {
        let mut t = start_tui(c, &e, root, &id, &sentinel)?;
        t.startup(accept)?;
        if !accept {
            t.catalog()?;
        }
        let s = inspect(c, legacy, &needles)?;
        ensure!(
            s["pending"] == json!([]) && s["credentials"] == json!([]) && s["calls"] == json!([]),
            "Startup choice began authentication"
        );
        auth::local_clean(root, &needles)?;
        let n = t.command("/cancel")?;
        t.wait("No authorization was started.", n)?;
        t.quit(false)?;
        println!("PASS empty-host startup choice {accept}: no credentials or OAuth");
    }
    {
        let mut t = start_tui(c, &e, root, &id, &sentinel)?;
        t.dismiss()?;
        let n = t.command("/login")?;
        t.wait(&format!("Login on {}:", c.host), n)?;
        t.catalog()?;
        ensure!(
            inspect(c, legacy, &needles)?["pending"] == json!([]),
            "Bare login started OAuth"
        );
        let n = t.command("/cancel")?;
        t.wait("No authorization was started.", n)?;
        let n = t.command("/login openai-api")?;
        t.wait(&format!("OpenAI API setup on {}", c.host), n)?;
        t.wait("no local credentials were accessed", n)?;
        ensure!(
            inspect(c, legacy, &needles)?["pending"] == json!([]),
            "API route started OAuth"
        );
        let n = t.command("/login openai")?;
        t.wait("https://auth.openai.com/", n)?;
        let s = wait_call(c, &mut t, "start", Some("openai"))?;
        let pending = s["pending"].as_array().context("pending")?;
        ensure!(pending.len() == 1, "Expected one OpenAI pending flow");
        let p = &pending[0];
        let starts: Vec<_> = s["calls"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|v| v["operation"] == "start")
            .collect();
        ensure!(
            starts.len() == 1 && starts[0]["exit_code"] == 0,
            "Remote OAuth did not start"
        );
        t.wait(&verify_url(p, starts[0])?, n)?;
        auth::local_clean(root, &needles)?;
        let state = format!("not-the-remote-state-{}", unique()?);
        let code = format!("NEXODE_SSH_LOGIN_INVALID_{}", unique()?);
        let callback = format!("http://localhost:1455/auth/callback?code={code}&state={state}");
        auth::callback_valid(
            callback.as_bytes(),
            &[p["state"].as_str().context("state")?.into()],
        )?;
        needles.extend([callback.clone(), code.clone()]);
        let n = t.mark();
        t.write(format!("\x1b[200~{callback}\x1b[201~\r").as_bytes())?;
        t.wait("SSH login failed", n)?;
        ensure!(
            !t.text().contains(&code),
            "Sensitive callback was displayed"
        );
        let s = inspect(c, legacy, &needles)?;
        let completed: Vec<_> = s["calls"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|v| v["operation"] == "callback_stdin")
            .collect();
        ensure!(
            completed.len() == 1
                && completed[0]["exit_code"] != 0
                && completed[0]["mismatch_verified"] == true
                && completed[0]["state_mismatch"] == true,
            "Synthetic callback did not fail before token exchange"
        );
        auth::local_clean(root, &needles)?;
        let n = t.command("/cancel")?;
        t.wait(CANCEL, n)?;
        let s = inspect(c, legacy, &needles)?;
        ensure!(
            s["pending"] == json!([])
                && s["calls"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|v| v["operation"] == "cancel"
                        && v["exit_code"] == 0
                        && v["flow_id"] == starts[0]["flow_id"]),
            "Cancel did not remove exact flow"
        );
        t.quit(false)?;
        println!("PASS OpenAI PKCE URL, private mismatched callback, flow-scoped cancel");
    }
    {
        let mut t = start_tui(c, &e, root, &id, &sentinel)?;
        t.dismiss()?;
        let n = t.command("/login not-a-provider")?;
        t.wait("SSH login supports:", n)?;
        ensure!(
            inspect(c, legacy, &needles)?["pending"] == json!([]),
            "Unsupported provider created state"
        );
        let marker = format!("DO_NOT_ECHO_REMOTE_AUTH_ERROR_{}", unique()?);
        remote::call(c, "inject_failure", json!({"error_marker":marker}))?;
        let n = t.command("/login openai")?;
        wait_call(c, &mut t, "injected_start_failure", None)?;
        t.wait("SSH login failed", n)?;
        ensure!(
            !t.text().contains(&marker),
            "Sensitive remote stderr was exposed"
        );
        needles.push(marker);
        ensure!(
            inspect(c, legacy, &needles)?["pending"] == json!([]),
            "Failed remote login left state"
        );
        let n = t.command("/cancel")?;
        t.wait(CANCEL, n)?;
        t.quit(false)?;
    }
    {
        let mut t = start_tui(c, &e, root, &id, &sentinel)?;
        t.dismiss()?;
        t.command("/login claude")?;
        let s = wait_call(c, &mut t, "start", Some("claude"))?;
        let starts: Vec<_> = s["calls"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|v| v["operation"] == "start" && v["provider"] == "claude")
            .collect();
        ensure!(
            starts.len() == 1 && starts[0]["exit_code"] == 0,
            "Claude authorization failed to begin"
        );
        let call = starts[0];
        ensure!(
            call["pkce_matches"] == true && call["legacy_state_is_verifier"] == true,
            "Claude VM-side PKCE mismatch"
        );
        let pending = s["pending"].as_array().context("pending")?;
        ensure!(
            pending.len() == 1
                && pending[0]["mode"] == 384
                && pending[0]["relative_path"]
                    == format!(
                        "pending-login/flows/{}/claude.json",
                        call["flow_id"].as_str().context("flow")?
                    ),
            "Claude pending state not private and scoped"
        );
        let len = call["auth_url_length"]
            .as_u64()
            .context("private URL length")? as usize;
        ensure!(len > 0 && len <= 16384, "Invalid private URL length");
        let end = Instant::now() + TIMEOUT;
        let private_url = loop {
            t.pump(Duration::from_millis(150))?;
            let text = t.text();
            let found = text.match_indices("https://").find_map(|(i, _)| {
                text.get(i..i + len)
                    .filter(|s| auth::digest(s.as_bytes()) == call["auth_url_sha256"])
                    .map(str::to_owned)
            });
            if let Some(url) = found {
                break url;
            }
            ensure!(
                t.alive()? && Instant::now() < end,
                "Private hash-matched Claude URL missing; output withheld"
            );
        };
        needles.push(private_url);
        auth::local_clean(root, &needles)?;
        inspect(c, legacy, &needles)?;
        let n = t.command("/cancel")?;
        t.wait(CANCEL, n)?;
        let s = inspect(c, legacy, &needles)?;
        ensure!(
            s["pending"] == json!([])
                && s["calls"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|v| v["operation"] == "cancel"
                        && v["provider"] == "claude"
                        && v["flow_id"] == call["flow_id"]
                        && v["exit_code"] == 0),
            "Claude cancel did not remove exact flow"
        );
        t.quit(false)?;
        println!("PASS Claude begin/cancel, private URL and VM-side PKCE; no completion");
    }
    auth::local_clean(root, &needles)?;
    no_transcript(&root.join("nexode"), &id, &sentinel)?;
    let mut b = Bridge::new(c)?;
    b.handshake()?;
    let history = b.subscribe(&c.cwd, &unique()?, Some(&id))?;
    ensure!(
        !needles.iter().any(|n| contains(&history, n)),
        "Sensitive input in remote transcript"
    );
    for m in history["messages"].as_array().context("messages")? {
        ensure!(m["role"] != "assistant", "Unexpected model turn");
        ensure!(
            m["role"] != "user" || m.to_string().contains(&sentinel),
            "Login command became a model prompt"
        );
    }
    b.finish()?;
    let s = inspect(c, legacy, &needles)?;
    ensure!(
        s["processes"].as_array().is_some_and(|p| !p.is_empty()),
        "Missing isolated daemon identity"
    );
    println!(
        "{}",
        json!({"status":"passed","host":c.host,"session_id":id,"remote_version":h["version"],"isolated_remote_root":c.root,"server_socket":c.socket,"daemon_identities":s["processes"],"provider_turns_requested":0,"oauth_completed":false,"token_exchange":"refused before exchange by real CLI state validation"})
    );
    Ok(())
}
pub(super) fn run(c: Config) -> Result<()> {
    let (c, created) = remote::create(&c, "login", json!({}))?;
    let root = tempfile::Builder::new()
        .prefix("nexode-ssh-login-")
        .tempdir()?;
    init_root(root.path())?;
    let result = scenarios(&c, &created["legacy_sha256"], root.path());
    match remote::call(&c, "inspect", json!({"needles":[]})) {
        Ok(s) => println!(
            "{}",
            json!({"isolated_remote_root":c.root,"server_socket":c.socket,"daemon_identities":s["processes"]})
        ),
        Err(_) => eprintln!("Could not identify private daemon; inspect reported root/socket"),
    };
    let cleanup = remote::call(&c, "cleanup_pending", json!({}));
    result?;
    cleanup?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn prompt_matches_exact_state_and_challenge() {
        let p = json!({"mode":384,"relative_path":"pending-login/flows/flow/openai.json","state":"state","challenge":"pkce","redirect_uri":"http://localhost/auth/callback"});
        let c = json!({"flow_id":"flow","auth_url":"https://auth.openai.com/oauth/authorize?state=state&code_challenge=pkce&redirect_uri=http%3A%2F%2Flocalhost%2Fauth%2Fcallback&code_challenge_method=S256"});
        verify_url(&p, &c).unwrap();
        let mut bad = p;
        bad["state"] = "other".into();
        assert!(verify_url(&bad, &c).is_err());
    }
}
