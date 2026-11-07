//! Public MCP CLI replay using isolated OAuth, Streamable HTTP and legacy SSE fixtures.
use anyhow::{Context, Result, ensure};
use clap::Parser;
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
mod cases;
mod fixture;
mod replay;
use fixture::Fixture;
use replay::Replay;
#[derive(Parser)]
struct Args {
    #[arg(long)]
    binary: PathBuf,
    #[arg(long)]
    output: PathBuf,
    #[arg(long)]
    scratch_root: Option<PathBuf>,
}
pub fn run(args: impl Iterator<Item = String>) -> Result<i32> {
    use std::os::unix::fs::PermissionsExt;
    let args = Args::try_parse_from(std::iter::once("mcp-cli-acceptance".into()).chain(args))?;
    let binary = args.binary.canonicalize()?;
    ensure!(
        binary.is_file() && fs::metadata(&binary)?.permissions().mode() & 0o111 != 0,
        "binary is not executable"
    );
    let scratch = args
        .scratch_root
        .or_else(|| std::env::var_os("NEXODE_SCRATCH_DIR").map(PathBuf::from))
        .or_else(|| dirs::home_dir().map(|p| p.join(".nexode/scratch")))
        .context("scratch root required")?;
    fs::create_dir_all(&scratch)?;
    let scratch = scratch.canonicalize()?;
    let output = if args.output.is_absolute() {
        args.output
    } else {
        std::env::current_dir()?.join(args.output)
    };
    ensure!(
        output.starts_with(&scratch)
            && !output
                .components()
                .any(|c| c == std::path::Component::ParentDir),
        "output must stay under scratch root"
    );
    let parent = output.parent().context("output parent missing")?;
    fs::create_dir_all(parent)?;
    ensure!(
        parent.canonicalize()?.starts_with(&scratch)
            && !output
                .symlink_metadata()
                .is_ok_and(|m| m.file_type().is_symlink()),
        "output escapes scratch root"
    );
    let fixture = Fixture::new()?;
    let root = tempfile::Builder::new()
        .prefix("mcp-acceptance-")
        .tempdir_in(scratch)?;
    let replay = Replay::new(&binary, root.path(), &fixture)?;
    let reports = cases::run(&replay);
    let errors = fixture.errors();
    let passed = reports.iter().all(|r| r["pass"] == true) && errors.is_empty();
    let failed = reports
        .iter()
        .filter(|r| r["pass"] != true)
        .map(|r| r["name"].clone())
        .collect::<Vec<_>>();
    let report = json!({"binary":binary,"cases":reports,"http_observations":fixture.events(),"fixture_errors":errors,"pass":passed});
    let data = serde_json::to_string_pretty(&report)?;
    ensure!(
        !data.contains(&fixture.token) && !data.contains(&fixture.static_token),
        "report contains a fixture credential"
    );
    fs::write(&output, format!("{data}\n"))?;
    println!(
        "{}",
        json!({"pass":passed,"cases":report["cases"].as_array().unwrap().len(),"failed":failed,"output":output})
    );
    Ok(i32::from(!passed))
}
