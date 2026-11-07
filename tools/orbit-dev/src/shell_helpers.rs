//! Native replacements for small JSON and recording transforms used by shell entrypoints.
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::{fs, io, process::Command, time::Duration};
pub fn run(command: &str, mut args: impl Iterator<Item = String>) -> Result<()> {
    match command {
        "niri-window" => {
            let mode = args.next().context("expected focused or app")?;
            let query = if mode == "app" {
                Some(args.next().context("app id required")?)
            } else {
                ensure!(mode == "focused", "expected focused or app");
                None
            };
            ensure!(args.next().is_none(), "unexpected argument");
            let value: Value = serde_json::from_reader(io::stdin().lock())?;
            let found = value
                .as_array()
                .context("expected niri windows array")?
                .iter()
                .find(|window| {
                    query
                        .as_ref()
                        .map_or(window["is_focused"] == true, |query| {
                            window["app_id"] == *query
                        })
                });
            if let Some(window) = found {
                println!("{}", window["id"].as_u64().context("invalid window id")?);
            } else if query.is_none() {
                println!("none");
            }
        }
        "strip-selfdev-marker" => {
            let path = args.next().context("file required")?;
            ensure!(args.next().is_none(), "expected one file");
            let mut data = fs::read(&path)?;
            let marker = b"\n// bench_selfdev_build touch\n";
            if data.ends_with(marker) {
                data.truncate(data.len() - marker.len());
                fs::write(path, data)?;
            }
        }
        "kitty-window" => {
            let mode = args.next().context("expected id or title")?;
            let query = args.next().context("query required")?;
            ensure!(args.next().is_none(), "unexpected argument");
            let value: Value = serde_json::from_reader(io::stdin().lock())?;
            if let Some(value) = kitty(&value, &mode, &query)? {
                println!("{value}");
            }
        }
        "replay-keys" => {
            let path = args.next().context("recording path required")?;
            let dry = args.next();
            ensure!(
                dry.is_none() || dry.as_deref() == Some("--dry-run"),
                "expected --dry-run"
            );
            ensure!(args.next().is_none(), "unexpected argument");
            let events: Vec<Value> = serde_json::from_slice(&fs::read(path)?)?;
            let actions = actions(&events)?;
            #[cfg(unix)]
            let _signals = crate::ci_suites::unix::Signals::install()?;
            for (delay, argv) in actions {
                if dry.is_some() {
                    println!("{}", serde_json::json!({"delay_ms":delay,"argv":argv}));
                    continue;
                }
                let deadline = std::time::Instant::now() + Duration::from_millis(delay);
                while std::time::Instant::now() < deadline {
                    #[cfg(unix)]
                    ensure!(
                        crate::ci_suites::unix::interrupted().is_none(),
                        "replay interrupted"
                    );
                    std::thread::sleep(
                        deadline
                            .saturating_duration_since(std::time::Instant::now())
                            .min(Duration::from_millis(20)),
                    );
                }
                if !argv.is_empty() {
                    let output = crate::process::capture(
                        Command::new("wtype").args(argv),
                        Duration::from_secs(30),
                    )?;
                    ensure!(
                        output.status.success(),
                        "wtype failed: {}",
                        String::from_utf8_lossy(&output.stderr)
                    );
                }
            }
        }
        _ => anyhow::bail!("unknown helper {command}"),
    }
    Ok(())
}
fn kitty(value: &Value, mode: &str, query: &str) -> Result<Option<String>> {
    ensure!(matches!(mode, "id" | "title"), "expected id or title");
    let id = if mode == "title" {
        Some(query.parse::<u64>().context("invalid window id")?)
    } else {
        None
    };
    for window in value
        .as_array()
        .context("expected Kitty window array")?
        .iter()
        .flat_map(|os| os["tabs"].as_array().into_iter().flatten())
        .flat_map(|tab| tab["windows"].as_array().into_iter().flatten())
    {
        let title = window["title"].as_str().unwrap_or("");
        if mode == "id" && title.contains(query) {
            return Ok(Some(
                window["id"]
                    .as_u64()
                    .context("missing window id")?
                    .to_string(),
            ));
        }
        if id.is_some() && window["id"].as_u64() == id {
            return Ok(Some(title.into()));
        }
    }
    Ok(None)
}
fn actions(events: &[Value]) -> Result<Vec<(u64, Vec<String>)>> {
    let mut previous = 0;
    let mut result = Vec::new();
    for event in events {
        let offset = event
            .get("offset_ms")
            .map(|v| v.as_u64().context("offset_ms must be nonnegative integer"))
            .transpose()?
            .unwrap_or(0);
        let delay = offset.saturating_sub(previous);
        previous = offset;
        let mut argv = Vec::new();
        let key = &event["event"];
        if key["type"] == "Key" {
            let code = key["data"]["code"].as_str().unwrap_or("");
            let character = code
                .strip_prefix("Char('")
                .and_then(|s| s.strip_suffix("')"));
            let name = match code {
                "Enter" => Some("Return"),
                "Backspace" => Some("BackSpace"),
                "Esc" => Some("Escape"),
                "PageUp" => Some("Page_Up"),
                "PageDown" => Some("Page_Down"),
                "Tab" | "Up" | "Down" | "Left" | "Right" | "Home" | "End" | "Delete" | "Insert" => {
                    Some(code)
                }
                _ if code
                    .strip_prefix('F')
                    .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())) =>
                {
                    Some(code)
                }
                _ => None,
            };
            if character.is_some() || name.is_some() {
                let modifiers: Vec<_> = key["data"]["modifiers"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .filter(|s| matches!(*s, "ctrl" | "alt" | "shift"))
                    .collect();
                for modifier in &modifiers {
                    argv.extend(["-M".into(), (*modifier).into()]);
                }
                if let Some(name) = name {
                    argv.extend(["-k".into(), name.into()]);
                }
                for modifier in modifiers.iter().rev() {
                    // For text input, keep modifiers active until after the text argument.
                    if character.is_none() {
                        argv.extend(["-m".into(), (*modifier).into()]);
                    }
                }
                if let Some(text) = character {
                    // wtype parses interspersed options, so release modifiers after text.
                    // Leading '-' needs --; a separate release invocation is unnecessary
                    // because wtype always releases its virtual keyboard when it exits.
                    argv.push("--".into());
                    argv.push(text.into());
                }
            }
        }
        result.push((delay, argv));
    }
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn replay_keeps_unicode_shell_characters_modifiers_and_delays() {
        let events = vec![
            json!({"offset_ms":25,"event":{"type":"Key","data":{"code":"Char('$')","modifiers":["ctrl"]}}}),
            json!({"offset_ms":100,"event":{"type":"Key","data":{"code":"Enter","modifiers":["alt","shift"]}}}),
            json!({"offset_ms":80,"event":{"type":"Resize"}}),
        ];
        let out = actions(&events).unwrap();
        assert_eq!(
            out[0],
            (
                25,
                vec!["-M".into(), "ctrl".into(), "--".into(), "$".into()]
            )
        );
        assert_eq!(
            out[1],
            (
                75,
                vec![
                    "-M", "alt", "-M", "shift", "-k", "Return", "-m", "shift", "-m", "alt"
                ]
                .into_iter()
                .map(String::from)
                .collect()
            )
        );
        assert_eq!(out[2], (0, vec![]));
    }
    #[test]
    fn kitty_searches_nested_windows_without_evaluating_titles() {
        let value = json!([{"tabs":[{"windows":[{"id":7,"title":"demo-$(touch bad)"}]}]}]);
        assert_eq!(
            kitty(&value, "id", "$(touch bad)").unwrap(),
            Some("7".into())
        );
        assert_eq!(
            kitty(&value, "title", "7").unwrap(),
            Some("demo-$(touch bad)".into())
        );
        assert_eq!(kitty(&value, "id", "missing").unwrap(), None);
    }
}
