mod badges;
mod bench_audit;
mod branding;
mod browser_cold;
mod browser_fixture;
mod browser_package;
mod ci_suites;
mod compile_report;
mod demo_shop;
mod detached_acceptance;
mod diagnostics;
mod discord_release;
mod discovery;
mod fake_openai;
mod gateway;
mod handoff;
mod memory_snapshot;
mod mermaid_probe;
mod monitor;
mod niri_fuzz;
mod oauth;
mod process;
#[cfg(unix)]
mod pty;
mod rehearsal;
mod release;
mod reload_storm;
mod resume_bench;
mod root_report;
mod safety_budget;
mod safety_scan;
mod score;
mod sdk_version;
mod size_budget;
mod startup_bench;
mod structure;
mod token_compare;
mod tui_probe;
mod waybar;
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
Benchmarks: bench-reload-storm --socket PATH [-n N], benchmark-attribution [--help], bench-startup [binary] [--check], bench-resume-picker <binary> [--runs N], bench-resume-suite [--samples N]
Fixtures: demo-shop [--help], demo-session <before> <after> <directory>, mock-sponsor [--help], openrelay-discovery-server [--help], openrelay-check [endpoint|rpc|trace], test-cloud-detached-turn --binary PATH, fake-openai-server <port> (FAKE_LOG required)
Processes: timeout <seconds> <command> [args...], log-build <log-path> [args...]
Diagnostics: analyze-root-crate [--json] [--full], compile-isolation [--json] [--strict-target-state], analyze-ws-cache [logs...] [--json], score-plans <input> <output>, find-unlocked-env-tests [directory], tb-compare <jobs> [baseline]
Rehearsal: update-rehearsal [--help]
Repository: downloads-badge [--help], star-history [--help]";
fn main() -> Result<()> {
    if std::env::args_os()
        .next()
        .and_then(|s| {
            std::path::PathBuf::from(s)
                .file_stem()
                .map(|s| s.to_owned())
        })
        .is_some_and(|s| s == "nexode-demo-shop")
    {
        return demo_shop::run(std::env::args().skip(1));
    }
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("--help" | "-h" | "help") => println!("{USAGE}"),
        Some("benchmark-attribution") => discovery::attribution::run(&root()?, args)?,
        Some("openrelay-check") => discovery::openrelay_check::run(args)?,
        Some("demo-shop") => demo_shop::run(args)?,
        Some("demo-session") => demo_shop::session(args)?,
        Some(
            command @ ("check-release-ci"
            | "validate-preview-version"
            | "verify-hosted-linux-abi"
            | "preview-release"
            | "reconcile-preview-release"
            | "verify-preview-arm-runtime"
            | "apply-release-recovery"
            | "verify-public-release"
            | "release-checksums"
            | "release-availability"),
        ) => release::run(command, args)?,
        Some("package-version") => {
            use std::io::Read;
            let mut input = String::new();
            std::io::stdin().read_to_string(&mut input)?;
            let value: toml::Value = input.parse()?;
            println!(
                "{}",
                value["package"]["version"]
                    .as_str()
                    .context("package version missing")?
            );
        }
        Some(
            command @ ("native-handoff"
            | "native-handoff-preflight"
            | "signing-check"
            | "sign-release"),
        ) => handoff::run(command, args)?,
        Some(command @ ("oauth-helper" | "oauth-probe")) => oauth::run(command, args)?,
        Some("audit-terminal-bench") => {
            let code = bench_audit::run(args)?;
            std::process::exit(code);
        }
        Some("test-ci-suites") => {
            let code = ci_suites::run(&root()?, args)?;
            std::process::exit(code);
        }
        Some("waybar-status") => waybar::run(args)?,
        Some("remote-check") => gateway::run(args)?,
        Some("compare-token-usage") => {
            let code = token_compare::run(&root()?, args)?;
            std::process::exit(code);
        }
        Some("fuzz-niri-insert-point") => niri_fuzz::run(args)?,
        Some("verify-browser-cold-start") => {
            let code = browser_cold::run(args)?;
            std::process::exit(code);
        }
        Some("mermaid-fit-probe") => {
            let code = mermaid_probe::run(args)?;
            std::process::exit(code);
        }
        Some("verify-discovery-select") => discovery::select_acceptance::run(args)?,
        Some(
            command @ ("measure-key-echo"
            | "profile-real-spawn"
            | "bench-startup-visible-ready"
            | "profile-single-spawn"
            | "dump-fresh-spawn-screen"
            | "why-no-animation-area"
            | "which-overlay-blocks-donut"
            | "count-idle-draws"
            | "pin-no-area-state"
            | "measure-animation-cpu-cost"
            | "sweep-animation-fps"
            | "verify-light-theme"
            | "diagnose-idle-render-cost"
            | "probe-idle-state"
            | "profile-idle-donut"
            | "check-donut-animates-live"
            | "verify-donut-still-animates"),
        ) => {
            let code = tui_probe::run(command, args)?;
            std::process::exit(code);
        }
        Some("monitor") => monitor::run(args)?,
        Some("memory-snapshot") => memory_snapshot::run(args)?,
        Some("post-discord-release") => discord_release::run(args)?,
        Some("mock-sponsor") => discovery::sponsor::run(args)?,
        Some("openrelay-discovery-server") => discovery::openrelay::run(args)?,
        Some("bench-reload-storm") => reload_storm::run(args)?,
        Some("bench-startup") => startup_bench::run(args)?,
        Some("bench-resume-picker") => resume_bench::run_picker(args)?,
        Some("bench-resume-suite") => resume_bench::run_suite(&root()?, args)?,
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
