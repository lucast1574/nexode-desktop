use anyhow::{Result, ensure};
use serde_json::json;
use std::{fs, io::Write, path::Path};
pub fn run(name: &str) -> Result<i32> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let mut log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(std::env::var("TEST_LOG")?)?;
    writeln!(log, "{}", json!([name, args]))?;
    match name {
        "uname" => println!("Darwin"),
        "security" if args[0] == "list-keychains" => {
            if !args.iter().any(|a| a == "-s") {
                println!(
                    "    \"/Users/runner/Library/Keychains/login.keychain-db\"\n    \"/Library/Keychains/Fixture With Spaces.keychain-db\""
                );
            } else if std::env::var_os("TEST_BAD_SEARCH_LIST").is_some()
                && args.iter().any(|a| a.contains("handoff.keychain-db"))
            {
                return Ok(1);
            }
        }
        "security" if args[0] == "find-identity" => {
            let mut identity = std::env::var("APPLE_SIGNING_IDENTITY")?;
            if std::env::var_os("TEST_WRONG_IDENTITY").is_some() {
                identity.push_str(" wrong");
            }
            println!("  1) {} \"{identity}\"", "A".repeat(40));
            if std::env::var_os("TEST_DUPLICATE_IDENTITY").is_some() {
                println!("  2) {} \"{identity}\"", "B".repeat(40));
            }
        }
        "codesign" if args.iter().any(|a| a == "--verify") => {
            let index = args.iter().position(|a| a == "-R").unwrap();
            ensure!(
                args[index + 1].starts_with('='),
                "Inline requirement must start with ="
            );
            if std::env::var_os("TEST_BAD_CHAIN").is_some() {
                return Ok(1);
            }
        }
        "ditto" => fs::write(Path::new(args.last().unwrap()), b"fixture zip")?,
        "xcrun" => println!("{}", json!({"status":"Accepted","id":"fixture-submission"})),
        _ => {}
    }
    Ok(0)
}
