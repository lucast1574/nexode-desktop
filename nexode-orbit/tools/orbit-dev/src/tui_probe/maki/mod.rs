//! Isolated no-prompt resource comparison. External Maki is selected explicitly.
mod run;
use anyhow::{Context, Result, ensure};
use clap::Parser;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};
pub(super) fn median(mut v: Vec<f64>) -> Result<f64> {
    ensure!(!v.is_empty(), "no measurement samples");
    v.sort_by(f64::total_cmp);
    let n = v.len();
    Ok(if n.is_multiple_of(2) {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    } else {
        v[n / 2]
    })
}
fn binary(path: &Path) -> Result<PathBuf> {
    if path.components().count() == 1
        && !path.is_file()
        && let Some(found) = std::env::var_os("PATH").and_then(|p| {
            std::env::split_paths(&p)
                .map(|d| d.join(path))
                .find(|p| p.is_file())
        })
    {
        return Ok(found.canonicalize()?);
    }
    Ok(path.canonicalize()?)
}
fn metadata(path: &Path) -> Result<Value> {
    let version =
        crate::process::capture(Command::new(path).arg("--version"), Duration::from_secs(20))?;
    ensure!(version.status.success(), "binary --version failed");
    let mut file = fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut buf = [0; 65536];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
    }
    Ok(
        json!({"path":path,"version":String::from_utf8(version.stdout)?.trim(),"bytes":file.metadata()?.len(),"sha256":format!("{:x}",hash.finalize())}),
    )
}
fn host() -> Result<Value> {
    let mut name = std::mem::MaybeUninit::<libc::utsname>::uninit();
    ensure!(
        unsafe { libc::uname(name.as_mut_ptr()) } == 0,
        "uname failed"
    );
    let name = unsafe { name.assume_init() };
    let names = [
        &name.sysname,
        &name.nodename,
        &name.release,
        &name.version,
        &name.machine,
    ]
    .map(|field| {
        unsafe { std::ffi::CStr::from_ptr(field.as_ptr()) }
            .to_string_lossy()
            .into_owned()
    });
    let load: Vec<f64> = fs::read_to_string("/proc/loadavg")?
        .split_whitespace()
        .take(3)
        .map(str::parse)
        .collect::<std::result::Result<_, _>>()?;
    Ok(
        json!({"uname":names,"cpu_count":unsafe{libc::sysconf(libc::_SC_NPROCESSORS_ONLN)},"loadavg":load}),
    )
}
fn write(path: &Path, value: &Value) -> Result<()> {
    let parent = path.parent().context("output parent missing")?;
    fs::create_dir_all(parent)?;
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(&mut tmp, value)?;
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(())
}
#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "nexode")]
    nexode: PathBuf,
    #[arg(long)]
    maki: Option<PathBuf>,
    #[arg(long,num_args=1..,default_values_t=[String::from("nexode"),String::from("maki")])]
    tools: Vec<String>,
    #[arg(long)]
    maki_no_splash: bool,
    #[arg(long, default_value_t = 3)]
    runs: usize,
    #[arg(long,num_args=1..,default_values_t=[1usize,5,10])]
    sessions: Vec<usize>,
    #[arg(long, default_value_t = 5.0)]
    idle_seconds: f64,
    #[arg(long)]
    output: PathBuf,
}
pub(super) fn run(args: impl Iterator<Item = String>) -> Result<i32> {
    let a = Args::parse_from(std::iter::once("bench-nexode-maki".into()).chain(args));
    ensure!(
        a.runs > 0
            && a.runs <= 100
            && a.sessions.iter().all(|n| *n > 0 && *n <= 100)
            && a.idle_seconds.is_finite()
            && a.idle_seconds > 0.0
            && a.idle_seconds <= 3600.0,
        "invalid resource benchmark bounds"
    );
    let output = std::env::current_dir()?.join(a.output);
    fs::create_dir_all(output.parent().unwrap())?;
    let mut binaries = BTreeMap::new();
    for tool in &a.tools {
        let path = match tool.as_str() {
            "nexode" => &a.nexode,
            "maki" => a
                .maki
                .as_ref()
                .context("--maki required when Maki is selected")?,
            _ => anyhow::bail!("unknown benchmark tool {tool}"),
        };
        binaries.insert(tool.clone(), binary(path)?);
    }
    let mut result = json!({"method":"Fresh processes and isolated returning-user HOME/workspace, dummy API key, no prompts submitted. 120x40 native PTY/vt100. Dedicated Nexode server included; memory subsystem disabled. CPU percent is one logical core at /proc tick resolution. PSS includes sharing with unrelated processes; private memory also reported. Startup includes Rust spawn/emulation. Deterministic Rust shuffle differs from the former Python seed stream.","timestamp":chrono::Local::now().to_rfc3339(),"host":host()?,"maki_no_splash":a.maki_no_splash,"binaries":{},"runs":[]});
    for (name, path) in &binaries {
        result["binaries"][name] = metadata(path)?;
    }
    let mut jobs = vec![];
    for r in 0..a.runs {
        for n in &a.sessions {
            for name in binaries.keys() {
                jobs.push((name.clone(), *n, r + 1));
            }
        }
    }
    ensure!(jobs.len() <= 10000, "too many benchmark jobs");
    let mut seed = 42u64;
    for i in (1..jobs.len()).rev() {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        jobs.swap(i, (seed as usize) % (i + 1));
    }
    for (tool, n, repetition) in jobs {
        println!("{tool}: {n} sessions, repetition {repetition}");
        let mut row = run::measure(&tool, &binaries[&tool], n, a.idle_seconds, a.maki_no_splash)?;
        row["repetition"] = json!(repetition);
        result["runs"].as_array_mut().unwrap().push(row.clone());
        write(&output, &result)?;
        for field in [
            "screen",
            "settled_screen",
            "startup",
            "input_echo_samples_ms",
        ] {
            row.as_object_mut().unwrap().remove(field);
        }
        println!("{row}");
    }
    println!("Saved {}", output.display());
    Ok(0)
}
