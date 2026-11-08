use serde_json::json;
use std::{fs, io::Read, path::PathBuf, time::Duration};
pub fn run() -> anyhow::Result<i32> {
    let home = PathBuf::from(std::env::var_os("NEXODE_HOME").expect("isolated auth home"));
    let args: Vec<String> = std::env::args().skip(1).collect();
    let option = |name: &str| {
        args.iter()
            .position(|s| s == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
            .expect("required auth argument")
    };
    let provider = option("--provider");
    let flow = option("--flow-id");
    let mode = fs::read_to_string(home.join("mode"))?;
    fs::write(home.join("argv"), serde_json::to_vec(&args)?)?;
    if args.iter().any(|a| a == "--cancel") {
        fs::write(home.join(format!("cancel-{flow}")), "cancelled")?;
        println!("{}", json!({"status":"cancelled","provider":provider}));
        return Ok(0);
    }
    if args.iter().any(|a| a == "--print-auth-url") {
        if mode == "oversized" {
            println!("{}", "X".repeat(70000));
            return Ok(0);
        }
        if mode == "bad-json" {
            println!("private-fixture-secret");
            return Ok(1);
        }
        let auth_url = fs::read_to_string(home.join("auth-url"))
            .unwrap_or_else(|_| "https://example.com/oauth?state=private-fixture-secret".into());
        println!(
            "{}",
            json!({"status":"pending","provider":provider,"auth_url":auth_url,"input_kind":if provider=="copilot"{"complete"}else{"callback_url"},"user_code":"ABCD-1234","expires_at_ms":9999999999999_u64})
        );
        return Ok(0);
    }
    if mode == "hang" {
        fs::write(home.join("pid"), std::process::id().to_string())?;
        std::thread::sleep(Duration::from_secs(60));
        return Ok(1);
    }
    if mode == "exchange-error" {
        eprintln!("Error: Token exchange failed: private-fixture-secret");
        return Ok(1);
    }
    let mut payload = String::new();
    std::io::stdin().read_to_string(&mut payload)?;
    fs::write(home.join("callback-input"), &payload)?;
    fs::write(
        home.join("stdin-ok"),
        if payload == "private-fixture-secret" {
            "True"
        } else {
            "False"
        },
    )?;
    eprintln!("private-fixture-secret");
    println!("{}", json!({"status":"authenticated","provider":provider}));
    if mode.starts_with("legacy-") {
        println!(
            "=== auth-test: Fixture ===\nresult: {}",
            if mode == "legacy-warning" {
                "FAIL"
            } else {
                "PASS"
            }
        );
    }
    Ok(i32::from(matches!(
        mode.as_str(),
        "warning" | "legacy-warning"
    )))
}
