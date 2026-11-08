//! Compare live jemalloc heaps around isolated session churn.
use super::{
    idle_rss::{memory, subscribe},
    isolated::pause,
    single::{Server, wait_socket},
};
use anyhow::{Context, Result, ensure};
use clap::Parser;
use std::{
    collections::BTreeSet,
    fs,
    os::unix::{fs::PermissionsExt, process::CommandExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};

#[derive(Parser)]
struct Args {
    #[arg(long)]
    binary: PathBuf,
    #[arg(long, default_value_t = 30)]
    sessions: usize,
    #[arg(long, default_value_t = 4)]
    rounds: usize,
    #[arg(long, default_value_t = 40.0)]
    settle: f64,
    #[arg(long)]
    abrupt: bool,
    #[arg(long)]
    realistic: bool,
    /// Elevate only the debugger attaching to the owned server.
    #[arg(long)]
    sudo: bool,
    #[arg(long, default_value = "gdb")]
    gdb: PathBuf,
    #[arg(long, default_value = "jeprof")]
    jeprof: PathBuf,
}
fn heaps(dir: &Path) -> Result<BTreeSet<PathBuf>> {
    let mut paths = BTreeSet::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if entry.file_type()?.is_file() && entry.path().extension().is_some_and(|s| s == "heap") {
            paths.insert(entry.path());
        }
    }
    Ok(paths)
}
fn capture(command: &mut Command) -> Result<std::process::Output> {
    crate::process::capture_interruptible(command, Duration::from_secs(30), || {
        crate::ci_suites::unix::interrupted().is_some()
    })
}
fn dump(a: &Args, pid: u32, dir: &Path, name: &str) -> Result<PathBuf> {
    let before = heaps(dir)?;
    let mut c = if a.sudo {
        let mut c = Command::new("sudo");
        c.arg("-n").arg(&a.gdb);
        c
    } else {
        Command::new(&a.gdb)
    };
    c.args([
        "-p",
        &pid.to_string(),
        "-batch",
        "-ex",
        "call (int)mallctl(\"prof.dump\", 0, 0, 0, 0)",
    ]);
    let out = capture(&mut c)?;
    fs::write(
        dir.join(format!("{name}-gdb.log")),
        [out.stdout, out.stderr].concat(),
    )?;
    ensure!(
        out.status.success(),
        "heap debugger failed; see {name}-gdb.log"
    );
    let after = heaps(dir)?;
    let new: Vec<_> = after.difference(&before).collect();
    ensure!(
        new.len() == 1,
        "expected one new heap dump, got {}",
        new.len()
    );
    let path = dir.join(format!("{name}.heap"));
    fs::rename(new[0], &path)?;
    Ok(path)
}
pub(super) fn run(args: impl Iterator<Item = String>) -> Result<i32> {
    let a = Args::parse_from(std::iter::once("heap-diff-idle-sessions".into()).chain(args));
    ensure!(
        (1..=1000).contains(&a.sessions)
            && (1..=1000).contains(&a.rounds)
            && a.settle.is_finite()
            && (0.0..=3600.0).contains(&a.settle),
        "invalid heap probe bounds"
    );
    let binary = a
        .binary
        .canonicalize()
        .context("resolve profiling binary")?;
    let root = tempfile::Builder::new()
        .prefix("orbit-heapdiff-")
        .tempdir()?
        .keep();
    eprintln!("artifacts: {}", root.display());
    let home = root.join("home");
    let runtime = root.join("run");
    let work = root.join("work");
    let prof = root.join("prof");
    for dir in [&home, &runtime, &work, &prof] {
        fs::create_dir(dir)?;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    }
    let socket = runtime.join("t.sock");
    let mut c = Command::new(&binary);
    c.env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", &home)
        .env("NEXODE_HOME", &home)
        .env("NEXODE_RUNTIME_DIR", &runtime)
        .env("NEXODE_TEMP_SERVER", "1")
        .env("NEXODE_SERVER_OWNER_PID", std::process::id().to_string())
        .env("NEXODE_NO_TELEMETRY", "1")
        .env("NEXODE_DEBUG_CONTROL", "1")
        .env("ANTHROPIC_API_KEY", "sk-ant-local-heap-fixture")
        .env(
            "MALLOC_CONF",
            format!(
                "prof_active:true,lg_prof_sample:15,prof_prefix:{}",
                prof.join("h").display()
            ),
        )
        .current_dir(&work)
        .args(["--no-update", "--no-selfdev", "serve", "--socket"])
        .arg(&socket)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(fs::File::create(root.join("server.log"))?)
        .process_group(0);
    let mut server = Server(c.spawn()?);
    wait_socket(&socket, &mut server, Duration::from_secs(30))?;
    let churn = || -> Result<()> {
        let mut connections = Vec::new();
        for _ in 0..a.sessions {
            connections.push(subscribe(&socket, &work, a.abrupt, a.realistic)?);
        }
        drop(connections);
        pause(a.settle)
    };
    churn()?;
    let first = dump(&a, server.0.id(), &prof, "A")?;
    println!("A {}", serde_json::to_string(&memory(server.0.id())?)?);
    for _ in 0..a.rounds {
        churn()?;
    }
    let last = dump(&a, server.0.id(), &prof, "B")?;
    println!("B {}", serde_json::to_string(&memory(server.0.id())?)?);
    for (option, filename) in [("--lines", "diff.txt"), ("--cum", "diff_cum.txt")] {
        let out = capture(
            Command::new(&a.jeprof)
                .args(["--text", option])
                .arg(format!("--base={}", first.display()))
                .arg(&binary)
                .arg(&last),
        )?;
        fs::write(
            root.join(filename),
            [&out.stdout[..], &out.stderr[..]].concat(),
        )?;
        ensure!(out.status.success(), "jeprof failed; see {filename}");
        if option == "--lines" {
            for line in String::from_utf8_lossy(&out.stdout).lines().take(45) {
                println!("{line}");
            }
        }
    }
    Ok(0)
}
