mod badges;
mod branding;
mod browser_fixture;
mod browser_package;
mod compile_report;
mod detached_acceptance;
mod diagnostics;
mod fake_openai;
mod process;
mod rehearsal;
mod root_report;
mod safety_budget;
mod safety_scan;
mod score;
mod sdk_version;
mod size_budget;
mod structure;
mod ws_cache;
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
const USAGE: &str = "Usage: cargo xtask <command> [arguments]
Checks: check-branding, check-modules, check-boundaries
Budgets: check-code-size, check-test-size, check-panics, check-swallowed-errors, check-wildcards [--update]
SDK: version-sdk <runtime-tag>
Browser: browser-fixture [--port N], test-browser-handoff-live --tab-id ID, package-browser <extension-source> <output>
Fixtures: test-cloud-detached-turn --binary PATH, fake-openai-server <port> (FAKE_LOG required)
Processes: timeout <seconds> <command> [args...], log-build <log-path> [args...]
Diagnostics: analyze-root-crate [--json] [--full], compile-isolation [--json] [--strict-target-state], analyze-ws-cache [logs...] [--json], score-plans <input> <output>, find-unlocked-env-tests [directory], tb-compare <jobs> [baseline]
Rehearsal: update-rehearsal [--help]
Repository: downloads-badge [--help], star-history [--help]";
fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("--help" | "-h" | "help") => println!("{USAGE}"),
        Some("test-cloud-detached-turn") => detached_acceptance::run(args)?,
        Some("fake-openai-server") => fake_openai::run(args)?,
        Some("browser-fixture") => browser_fixture::run(args)?,
        Some("test-browser-handoff-live") => browser_fixture::live(&root()?, args)?,
        Some("analyze-root-crate") => root_report::run(&root()?, args)?,
        Some("compile-isolation") => compile_report::run(&root()?, args)?,
        Some("analyze-ws-cache") => ws_cache::run(args)?,
        Some("downloads-badge") => badges::run(false, args)?,
        Some("star-history") => badges::run(true, args)?,
        Some("find-unlocked-env-tests") => {
            let path = args
                .next()
                .map(PathBuf::from)
                .unwrap_or(root()?.join("crates/nexode-tui/src"));
            anyhow::ensure!(args.next().is_none(), "unexpected argument");
            diagnostics::env_tests(&path)?;
        }
        Some("tb-compare") => {
            let jobs = PathBuf::from(args.next().context("missing jobs directory")?);
            let baseline = args
                .next()
                .map(PathBuf::from)
                .unwrap_or(root()?.join("scripts/tb_baseline_cc_opus48.tsv"));
            anyhow::ensure!(args.next().is_none(), "unexpected argument");
            diagnostics::compare(&jobs, &baseline)?;
        }
        Some("update-rehearsal") => rehearsal::run(&root()?, args)?,
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
        _ => bail!("{USAGE}"),
    }
    Ok(())
}
