//! Release maintenance. All network mutations are explicit CLI actions.
mod arm;
pub mod ci;
mod notes;
mod preview;
mod recovery;
mod verify;
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::{process::Command, time::Duration};
pub trait Github {
    fn command(&mut self, args: &[String]) -> Result<String>;
    fn api(&mut self, endpoint: &str, paginated: bool) -> Result<Value> {
        let mut args = vec!["api".into(), endpoint.into()];
        if paginated {
            args.extend(["--paginate".into(), "--slurp".into()]);
        }
        serde_json::from_str(&self.command(&args)?).context("malformed GitHub API response")
    }
}
pub struct Gh;
impl Github for Gh {
    fn command(&mut self, args: &[String]) -> Result<String> {
        let result =
            crate::process::capture(Command::new("gh").args(args), Duration::from_secs(60))
                .map_err(|_| anyhow::anyhow!("GitHub command could not complete"))?;
        ensure!(
            result.status.success(),
            "GitHub command failed (output withheld)"
        );
        String::from_utf8(result.stdout).context("invalid GitHub response encoding")
    }
}
pub fn cmd(api: &mut impl Github, args: &[&str]) -> Result<String> {
    api.command(&args.iter().map(|s| s.to_string()).collect::<Vec<_>>())
}
pub fn array_pages(value: Value) -> Result<Vec<Value>> {
    let pages = value.as_array().context("malformed paginated response")?;
    let mut items = vec![];
    for page in pages {
        for item in page.as_array().context("malformed page")? {
            ensure!(item.is_object(), "malformed API object");
            items.push(item.clone());
        }
    }
    Ok(items)
}
pub fn objects(api: &mut impl Github, endpoint: &str, key: &str) -> Result<Vec<Value>> {
    let value = api.api(endpoint, true)?;
    let pages = value
        .as_array()
        .filter(|p| !p.is_empty())
        .context("malformed paginated API response")?;
    let mut items = vec![];
    for page in pages {
        for item in page[key].as_array().context("malformed API collection")? {
            ensure!(item.is_object(), "malformed API object");
            items.push(item.clone());
        }
    }
    Ok(items)
}
pub fn run(command: &str, args: impl Iterator<Item = String>) -> Result<()> {
    match command {
        "verify-preview-arm-runtime" => arm::run(args),
        "apply-release-recovery" => recovery::run(args),
        "verify-public-release" => verify::run(args),
        "release-checksums" => {
            let paths: Vec<_> = args.collect();
            ensure!(
                paths.len() == 2,
                "expected artifact directory and checksum output"
            );
            verify::checksums(
                std::path::Path::new(&paths[0]),
                std::path::Path::new(&paths[1]),
            )
        }
        "release-availability" => {
            let paths: Vec<_> = args.collect();
            ensure!(paths.len() == 1, "expected notes file");
            verify::availability(std::path::Path::new(&paths[0]), |key| {
                std::env::var(key).ok()
            })
        }
        "check-release-ci" => ci::run(args),
        "preview-release" => preview::run(args),
        "reconcile-preview-release" => preview::reconcile_cli(args),
        _ => anyhow::bail!("unknown release command"),
    }
}
