use anyhow::{Context, Result, ensure};
use clap::Parser;
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant, SystemTime},
};
#[derive(Parser)]
struct Args {
    #[arg(long)]
    skip_build: bool,
    #[arg(long)]
    timing_html: Option<PathBuf>,
    #[arg(long)]
    touch: Option<PathBuf>,
    #[arg(long, default_value = "selfdev")]
    profile: String,
    #[arg(long, short = 'p', default_value = "nexode")]
    package: String,
    #[arg(long, default_value = "nexode")]
    bin: String,
    #[arg(long, default_value = "default")]
    feature_profile: String,
    #[arg(long)]
    json: Option<PathBuf>,
    #[arg(long,default_value="12",value_parser=clap::value_parser!(u32).range(1..))]
    top: u32,
}
fn number(v: &Value, key: &str) -> f64 {
    v[key].as_f64().unwrap_or(0.)
}
fn rounded(n: f64) -> f64 {
    (n * 1000.).round() / 1000.
}
fn section(v: &Value, key: &str) -> Option<f64> {
    v["sections"]
        .as_array()?
        .iter()
        .find(|s| s[0] == key)
        .map(|s| number(&s[1], "end") - number(&s[1], "start"))
}
fn unit(v: &Value, stack: bool) -> Value {
    let mut out = json!({"name":v["name"],"target":v["target"].as_str().unwrap_or(""),"features":v["features"].as_array().cloned().unwrap_or_default(),"start_seconds":rounded(number(v,"start")),"duration_seconds":rounded(number(v,"duration")),"frontend_seconds":section(v,"frontend").map(rounded),"codegen_seconds":section(v,"codegen").map(rounded)});
    if stack {
        out["end_seconds"] = json!(rounded(number(v, "start") + number(v, "duration")));
    } else {
        out["version"] = v["version"].clone();
    }
    out
}
fn summarize(text: &str, args: &Args, path: &Path, elapsed: Option<f64>) -> Result<Value> {
    let (_, tail) = text
        .split_once("const UNIT_DATA = ")
        .context("timing HTML does not contain Cargo UNIT_DATA")?;
    ensure!(
        tail.trim_start().starts_with('['),
        "Cargo UNIT_DATA did not start with '['"
    );
    let units = serde_json::Deserializer::from_str(tail)
        .into_iter::<Vec<Value>>()
        .next()
        .context("missing UNIT_DATA")??;
    let regex = regex::Regex::new(r"(?:const\s+)?DURATION\s*=\s*([0-9]+(?:\.[0-9]+)?)\s*;")?;
    let duration = regex.captures(text).and_then(|c| c[1].parse::<f64>().ok());
    let mut top: Vec<_> = units.iter().collect();
    top.sort_by(|a, b| number(b, "duration").total_cmp(&number(a, "duration")));
    let mut stack: Vec<_> = units
        .iter()
        .filter(|v| {
            let name = v["name"].as_str().unwrap_or("");
            let target = v["target"].as_str().unwrap_or("");
            (matches!(name, "nexode-base" | "nexode-app-core" | "nexode-tui")
                && !target.contains("build script"))
                || (name == "nexode"
                    && (target.is_empty() || target.contains(&format!("bin \"{}\"", args.bin))))
        })
        .collect();
    stack.sort_by(|a, b| number(a, "start").total_cmp(&number(b, "start")));
    let span = stack.first().map(|first| {
        rounded(
            stack
                .iter()
                .map(|v| number(v, "start") + number(v, "duration"))
                .fold(f64::NEG_INFINITY, f64::max)
                - number(first, "start"),
        )
    });
    let sum = |key: &str| {
        rounded(
            stack
                .iter()
                .map(|v| {
                    if key == "duration" {
                        number(v, key)
                    } else {
                        section(v, key).unwrap_or(0.)
                    }
                })
                .sum(),
        )
    };
    Ok(
        json!({"timing_html":path,"profile":args.profile,"package":args.package,"bin":args.bin,"feature_profile":args.feature_profile,"wall_seconds_from_cargo_timing":duration,"wall_seconds_measured_by_probe":elapsed,"unit_count":units.len(),"top_units":top.into_iter().take(args.top as usize).map(|v|unit(v,false)).collect::<Vec<_>>(),"nexode_serial_stack":{"span_seconds":span,"sum_unit_seconds":sum("duration"),"sum_frontend_seconds":sum("frontend"),"sum_codegen_seconds":sum("codegen"),"units":stack.iter().map(|v|unit(v,true)).collect::<Vec<_>>()}}),
    )
}
pub fn run(root: &Path, args: impl Iterator<Item = String>) -> Result<()> {
    let args = Args::parse_from(std::iter::once("compile-time-probe".into()).chain(args));
    #[cfg(unix)]
    let _signals = crate::ci_suites::unix::Signals::install()?;
    let resolve = |path: &Path| {
        if path.is_absolute() {
            path.to_owned()
        } else {
            root.join(path)
        }
    };
    if let Some(path) = &args.touch {
        ensure!(
            resolve(path).exists(),
            "touch path does not exist: {}",
            path.display()
        );
    }
    let elapsed = if args.skip_build {
        None
    } else {
        if let Some(path) = &args.touch {
            fs::File::options()
                .write(true)
                .open(resolve(path))?
                .set_modified(SystemTime::now())?;
        }
        let start = Instant::now();
        let result = crate::process::capture_interruptible(
            Command::new("bash")
                .arg(root.join("scripts/dev_cargo.sh"))
                .args([
                    "build",
                    "--profile",
                    &args.profile,
                    "-p",
                    &args.package,
                    "--bin",
                    &args.bin,
                    "--timings",
                ])
                .env("NEXODE_DEV_FEATURE_PROFILE", &args.feature_profile)
                .current_dir(root),
            Duration::from_secs(10800),
            || {
                #[cfg(unix)]
                {
                    crate::ci_suites::unix::interrupted().is_some()
                }
                #[cfg(not(unix))]
                {
                    false
                }
            },
        )?;
        print!("{}", String::from_utf8_lossy(&result.stdout));
        eprint!("{}", String::from_utf8_lossy(&result.stderr));
        ensure!(
            result.status.success(),
            "timed build failed: {}",
            result.status
        );
        Some(rounded(start.elapsed().as_secs_f64()))
    };
    let path = if let Some(path) = &args.timing_html {
        resolve(path)
    } else {
        let mut candidates = Vec::new();
        for entry in fs::read_dir(root.join("target/cargo-timings"))
            .context("no Cargo timings; build first or pass --timing-html")?
        {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("cargo-timing")
                && name.ends_with(".html")
                && entry.file_type()?.is_file()
            {
                candidates.push((entry.metadata()?.modified()?, entry.path()));
            }
        }
        candidates.sort();
        candidates.pop().context("no Cargo timing HTML found")?.1
    };
    let summary = summarize(
        &String::from_utf8_lossy(&fs::read(&path)?),
        &args,
        &path,
        elapsed,
    )?;
    if let Some(path) = &args.json {
        let path = resolve(path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(
            path,
            format!("{}\n", serde_json::to_string_pretty(&summary)?),
        )?;
    }
    println!(
        "compile_time_probe summary\n  timing html: {}\n  cargo timing wall: {}\n  measured wall: {}\n  units: {}\n  nexode serial stack: {}",
        path.display(),
        seconds(&summary["wall_seconds_from_cargo_timing"]),
        seconds(&summary["wall_seconds_measured_by_probe"]),
        summary["unit_count"],
        summary["nexode_serial_stack"]
    );
    for unit in summary["top_units"]
        .as_array()
        .context("top units missing")?
    {
        println!(
            "  - {} {}: {} (frontend {}, codegen {})",
            unit["name"].as_str().unwrap_or(""),
            unit["target"]
                .as_str()
                .filter(|s| !s.is_empty())
                .unwrap_or("lib"),
            seconds(&unit["duration_seconds"]),
            seconds(&unit["frontend_seconds"]),
            seconds(&unit["codegen_seconds"])
        );
    }
    Ok(())
}
fn seconds(v: &Value) -> String {
    v.as_f64()
        .map(|n| format!("{n:.2}s"))
        .unwrap_or("n/a".into())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cargo_js_nested_sections_quotes_and_parallel_stack() {
        let args = Args::parse_from(["probe", "--skip-build", "--top", "2"]);
        let html = r#"const DURATION = 14.25; const UNIT_DATA = [{"name":"nexode-base","target":"","start":1,"duration":10,"sections":[["frontend",{"start":1,"end":7}],["codegen",{"start":7,"end":11}]]},{"name":"nexode","target":"bin \"nexode\"","start":8,"duration":4},{"name":"nexode-tui","target":"build script ]","start":0,"duration":12}]; other_js();"#;
        let report = summarize(html, &args, Path::new("timing.html"), None).unwrap();
        assert_eq!(report["unit_count"], 3);
        assert_eq!(report["wall_seconds_from_cargo_timing"], 14.25);
        assert_eq!(report["top_units"][0]["duration_seconds"], 12.0);
        assert_eq!(report["nexode_serial_stack"]["span_seconds"], 11.0);
        assert_eq!(report["nexode_serial_stack"]["sum_unit_seconds"], 14.0);
        assert_eq!(report["nexode_serial_stack"]["sum_frontend_seconds"], 6.0);
        assert!(report["top_units"][0]["frontend_seconds"].is_null());
        assert!(summarize("const UNIT_DATA = [broken", &args, Path::new("bad"), None).is_err());
    }
}
