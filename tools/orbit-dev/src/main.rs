mod branding;
mod browser_package;
mod process;
mod safety_budget;
mod safety_scan;
mod score;
mod sdk_version;
mod size_budget;
mod structure;
use anyhow::{Context, Result, bail};
use std::path::PathBuf;
fn root() -> Result<PathBuf> {
    Ok(PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .context("tool parent")?
        .parent()
        .context("repository parent")?
        .to_owned())
}
fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some(command @ ("check-panics" | "check-swallowed-errors" | "check-wildcards")) => {
            let mode = args.next();
            anyhow::ensure!(
                mode.as_deref()
                    .is_none_or(|s| s == "--update" || s == "--dump"),
                "expected --update or --dump"
            );
            anyhow::ensure!(args.next().is_none(), "unexpected arguments");
            let kind = match command {
                "check-panics" => safety_budget::Kind::Panic,
                "check-swallowed-errors" => safety_budget::Kind::Swallowed,
                _ => safety_budget::Kind::Wildcard,
            };
            safety_budget::run(
                &root()?,
                kind,
                mode.as_deref() == Some("--update"),
                mode.as_deref() == Some("--dump"),
            )?;
        }
        Some(command @ ("check-code-size" | "check-test-size")) => {
            let update = match args.next().as_deref() {
                None => false,
                Some("--update") => true,
                _ => bail!("expected --update or no arguments"),
            };
            size_budget::run(&root()?, command == "check-test-size", update)?;
        }
        Some("check-modules") => structure::modules(&root()?)?,
        Some("check-boundaries") => structure::boundaries(&root()?)?,
        Some("score-plans") => {
            let input = PathBuf::from(args.next().context("missing input JSON path")?);
            let output = PathBuf::from(args.next().context("missing output JSON path")?);
            score::run(&input, &output)?;
        }
        Some("timeout") => {
            let seconds = args.next().context("missing timeout seconds")?.parse()?;
            std::process::exit(process::run_with_timeout(seconds, args.collect())?);
        }
        Some("log-build") => {
            let path = PathBuf::from(args.next().context("missing log path")?);
            process::log_build(&path, args.collect())?;
        }
        Some("package-browser") => {
            let source = PathBuf::from(
                args.next()
                    .context("missing bridge extension source directory")?,
            );
            let output = PathBuf::from(args.next().context("missing output directory")?);
            browser_package::package(&source, &output)?;
        }
        Some("check-branding") => {
            branding::check(&root()?)?;
        }
        Some("version-sdk") => {
            let tag = args
                .next()
                .context("usage: cargo xtask version-sdk <runtime-tag>")?;
            let result = sdk_version::version_packages(&root()?, &tag)?;
            println!("tag={}\nversion={}", result.tag, result.version);
        }
        _ => bail!(
            "usage: cargo xtask <check-branding | version-sdk <runtime-tag> | package-browser <extension-source> <output>>"
        ),
    }
    Ok(())
}
