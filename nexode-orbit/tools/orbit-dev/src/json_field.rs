//! Strict JSON extraction for shell integrations, without interpreter dependencies.
use anyhow::{Context, Result, ensure};
use clap::Parser;
use serde_json::Value;
use std::io::Read;
#[derive(Parser)]
struct Args {
    /// RFC 6901 JSON pointer (empty string selects the root).
    pointer: String,
    /// Parse the selected string as JSON before applying a second pointer.
    #[arg(long)]
    decode: Option<String>,
    /// Return the selected array/object/string length.
    #[arg(long)]
    length: bool,
}
fn select(input: &str, a: &Args) -> Result<String> {
    let input: Value = serde_json::from_str(input).context("invalid JSON input")?;
    let mut v = input
        .pointer(&a.pointer)
        .context("JSON field missing")?
        .clone();
    if let Some(path) = &a.decode {
        v = serde_json::from_str::<Value>(
            v.as_str().context("decode field must be a JSON string")?,
        )?
        .pointer(path)
        .context("decoded JSON field missing")?
        .clone();
    }
    if a.length {
        return Ok(match v {
            Value::Array(v) => v.len(),
            Value::Object(v) => v.len(),
            Value::String(v) => v.chars().count(),
            _ => anyhow::bail!("length requires collection or string"),
        }
        .to_string());
    }
    ensure!(!v.is_null(), "JSON field is null");
    Ok(match v {
        Value::String(s) => s,
        other => other.to_string(),
    })
}
pub fn run(args: impl Iterator<Item = String>) -> Result<()> {
    let a = Args::try_parse_from(std::iter::once("json-field".into()).chain(args))?;
    let mut text = String::new();
    std::io::stdin()
        .take(16 * 1024 * 1024 + 1)
        .read_to_string(&mut text)?;
    ensure!(text.len() <= 16 * 1024 * 1024, "JSON input too large");
    println!("{}", select(&text, &a)?);
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn escaped_pointer_and_decoded_lists() {
        let a = Args {
            pointer: "/a~1b".into(),
            decode: None,
            length: false,
        };
        assert_eq!(
            select(r#"{"a/b":"line\nquoted\""}"#, &a).unwrap(),
            "line\nquoted\""
        );
        let a = Args {
            pointer: "/output".into(),
            decode: Some("".into()),
            length: true,
        };
        assert_eq!(select(r#"{"output":"[1,2,3]"}"#, &a).unwrap(), "3");
        assert!(select("{}", &a).is_err());
        assert!(select("invalid", &a).is_err());
    }
}
