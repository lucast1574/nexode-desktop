mod branding;
mod browser_package;
mod process;
mod sdk_version;
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
