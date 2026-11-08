//! Credential handoff contracts using synthetic accounts and local pipes only.
mod cases;
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Seek},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Output},
    time::Duration,
};
const ACCESS: &str = "synthetic-cloud-access-not-a-real-secret";
const REFRESH: &str = "synthetic-cloud-refresh-not-a-real-secret";
const ROUTER: &str = "synthetic-router-api-key-not-a-real-secret";
const REFRESHED: &str = "synthetic-refreshed-refresh";
const SID: &str = "cloud-auth-test";
const SESSION: &str = "sessions/cloud-auth-test.json";
const LEASE: &str = "session_leases/cloud-auth-test.json";
fn read(path: impl AsRef<Path>) -> Result<Value> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}
fn write(path: impl AsRef<Path>, value: &Value) -> Result<()> {
    fs::write(path, serde_json::to_vec(value)?)?;
    Ok(())
}
fn private(path: &Path, mode: u32) -> Result<()> {
    fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    Ok(())
}
fn mode(path: &Path) -> Result<u32> {
    Ok(fs::metadata(path)?.permissions().mode() & 0o777)
}
fn quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}
struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    local: PathBuf,
    remote: PathBuf,
    work: PathBuf,
    transport: PathBuf,
    binary: PathBuf,
    filename: &'static str,
    accounts: &'static str,
    refresh: &'static str,
}
impl Fixture {
    fn new(binary: &Path) -> Result<Self> {
        let temp = tempfile::Builder::new().prefix("orbit-auth-").tempdir()?;
        let root = temp.path().to_owned();
        let local = root.join("local");
        let remote = root.join("remote");
        let work = root.join("work");
        let transport = root.join("transport");
        for path in [&local, &remote, &work] {
            fs::create_dir(path)?;
            private(path, 0o700)?;
        }
        fs::write(
            &transport,
            format!(
                "#!/bin/sh\nexport HOME={}\nexport NEXODE_HOME={}\nexec /bin/sh -c \"$1\"\n",
                quote(&root),
                quote(&remote)
            ),
        )?;
        private(&transport, 0o700)?;
        Ok(Self {
            _temp: temp,
            root,
            local,
            remote,
            work,
            transport,
            binary: binary.into(),
            filename: "openai-auth.json",
            accounts: "openai_accounts",
            refresh: "refresh_token",
        })
    }
    fn invoke_at(&self, home: &Path, args: &[&str], success: bool) -> Result<Output> {
        let out = crate::process::capture_interruptible(
            Command::new(&self.binary)
                .args(["--no-update", "--no-selfdev", "cloud"])
                .args(args)
                .current_dir(&self.work)
                .env_clear()
                .env("PATH", "/usr/bin:/bin")
                .env("HOME", &self.root)
                .env("NEXODE_HOME", home)
                .env("NEXODE_RUNTIME_DIR", &self.root)
                .env("NEXODE_NO_TELEMETRY", "1")
                .env("NEXODE_TELEMETRY", "off")
                .env("HTTPS_PROXY", "http://127.0.0.1:9")
                .env("HTTP_PROXY", "http://127.0.0.1:9"),
            Duration::from_secs(30),
            || crate::ci_suites::unix::interrupted().is_some(),
        )?;
        for secret in [
            ACCESS,
            REFRESH,
            ROUTER,
            "synthetic-refreshed-access",
            REFRESHED,
        ] {
            ensure!(
                !out.stdout
                    .windows(secret.len())
                    .any(|s| s == secret.as_bytes())
                    && !out
                        .stderr
                        .windows(secret.len())
                        .any(|s| s == secret.as_bytes()),
                "synthetic secret leaked in diagnostics"
            );
        }
        ensure!(
            out.status.success() == success,
            "unexpected status {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        Ok(out)
    }
    fn invoke(&self, args: &[&str], success: bool) -> Result<Output> {
        self.invoke_at(&self.local, args, success)
    }
    fn transfer(&self, operation: &str, extra: &[&str], success: bool) -> Result<Output> {
        let binary = self.binary.to_str().context("non-UTF8 binary")?;
        let transport = self.transport.to_str().context("non-UTF8 transport")?;
        let mut args = vec![
            operation,
            "--session",
            SID,
            "--host",
            "test-cloud",
            "--remote-binary",
            binary,
            "--transport",
            transport,
            "--json",
        ];
        args.extend_from_slice(extra);
        self.invoke(&args, success)
    }
    fn move_out(&self) -> Result<()> {
        self.transfer("move", &[], true)?;
        Ok(())
    }
    fn return_home(&self) -> Result<()> {
        self.transfer("return", &[], true)?;
        Ok(())
    }
    fn seed(&mut self, provider: &str, expiry: u64) -> Result<()> {
        let label = format!("{provider}-fox");
        let (account, active, provider_key, route, model) = if provider == "openai" {
            (
                json!({"label":label,"email":"test@example.invalid","access_token":ACCESS,"refresh_token":REFRESH,"expires_at":expiry,"account_id":"synthetic-account-id"}),
                "active_openai_account",
                "openai",
                "openai-oauth@",
                "gpt-test",
            )
        } else {
            self.filename = "auth.json";
            self.accounts = "anthropic_accounts";
            self.refresh = "refresh";
            (
                json!({"label":label,"email":"test@example.invalid","access":ACCESS,"refresh":REFRESH,"expires":expiry,"scopes":["user:inference"]}),
                "active_anthropic_account",
                "anthropic",
                "claude-oauth@",
                "claude-test",
            )
        };
        let mut store = json!({});
        store[self.accounts] = json!([account]);
        store[active] = json!(label);
        write(self.local.join(self.filename), &store)?;
        private(&self.local.join(self.filename), 0o600)?;
        fs::create_dir(self.local.join("sessions"))?;
        write(
            self.local.join(SESSION),
            &json!({"id":SID,"parent_id":null,"title":"Cloud auth test","created_at":"2026-10-20T00:00:00Z","updated_at":"2026-10-20T00:00:00Z","messages":[],"working_dir":self.work,"provider_key":provider_key,"model":model,"route_api_method":format!("{route}{label}")}),
        )
    }
    fn archives(&self) -> Result<Vec<PathBuf>> {
        fn walk(path: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
            for entry in fs::read_dir(path)? {
                let entry = entry?;
                if entry.file_type()?.is_dir() {
                    walk(&entry.path(), out)?;
                } else if entry.path().extension().is_some_and(|s| s == "tar") {
                    out.push(entry.path());
                }
            }
            Ok(())
        }
        let mut paths = vec![];
        walk(&self.remote, &mut paths)?;
        Ok(paths)
    }
    fn archives_secret_free(&self) -> Result<()> {
        for path in self.archives()? {
            let mut file = fs::File::open(path)?;
            let mut magic = [0u8; 2];
            file.read_exact(&mut magic)?;
            file.rewind()?;
            let input: Box<dyn Read> = if magic == [0x1f, 0x8b] {
                Box::new(flate2::read::GzDecoder::new(file))
            } else {
                Box::new(file)
            };
            let mut archive = tar::Archive::new(input);
            for entry in archive.entries()? {
                let mut entry = entry?;
                if entry.header().entry_type().is_file() {
                    let mut contents = vec![];
                    entry.read_to_end(&mut contents)?;
                    for secret in [ACCESS, REFRESH, ROUTER] {
                        ensure!(
                            !contents
                                .windows(secret.len())
                                .any(|s| s == secret.as_bytes()),
                            "credential leaked in session archive"
                        );
                    }
                }
            }
        }
        Ok(())
    }
    fn change_transport(&self, script: &str) -> Result<()> {
        let original = fs::read_to_string(&self.transport)?;
        fs::write(
            &self.transport,
            original.replace(
                "exec /bin/sh -c \"$1\"",
                &format!("{script}\nexec /bin/sh -c \"$1\""),
            ),
        )?;
        Ok(())
    }
    fn seed_router(&self) -> Result<PathBuf> {
        let config = self.local.join("config/nexode");
        fs::create_dir_all(&config)?;
        for (file, key) in [
            ("openrouter.env", "OPENROUTER_API_KEY"),
            ("bedrock.env", "AWS_BEARER_TOKEN_BEDROCK"),
            ("unlisted.env", "X"),
        ] {
            fs::write(config.join(file), format!("{key}={ROUTER}\n"))?;
        }
        fs::write(config.join("config.toml"), "")?;
        write(
            self.local.join("gemini_oauth.json"),
            &json!({"access_token":ROUTER,"refresh_token":ROUTER,"expires_at":1}),
        )?;
        write(
            self.local.join("account-pool.json"),
            &json!({"members":["gemini","openrouter"],"excluded":[]}),
        )?;
        let remote = self.remote.join("config/nexode");
        fs::create_dir_all(&remote)?;
        fs::write(remote.join("openrouter.env"), "OPENROUTER_API_KEY=stale\n")?;
        fs::write(remote.join("groq.env"), "GROQ_API_KEY=cloud-only\n")?;
        write(
            self.remote.join("account-pool.json"),
            &json!({"members":["cloud-only"],"excluded":["xai"]}),
        )?;
        Ok(remote)
    }
}
pub fn run(mut args: impl Iterator<Item = String>) -> Result<()> {
    let binary = PathBuf::from(
        args.next()
            .context("usage: test-cloud-credentials /absolute/path/to/nexode")?,
    )
    .canonicalize()?;
    let filter = match args.next().as_deref() {
        None => None,
        Some("--case") => Some(args.next().context("--case requires a name")?),
        _ => anyhow::bail!("expected --case NAME"),
    };
    ensure!(args.next().is_none(), "unexpected argument");
    if let Some(name) = &filter {
        ensure!(
            cases::tests().iter().any(|(case, _)| case == name),
            "unknown credential case: {name}"
        );
    }
    let _signals = crate::ci_suites::unix::Signals::install()?;
    let mut failures = vec![];
    for (name, test) in cases::tests() {
        if filter.as_deref().is_some_and(|selected| selected != name) {
            continue;
        }
        match Fixture::new(&binary).and_then(|mut f| test(&mut f)) {
            Ok(()) => println!("PASS {name}"),
            Err(e) => {
                eprintln!("FAIL {name}: {e:#}");
                failures.push(name);
            }
        }
    }
    ensure!(
        failures.is_empty(),
        "credential acceptance failed: {failures:?}"
    );
    Ok(())
}
