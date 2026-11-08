use anyhow::{Context, Result, ensure};
use clap::Parser;
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fs::{self, File},
    io::{Read, Seek, SeekFrom, Write},
    os::unix::{
        fs::{MetadataExt, PermissionsExt},
        process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Output, Stdio},
    time::{Duration, Instant},
};
const SECRET: &str = "synthetic-import-cli-not-a-real-access-token";
const REFRESH: &str = "synthetic-import-cli-not-a-real-refresh-token";
fn envelope(provider: &str) -> Vec<u8> {
    let credential = if provider == "openai" {
        json!({"access_token":SECRET,"refresh_token":REFRESH,"expires_at":4102444800000u64})
    } else {
        json!({"access":SECRET,"refresh":REFRESH,"expires":4102444800000u64})
    };
    serde_json::to_vec(&json!({"version":1,"provider":provider,"credential":credential})).unwrap()
}
struct Running {
    child: Child,
    stdin: Option<ChildStdin>,
    out: File,
    err: File,
    done: bool,
}
impl Running {
    fn new(mut command: Command, stdin: Stdio) -> Result<Self> {
        let out = tempfile::tempfile()?;
        let err = tempfile::tempfile()?;
        command
            .stdin(stdin)
            .stdout(out.try_clone()?)
            .stderr(err.try_clone()?)
            .process_group(0);
        let mut child = command.spawn()?;
        let stdin = child.stdin.take();
        Ok(Self {
            child,
            stdin,
            out,
            err,
            done: false,
        })
    }
    fn send(&mut self, payload: &[u8]) -> Result<()> {
        let input = self.stdin.as_mut().context("stdin not piped")?;
        input.write_all(payload)?;
        input.flush()?;
        Ok(())
    }
    fn finish_input(&mut self) {
        self.stdin.take();
    }
    fn wait(&mut self, seconds: u64) -> Result<Output> {
        let deadline = Instant::now() + Duration::from_secs(seconds);
        let status = loop {
            if let Some(status) = self.child.try_wait()? {
                self.done = true;
                break status;
            }
            ensure!(Instant::now() < deadline, "CLI exceeded {seconds}s timeout");
            std::thread::sleep(Duration::from_millis(10));
        };
        let mut stdout = vec![];
        let mut stderr = vec![];
        for (file, bytes) in [(&mut self.out, &mut stdout), (&mut self.err, &mut stderr)] {
            ensure!(
                file.metadata()?.len() <= 1024 * 1024,
                "CLI output unexpectedly large"
            );
            file.seek(SeekFrom::Start(0))?;
            file.read_to_end(bytes)?;
        }
        for secret in [SECRET, REFRESH] {
            ensure!(
                !stdout.windows(secret.len()).any(|w| w == secret.as_bytes())
                    && !stderr.windows(secret.len()).any(|w| w == secret.as_bytes()),
                "synthetic secret exposed by CLI"
            );
        }
        Ok(Output {
            status,
            stdout,
            stderr,
        })
    }
}
impl Drop for Running {
    fn drop(&mut self) {
        if !self.done {
            unsafe {
                libc::kill(-(self.child.id() as i32), libc::SIGKILL);
            }
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}
struct Fixture {
    root: tempfile::TempDir,
    binary: PathBuf,
    home: PathBuf,
}
impl Fixture {
    fn new(binary: &Path) -> Result<Self> {
        let root = tempfile::Builder::new()
            .prefix("orbit-import-cli-")
            .tempdir()?;
        let home = root.path().join("data");
        Ok(Self {
            root,
            binary: binary.into(),
            home,
        })
    }
    fn command(&self, args: &[&str]) -> Command {
        let mut c = Command::new(&self.binary);
        c.env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("HOME", self.root.path())
            .env("NEXODE_HOME", &self.home)
            .env("XDG_CONFIG_HOME", self.root.path().join("config"))
            .env("NEXODE_TELEMETRY", "off")
            .env("HTTPS_PROXY", "http://127.0.0.1:9")
            .env("HTTP_PROXY", "http://127.0.0.1:9")
            .env("ALL_PROXY", "http://127.0.0.1:9")
            .env("NO_PROXY", "")
            .args(["--no-update", "--no-selfdev"])
            .args(args);
        c
    }
    fn import(&self, provider: &str) -> Command {
        self.command(&[
            "auth",
            "import",
            "--provider",
            provider,
            "--stdin",
            "--json",
        ])
    }
    fn payload(&self, payload: &[u8], provider: &str) -> Result<Output> {
        let mut r = Running::new(self.import(provider), Stdio::piped())?;
        r.send(payload)?;
        r.finish_input();
        r.wait(10)
    }
}
fn names(path: &Path) -> Result<BTreeSet<String>> {
    fs::read_dir(path)?
        .map(|e| Ok(e?.file_name().to_string_lossy().into_owned()))
        .collect()
}
fn mode(path: &Path) -> Result<u32> {
    Ok(fs::metadata(path)?.permissions().mode() & 0o777)
}
fn fingerprint(path: &Path) -> Result<(u64, u32, i64, i64)> {
    let m = fs::metadata(path)?;
    Ok((m.ino(), m.mode(), m.mtime(), m.mtime_nsec()))
}
fn both(f: &Fixture) -> Result<()> {
    for (provider, name) in [("openai", "openai-auth.json"), ("claude", "auth.json")] {
        let out = f.payload(&envelope(provider), provider)?;
        ensure!(out.status.success(), "provider import failed");
        let value: Value = serde_json::from_slice(&out.stdout)?;
        ensure!(
            value == json!({"status":"imported","provider":provider}),
            "import acknowledgement changed"
        );
        let path = f.home.join(name);
        ensure!(
            fs::read_to_string(&path)?.contains(SECRET) && mode(&path)? == 0o600,
            "private credential store missing"
        );
    }
    ensure!(mode(&f.home)? == 0o700, "home is not private");
    ensure!(
        names(&f.home)? == BTreeSet::from(["auth.json".into(), "openai-auth.json".into()]),
        "unexpected files after import"
    );
    Ok(())
}
fn existing(f: &Fixture) -> Result<()> {
    fs::create_dir(&f.home)?;
    fs::set_permissions(&f.home, fs::Permissions::from_mode(0o755))?;
    let path = f.home.join("openai-auth.json");
    fs::write(&path, b"{malformed-existing-preserve-me")?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640))?;
    let before = fingerprint(&path)?;
    let out = f.payload(&envelope("openai"), "openai")?;
    ensure!(
        !out.status.success() && serde_json::from_slice::<Value>(&out.stdout)?["status"] == "error",
        "existing store import unexpectedly accepted"
    );
    ensure!(
        fs::read(&path)? == b"{malformed-existing-preserve-me"
            && fingerprint(&path)? == before
            && mode(&f.home)? == 0o755,
        "existing store was mutated"
    );
    ensure!(
        names(&f.home)? == BTreeSet::from(["openai-auth.json".into()]),
        "extra files created"
    );
    Ok(())
}
fn invalid(f: &Fixture) -> Result<()> {
    for payload in [
        vec![],
        format!("not json {SECRET}").into_bytes(),
        vec![b'x'; 65537],
        envelope("claude"),
    ] {
        let out = f.payload(&payload, "openai")?;
        ensure!(
            !out.status.success() && !f.home.exists(),
            "invalid payload accepted or created storage"
        );
    }
    Ok(())
}
fn unsupported(f: &Fixture) -> Result<()> {
    for provider in ["auto", "openai-api", "bedrock"] {
        let mut r = Running::new(f.import(provider), Stdio::piped())?;
        let out = r.wait(5)?;
        ensure!(
            !out.status.success() && !f.home.exists(),
            "unsupported provider accepted/read stdin"
        );
    }
    Ok(())
}
fn incremental(f: &Fixture) -> Result<()> {
    let payload = envelope("openai");
    let mut r = Running::new(f.import("openai"), Stdio::piped())?;
    for chunk in [&payload[..20], &payload[20..]] {
        r.send(chunk)?;
        std::thread::sleep(Duration::from_millis(30));
    }
    ensure!(r.child.try_wait()?.is_none(), "import did not wait for EOF");
    r.finish_input();
    let out = r.wait(10)?;
    ensure!(
        out.status.success()
            && serde_json::from_slice::<Value>(&out.stdout)?["status"] == "imported",
        "incremental import failed"
    );
    Ok(())
}
fn concurrent(f: &Fixture) -> Result<()> {
    let mut jobs = vec![];
    for _ in 0..2 {
        jobs.push(Running::new(f.import("openai"), Stdio::piped())?);
    }
    for r in &mut jobs {
        r.send(&envelope("openai"))?;
        r.finish_input();
    }
    let mut winners = 0;
    for r in &mut jobs {
        winners += usize::from(r.wait(10)?.status.success());
    }
    ensure!(
        winners == 1,
        "concurrent import must have exactly one winner"
    );
    ensure!(
        names(&f.home)? == BTreeSet::from(["openai-auth.json".into()]),
        "unexpected concurrent import artifacts"
    );
    Ok(())
}
fn terminal(f: &Fixture) -> Result<()> {
    use std::os::fd::FromRawFd;
    let (mut master, mut slave) = (-1, -1);
    ensure!(
        unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
            )
        } == 0,
        "openpty failed"
    );
    let _master = unsafe { File::from_raw_fd(master) };
    let mut r = Running::new(
        f.import("openai"),
        Stdio::from(unsafe { File::from_raw_fd(slave) }),
    )?;
    let out = r.wait(5)?;
    ensure!(
        !out.status.success()
            && String::from_utf8_lossy(&out.stdout).contains("piped stdin")
            && !f.home.exists(),
        "terminal input not rejected before storage"
    );
    Ok(())
}
fn arguments(f: &Fixture) -> Result<()> {
    fs::create_dir(&f.home)?;
    fs::set_permissions(&f.home, fs::Permissions::from_mode(0o755))?;
    let path = f.home.join("openai-auth.json");
    fs::write(&path, b"existing-store-not-to-be-touched")?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644))?;
    let before = fingerprint(&path)?;
    for args in [
        vec!["auth", "import", "--provider", "openai", "--json"],
        vec!["auth", "import", "--provider", "nonsense", "--stdin"],
        vec!["auth", "import", "--help"],
        vec!["auth", "import", "--stdin", "--overwrite"],
    ] {
        let out = Running::new(f.command(&args), Stdio::null())?.wait(5)?;
        ensure!(
            out.status.success() == args.contains(&"--help"),
            "unexpected argument validation result"
        );
        ensure!(
            fs::read(&path)? == b"existing-store-not-to-be-touched"
                && fingerprint(&path)? == before,
            "argument handling mutated credentials"
        );
        ensure!(
            names(f.root.path())? == BTreeSet::from(["data".into()])
                && names(&f.home)? == BTreeSet::from(["openai-auth.json".into()]),
            "argument handling created files"
        );
    }
    Ok(())
}
fn stalled(f: &Fixture) -> Result<()> {
    let mut r = Running::new(f.import("openai"), Stdio::piped())?;
    let out = r.wait(35)?;
    ensure!(
        !out.status.success()
            && String::from_utf8_lossy(&out.stdout).contains("timed out")
            && !f.home.exists(),
        "stalled writer was not rejected without storage"
    );
    Ok(())
}
#[derive(Parser)]
struct Args {
    binary: PathBuf,
}
pub(super) fn run(args: impl Iterator<Item = String>) -> Result<()> {
    let a = Args::parse_from(std::iter::once("verify-auth-import".into()).chain(args));
    let binary = a.binary.canonicalize()?;
    type Case = (&'static str, fn(&Fixture) -> Result<()>);
    let cases: [Case; 9] = [
        ("private stores for both providers", both),
        ("existing bytes/inode/mode preserved", existing),
        ("invalid/oversized/mismatched payloads", invalid),
        ("unsupported provider rejected before input", unsupported),
        ("incremental input waits for EOF", incremental),
        ("concurrent imports have one winner", concurrent),
        ("terminal input rejected", terminal),
        ("help/invalid args preserve stores", arguments),
        ("stalled input exits without shutdown hang", stalled),
    ];
    let mut failed = 0;
    for (name, case) in cases {
        let result = Fixture::new(&binary).and_then(|fixture| case(&fixture));
        match result {
            Ok(()) => println!("PASS: {name}"),
            Err(e) => {
                failed += 1;
                eprintln!("FAIL: {name}: {e}");
            }
        }
    }
    ensure!(failed == 0, "{failed} auth import cases failed");
    println!("PASS: 9 isolated auth import cases; only synthetic credentials used");
    Ok(())
}
