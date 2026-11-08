mod support;
use serde_json::{Value, json};
use std::{fs, path::Path, process::Command};
fn run(path: &Path, args: &[&str], code: i32) -> String {
    let out = Command::new(support::binary("orbit-dev"))
        .env_clear()
        .env("PATH", "")
        .env("NEXODE_DEMO_SHOP_STATE", path)
        .arg("demo-shop")
        .args(args)
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(code),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}
#[test]
fn offline_shop_filters_preserves_cart_and_never_places_orders() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.json");
    let out = run(
        &path,
        &["search", "65W", "USB-C", "charger", "--max-price", "50"],
        0,
    );
    assert!(out.contains("charger-65w"));
    assert!(!out.contains("charger-100w"));
    run(&path, &["search", "nonexistent"], 1);
    run(&path, &["show", "unknown"], 2);
    run(&path, &["checkout"], 2);
    run(&path, &["add", "charger-65w"], 0);
    let out = run(&path, &["checkout"], 3);
    for marker in [
        "Checkout total: $43.19",
        "no payment method is available",
        "No account was created",
        "Stop before creating or funding a prepaid card",
    ] {
        assert!(out.contains(marker));
    }
    for absent in [
        "capability",
        "setup instructions",
        "agentcard",
        "discover_tools",
    ] {
        assert!(!out.to_lowercase().contains(absent));
    }
    run(
        &path,
        &["prepare-checkout", "charger-65w", "--max-total", "50"],
        3,
    );
    assert!(
        run(
            &path,
            &["prepare-checkout", "charger-100w", "--max-total", "50"],
            2
        )
        .contains("exceeds limit")
    );
    let state: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(state, json!({"cart":["charger-65w"]}));
    assert!(run(&path, &["reset"], 0).contains("Cart is empty"));
    assert!(run(&path, &["cart"], 0).contains("Demo cart is empty"));
}
#[test]
fn launcher_keeps_prompt_blind_and_targets_only_new_demo_sessions() {
    let text = support::read("scripts/launch_agentcard_discovery_demo.sh");
    let prompt = text.lines().find(|s| s.starts_with("PROMPT=")).unwrap();
    let lower = prompt.to_lowercase();
    for present in ["Use `./bin/nexode-demo-shop`", "USB-C laptop charger"] {
        assert!(prompt.contains(present));
    }
    for present in [
        "work through any prerequisites",
        "ask me for confirmation immediately before",
        "prepaid card",
    ] {
        assert!(lower.contains(present));
    }
    for absent in [
        "agentcard",
        "discover_tools",
        "discovery",
        "missing capability",
        "setup instructions",
        "prepare-checkout",
        "charger-65w",
    ] {
        assert!(!lower.contains(absent));
    }
    let dir = tempfile::tempdir().unwrap();
    let before = dir.path().join("before");
    let after = dir.path().join("after");
    fs::write(
        &before,
        json!({"clients":[{"session_id":"old"}]}).to_string(),
    )
    .unwrap();
    fs::write(&after,json!({"clients":[{"session_id":"old","working_dir":"demo","connected_secs_ago":0},{"session_id":"other","working_dir":"other","connected_secs_ago":0},{"session_id":"slow","working_dir":"demo","connected_secs_ago":10},{"session_id":"new","working_dir":"demo","connected_secs_ago":1}]}).to_string()).unwrap();
    let output = support::success(
        Command::new(support::binary("orbit-dev"))
            .arg("demo-session")
            .arg(before)
            .arg(after)
            .arg("demo")
            .output()
            .unwrap(),
    );
    assert_eq!(output.trim(), "new");
}
#[cfg(unix)]
#[test]
fn executable_alias_runs_the_same_native_cli() {
    let dir = tempfile::tempdir().unwrap();
    let alias = dir.path().join("nexode-demo-shop");
    std::os::unix::fs::symlink(support::binary("orbit-dev"), &alias).unwrap();
    assert!(
        support::success(
            Command::new(alias)
                .env_clear()
                .env("NEXODE_DEMO_SHOP_STATE", dir.path().join("state"))
                .args(["search", "65w"])
                .output()
                .unwrap()
        )
        .contains("charger-65w")
    );
}
