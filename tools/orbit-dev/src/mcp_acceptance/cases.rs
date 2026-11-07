use super::*;
use std::os::unix::fs::PermissionsExt;

fn listing(replay: &Replay<'_>, auth: &str) -> Result<()> {
    let before = replay.fixture.events().len();
    let rows: Value = serde_json::from_str(&replay.command(&["list", "--json"], true, 12)?)?;
    ensure!(
        rows.as_array().is_some_and(|rows| rows.len() == 1),
        "isolated list must contain one server"
    );
    ensure!(rows[0]["auth"] == auth, "unexpected authentication status");
    ensure!(
        replay.fixture.events().len() == before,
        "list performed network request"
    );
    Ok(())
}
fn unauthenticated(replay: &Replay<'_>) -> Result<()> {
    let before = replay.fixture.events().len();
    let start = Instant::now();
    let text = replay.command(&["test", "remote"], false, 8)?;
    ensure!(
        text.contains("mcp login remote"),
        "missing actionable login guidance"
    );
    ensure!(
        start.elapsed() < Duration::from_secs(8),
        "unauthenticated test not fail-fast"
    );
    let events = replay.fixture.events();
    let events = &events[before..];
    ensure!(
        events.len() == 1
            && events[0]["verb"] == "POST"
            && events[0]["path"] == "/mcp"
            && events[0]["rpc"] == "initialize"
            && events[0]["auth"] == "absent"
            && events[0]["status"] == 401,
        "unauthenticated test did not stop at initialize 401"
    );
    Ok(())
}
fn authenticated(replay: &Replay<'_>, name: &str, kind: &str) -> Result<()> {
    let before = replay.fixture.events().len();
    let text = replay.command(&["test", name], true, 12)?;
    ensure!(
        text.contains("disconnected cleanly") && text.contains("Tools: 2"),
        "test did not report clean shutdown/tools"
    );
    ensure!(
        text.find("alpha_fixture")
            .zip(text.find("zeta_fixture"))
            .is_some_and(|(a, z)| a < z),
        "CLI tools not sorted"
    );
    let events = replay.fixture.events();
    let events = &events[before..];
    let rpc: Vec<_> = events.iter().filter(|e| e["rpc"].is_string()).collect();
    ensure!(
        rpc.iter().map(|e| e["rpc"].as_str().unwrap()).eq([
            "initialize",
            "notifications/initialized",
            "tools/list"
        ]),
        "MCP handshake order mismatch"
    );
    ensure!(
        events.iter().all(|e| e["auth"] == kind),
        "authentication header missing or incorrect"
    );
    ensure!(
        rpc[1..]
            .iter()
            .all(|e| e["session"] == "fixture-session" && e["protocol"] == "2025-06-18"),
        "session/protocol headers not propagated"
    );
    ensure!(
        events
            .iter()
            .any(|e| e["verb"] == "DELETE" && e["session"] == "fixture-session"),
        "no session DELETE shutdown"
    );
    Ok(())
}
fn case(
    replay: &Replay<'_>,
    reports: &mut Vec<Value>,
    name: &str,
    operation: impl FnOnce() -> Result<()>,
) {
    let start = Instant::now();
    let begin = replay.fixture.events().len();
    let passed = operation().is_ok() && replay.fixture.errors().is_empty();
    // Neither captured CLI output nor transport errors (which can contain OAuth URLs) enter reports.
    let mut result = json!({"name":name,"pass":passed,"seconds":(start.elapsed().as_secs_f64()*10000.0).round()/10000.0,"http_begin":begin,"http_end":replay.fixture.events().len()});
    if !passed {
        result["error"] = json!("acceptance assertion or fixture failed");
    }
    println!("{} {name}", if passed { "PASS" } else { "FAIL" });
    reports.push(result);
}
pub(super) fn run(replay: &Replay<'_>) -> Vec<Value> {
    let mut reports = vec![];
    case(replay, &mut reports, "list_no_network", || {
        listing(replay, "no OAuth credentials stored")?;
        let rows: Value = serde_json::from_str(&replay.command(&["list", "--json"], true, 12)?)?;
        ensure!(
            rows[0]["name"] == "remote",
            "isolated list missing configured server"
        );
        Ok(())
    });
    case(
        replay,
        &mut reports,
        "without_credentials_fail_fast_no_browser",
        || unauthenticated(replay),
    );
    case(
        replay,
        &mut reports,
        "login_discovery_dcr_pkce_real_callback_no_browser",
        || replay.login(),
    );
    let store = replay.home.join("mcp-oauth.json");
    case(
        replay,
        &mut reports,
        "stored_token_private_mode_and_offline_status",
        || {
            ensure!(
                fs::metadata(&store)?.permissions().mode() & 0o777 == 0o600,
                "credential file not mode 0600"
            );
            ensure!(
                fs::metadata(&replay.home)?.permissions().mode() & 0o777 == 0o700,
                "credential directory not mode 0700"
            );
            ensure!(
                fs::read_to_string(&store)?.contains(&replay.fixture.token),
                "issued token not stored"
            );
            listing(replay, "OAuth credentials stored")
        },
    );
    case(
        replay,
        &mut reports,
        "authenticated_tools_headers_order_shutdown",
        || authenticated(replay, "remote", "oauth"),
    );
    case(replay, &mut reports, "logout", || {
        let before = replay.fixture.events().len();
        ensure!(
            replay
                .command(&["logout", "remote"], true, 12)?
                .contains("Removed local OAuth credentials"),
            "logout did not remove credentials"
        );
        listing(replay, "no OAuth credentials stored")?;
        ensure!(
            !store.exists() || !fs::read_to_string(&store)?.contains(&replay.fixture.token),
            "logout retained access token"
        );
        ensure!(
            before == replay.fixture.events().len(),
            "logout performed network"
        );
        Ok(())
    });
    case(
        replay,
        &mut reports,
        "after_logout_fail_fast_no_browser",
        || unauthenticated(replay),
    );
    case(
        replay,
        &mut reports,
        "static_authorization_over_malformed_oauth_store",
        || {
            replay.config(json!({"static":{"type":"http","url":format!("{}/mcp",replay.fixture.base),"headers":{"aUtHoRiZaTiOn":format!("Bearer {}",replay.fixture.static_token)}}}))?;
            fs::write(&store, "malformed-store-private-sentinel")?;
            listing(replay, "static Authorization header")?;
            authenticated(replay, "static", "static")?;
            ensure!(
                fs::read_to_string(&store)? == "malformed-store-private-sentinel",
                "static test altered OAuth store"
            );
            Ok(())
        },
    );
    for (name, path, transport, case_name) in [
        ("legacy", "/legacy", "sse", "explicit_legacy_sse"),
        (
            "fallback",
            "/fallback",
            "http",
            "http405_legacy_sse_fallback",
        ),
    ] {
        case(replay, &mut reports, case_name, || {
            replay.config(json!({name:{"type":transport,"url":format!("{}{path}",replay.fixture.base),"headers":{"Authorization":format!("Bearer {}",replay.fixture.static_token)}}}))?;
            let before = replay.fixture.events().len();
            let text = replay.command(&["test", name], true, 15)?;
            ensure!(
                text.contains("disconnected cleanly") && text.contains("Tools: 2"),
                "legacy CLI test failed"
            );
            let events = replay.fixture.events();
            let events = &events[before..];
            ensure!(
                events.iter().all(|e| e["auth"] == "static"),
                "legacy static auth absent"
            );
            ensure!(
                events
                    .iter()
                    .any(|e| e["verb"] == "GET" && e["path"] == path),
                "legacy SSE stream not opened"
            );
            ensure!(
                events
                    .iter()
                    .filter(|e| e["path"] == format!("/messages{path}"))
                    .map(|e| e["rpc"].as_str().unwrap_or(""))
                    .eq(["initialize", "notifications/initialized", "tools/list"]),
                "legacy handshake order mismatch"
            );
            if name == "fallback" {
                ensure!(
                    events
                        .first()
                        .is_some_and(|e| e["verb"] == "POST" && e["rpc"] == "initialize"),
                    "HTTP405 fallback did not probe HTTP first"
                );
            }
            Ok(())
        });
    }
    reports
}
