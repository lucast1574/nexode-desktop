//! Native remote guard/control: stdin requests, strict owned roots, no Python.
use super::*;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use sha2::{Digest, Sha256};
use std::{
    os::{
        fd::AsRawFd,
        unix::{net::UnixStream, process::CommandExt},
    },
    time::SystemTime,
};
fn bounded_stdin(limit: usize) -> Result<Vec<u8>> {
    let mut b = vec![];
    std::io::stdin()
        .take(limit as u64 + 1)
        .read_to_end(&mut b)?;
    ensure!(b.len() <= limit, "Oversized private stdin");
    Ok(b)
}
fn read_json(p: &Path) -> Result<Value> {
    serde_json::from_slice(&fs::read(p)?)
        .map_err(|_| anyhow::anyhow!("Invalid private JSON artifact"))
}
fn field<'a>(v: &'a Value, k: &str) -> Result<&'a str> {
    v[k].as_str().with_context(|| format!("missing {k}"))
}
fn audit(root: &Path, name: &str, v: &Value) -> Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = fs::OpenOptions::new()
        .append(true)
        .create(true)
        .mode(0o600)
        .open(root.join(name))?;
    serde_json::to_writer(&mut f, v)?;
    f.write_all(b"\n")?;
    Ok(())
}
fn lines(root: &Path, name: &str) -> Result<Value> {
    let p = root.join(name);
    if !p.exists() {
        return Ok(json!([]));
    }
    let b = fs::read(p)?;
    Ok(b.split(|b| *b == b'\n')
        .filter(|s| !s.is_empty())
        .map(serde_json::from_slice)
        .collect::<std::result::Result<Vec<Value>, _>>()?
        .into())
}
fn owned(root: &Path, owner: Option<&str>) -> Result<Value> {
    ensure!(
        root.is_absolute()
            && root
                .file_name()
                .is_some_and(|s| s.to_string_lossy().starts_with("run-")),
        "Invalid acceptance root"
    );
    let name = root
        .parent()
        .and_then(Path::file_name)
        .context("root parent")?
        .to_string_lossy();
    ensure!(
        ["nexode-login-acceptance", "nexode-import-acceptance"].contains(&name.as_ref()),
        "Invalid acceptance root parent"
    );
    ensure!(
        !fs::symlink_metadata(root)?.is_symlink(),
        "Symlink acceptance root"
    );
    if let Some(owner) = owner {
        ensure!(
            fs::read_to_string(root.join(".acceptance-owned"))? == owner,
            "Acceptance ownership mismatch"
        );
    }
    read_json(&root.join("guard.json"))
}
fn daemon(root: &Path) -> Result<Value> {
    let socket = root.join("runtime/server.sock");
    if !socket.exists() {
        return Ok(json!([]));
    }
    let conn = UnixStream::connect(socket)?;
    let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
    let mut size = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    ensure!(
        unsafe {
            libc::getsockopt(
                conn.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                (&mut cred as *mut libc::ucred).cast(),
                &mut size,
            )
        } == 0,
        "Read isolated peer credentials"
    );
    let p = tui::info(cred.pid as u32).context("peer identity")?;
    ensure!(
        auth::token_hashes(&p.argv).is_empty(),
        "Synthetic token in daemon argv"
    );
    Ok(
        json!([{"pid":cred.pid,"uid":cred.uid,"start_time":p.start,"server":true,"identity_source":"isolated Unix socket SO_PEERCRED"}]),
    )
}
fn command(root: &Path, guard: &Value, args: &[String]) -> Result<Command> {
    let mut c = Command::new(field(guard, "executable")?);
    c.args(args)
        .env_clear()
        .envs(env(root, true))
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .env("USER", "nexode-ssh-acceptance");
    Ok(c)
}
pub(super) fn control() -> Result<()> {
    unsafe {
        libc::umask(0o077);
    }
    let r: Value = serde_json::from_slice(&bounded_stdin(MAX_FRAME)?)
        .map_err(|_| anyhow::anyhow!("Invalid control request"))?;
    let op = field(&r, "operation")?;
    let response = if op == "create" {
        let executable = Path::new(field(&r, "executable")?);
        ensure!(executable.is_absolute(), "Absolute executable required");
        elf(executable)?;
        ensure!(
            fs::metadata(executable)?.permissions().mode() & 0o111 != 0,
            "Executable permission required"
        );
        let mode = field(&r, "mode")?;
        let cwd = field(&r, "cwd")?;
        ensure!(
            cwd.starts_with('/') && clean_path(cwd),
            "Absolute working directory required"
        );
        ensure!(
            ["login", "import"].contains(&mode),
            "Unknown remote guard mode"
        );
        let helper = std::env::current_exe()?.canonicalize()?;
        elf(&helper)?;
        // Short unique roots keep the private Unix socket below sockaddr_un limits.
        let parent = PathBuf::from(format!("/tmp/nexode-{mode}-acceptance"));
        if parent.exists() {
            ensure!(
                !fs::symlink_metadata(&parent)?.is_symlink()
                    && fs::metadata(&parent)?.uid() == unsafe { libc::getuid() },
                "Acceptance parent must be an owned real directory"
            );
        }
        mkdir(&parent)?;
        ensure!(
            fs::metadata(&parent)?.uid() == unsafe { libc::getuid() },
            "Acceptance parent belongs to another user"
        );
        let root = tempfile::Builder::new()
            .prefix("run-")
            .tempdir_in(parent)?
            .keep();
        init_root(&root)?;
        private_write(
            &root.join(".acceptance-owned"),
            field(&r, "owner")?.as_bytes(),
        )?;
        private_write(
            &root.join("guard.json"),
            &serde_json::to_vec(
                &json!({"executable":executable,"mode":mode,"cwd":field(&r,"cwd")?,"expected":r["expected"]}),
            )?,
        )?;
        let wrapper = root.join("remote-nexode");
        private_write(
            &wrapper,
            format!(
                "#!/bin/sh\nexec {} native-ssh-guard {} \"$@\"\n",
                quote(&helper.display().to_string()),
                quote(&root.display().to_string())
            )
            .as_bytes(),
        )?;
        fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o700))?;
        let mut result =
            json!({"root":root,"wrapper":wrapper,"socket":root.join("runtime/server.sock")});
        if mode == "login" {
            let pending = root.join("nexode/pending-login");
            mkdir(&pending)?;
            let legacy = json!({"expires_at_ms":SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)?.as_millis()+3600000,"login":{"provider":"openai","account_label":"acceptance-legacy","verifier":"legacy-never-use-verifier","state":"legacy-never-use-state","redirect_uri":"http://localhost:1455/auth/callback"}});
            let bytes = serde_json::to_vec(&legacy)?;
            private_write(&pending.join("openai.json"), &bytes)?;
            result["legacy_sha256"] = auth::digest(&bytes).into();
        } else {
            let other = if r["provider"] == "openai" {
                "claude"
            } else {
                ensure!(r["provider"] == "claude", "Unknown provider");
                "openai"
            };
            let (s, _) = auth::fixture(other)?;
            let name = auth::FILES.iter().find(|(p, _)| *p == other).unwrap().1;
            private_write(&root.join("nexode").join(name), &serde_json::to_vec(&s)?)?;
        }
        result
    } else {
        let root = Path::new(field(&r, "root")?);
        let guard = owned(root, Some(field(&r, "owner")?))?;
        match op {
            "cleanup_pending" => {
                ensure!(guard["mode"] == "login", "Wrong cleanup mode");
                for p in files(&root.join("nexode/pending-login"))? {
                    if p.extension().is_some_and(|e| e == "json") {
                        ensure!(
                            p.canonicalize()?.starts_with(root.canonicalize()?),
                            "Pending path escapes root"
                        );
                        fs::remove_file(p)?;
                    }
                }
                json!({"cleaned":true})
            }
            "inject_failure" => {
                ensure!(guard["mode"] == "login", "Wrong failure mode");
                private_write(
                    &root.join("fail-next-login"),
                    field(&r, "error_marker")?.as_bytes(),
                )?;
                json!({"armed":true})
            }
            "inspect" => {
                if guard["mode"] == "import" {
                    inspect_import(root, &guard)?
                } else {
                    inspect_login(root, &r)?
                }
            }
            _ => anyhow::bail!("Unknown control operation"),
        }
    };
    println!("{}", response);
    Ok(())
}
use std::os::unix::fs::MetadataExt;
fn inspect_import(root: &Path, guard: &Value) -> Result<Value> {
    let stores = auth::scan(root, &auth::allowed())?;
    let out = crate::process::capture(
        &mut command(
            root,
            guard,
            &["--no-update", "--no-selfdev", "auth", "status", "--json"].map(String::from),
        )?,
        Duration::from_secs(30),
    )?;
    ensure!(
        out.status.success()
            && auth::token_hashes(&[out.stdout.clone(), out.stderr].concat()).is_empty(),
        "Unsafe or failed auth status"
    );
    let status: Value =
        serde_json::from_slice(&out.stdout).map_err(|_| anyhow::anyhow!("Invalid auth status"))?;
    let mut states = serde_json::Map::new();
    for p in status["providers"].as_array().context("status providers")? {
        if ["openai", "claude"].iter().any(|s| p["id"] == *s) {
            ensure!(
                ["available", "expired", "not_configured"]
                    .iter()
                    .any(|s| p["status"] == *s),
                "Unknown provider state"
            );
            states.insert(field(p, "id")?.into(), p["status"].clone());
        }
    }
    ensure!(
        auth::scan(root, &auth::allowed())? == stores,
        "Status changed credential stores"
    );
    Ok(
        json!({"stores":stores,"calls":lines(root,"import-audit.jsonl")?,"starts":lines(root,"import-starts.jsonl")?,"states":states,"processes":daemon(root)?}),
    )
}
fn inspect_login(root: &Path, r: &Value) -> Result<Value> {
    let mut pending = vec![];
    for p in files(&root.join("nexode/pending-login/flows"))? {
        if p.extension().is_none_or(|ext| ext != "json") {
            continue;
        }
        let provider = p.file_stem().unwrap().to_string_lossy();
        if !["openai", "claude"].contains(&provider.as_ref()) {
            continue;
        }
        let v = read_json(&p)?;
        let login = &v["login"];
        let mut record = json!({"relative_path":p.strip_prefix(root.join("nexode"))?,"mode":fs::metadata(&p)?.permissions().mode()&0o777,"provider":provider});
        if provider == "openai" {
            record["state"] = login["state"].clone();
            record["redirect_uri"] = login["redirect_uri"].clone();
            record["challenge"] = URL_SAFE_NO_PAD
                .encode(Sha256::digest(field(login, "verifier")?.as_bytes()))
                .into();
        }
        pending.push(record);
    }
    let mut leaks = vec![];
    let mut credentials = vec![];
    let needles: Vec<_> = r["needles"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|s| !s.is_empty())
        .collect();
    for p in files(root)? {
        let rel = p.strip_prefix(root)?.display().to_string();
        if auth::CREDENTIALS.contains(&p.file_name().unwrap().to_string_lossy().as_ref()) {
            credentials.push(rel.clone());
        }
        if !needles.is_empty() {
            let b = fs::read(p)?;
            if needles
                .iter()
                .any(|s| b.windows(s.len()).any(|b| b == s.as_bytes()))
            {
                leaks.push(rel);
            }
        }
    }
    let old = root.join("nexode/pending-login/openai.json");
    let legacy = if old.exists() {
        Some(auth::digest(&fs::read(old)?))
    } else {
        None
    };
    Ok(
        json!({"pending":pending,"legacy_sha256":legacy,"calls":lines(root,"login-audit.jsonl")?,"leaks":leaks,"credentials":credentials,"processes":daemon(root)?}),
    )
}
fn option<'a>(args: &'a [String], key: &str) -> Option<&'a str> {
    args.iter()
        .position(|s| s == key)
        .and_then(|i| args.get(i + 1))
        .map(String::as_str)
}
fn transport_tail(args: &[String]) -> Result<&[String]> {
    let mut i = 0;
    while i < args.len() && args[i].starts_with("--") {
        match args[i].as_str() {
            "--no-update" | "--no-selfdev" => i += 1,
            "--cwd" | "--socket" if i + 1 < args.len() => i += 2,
            _ => anyhow::bail!("Acceptance guard refused global option"),
        }
    }
    Ok(&args[i..])
}
pub(super) fn guard(mut args: Vec<String>) -> Result<()> {
    ensure!(!args.is_empty(), "guard requires owned root");
    let root = PathBuf::from(args.remove(0));
    let g = owned(&root, None)?;
    unsafe {
        libc::umask(0o077);
    }
    let result = (|| -> Result<()> {
        ensure!(
            args.iter()
                .all(|s| auth::token_hashes(s.as_bytes()).is_empty()),
            "Synthetic token in argv refused"
        );
        let tail = transport_tail(&args)?;
        // The native SSH directory panel collects inventory before provider
        // initialization. Permit only this read-only call in the selected cwd.
        if tail.first().is_some_and(|s| s == "__remote-inventory") {
            ensure!(
                tail.len() == 3
                    && tail[1] == field(&g, "cwd")?
                    && tail[2].len() <= 256
                    && tail[2]
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'),
                "Inventory request escapes the configured workspace"
            );
            return Err(command(&root, &g, &args)?.exec().into());
        }
        if tail == ["server", "stdio"] || tail == ["auth", "status", "--json"] {
            return Err(command(&root, &g, &args)?.exec().into());
        }
        if g["mode"] == "import" {
            import_guard(&root, &g, &args, tail)
        } else {
            login_guard(&root, &g, &args, tail)
        }
    })();
    if result.is_err() {
        if g["mode"] == "import" {
            audit(
                &root,
                "import-audit.jsonl",
                &json!({"operation":"guard_refusal"}),
            )?;
        }
        eprintln!("Acceptance safety guard refused invocation; private output withheld");
        std::process::exit(93);
    }
    Ok(())
}
fn import_guard(root: &Path, g: &Value, args: &[String], tail: &[String]) -> Result<()> {
    ensure!(
        tail.len() == 6
            && tail[..3] == ["auth", "import", "--provider"]
            && tail[4..] == ["--stdin", "--json"]
            && ["openai", "claude"].contains(&tail[3].as_str()),
        "Acceptance guard refused import invocation"
    );
    let provider = &tail[3];
    audit(root, "import-starts.jsonl", &json!({"provider":provider}))?;
    let payload = bounded_stdin(auth::MAX_PAYLOAD)?;
    auth::check_payload(&payload, provider, &g["expected"][provider])?;
    let out = capture_input(
        &mut command(root, g, args)?,
        &payload,
        Duration::from_secs(30),
    )?;
    let leak = !auth::token_hashes(&[out.stdout.clone(), out.stderr.clone()].concat()).is_empty();
    let reply: Value = serde_json::from_slice(&out.stdout).unwrap_or(Value::Null);
    let status = reply["status"]
        .as_str()
        .filter(|s| ["imported", "error"].contains(s));
    let response_provider = reply["provider"]
        .as_str()
        .filter(|s| ["openai", "claude"].contains(s));
    audit(
        root,
        "import-audit.jsonl",
        &json!({"operation":"import","provider":provider,"stdin_bytes":payload.len(),"selected_tokens_only":true,"exit_code":out.status.code(),"output_leak":leak,"reply_status":status,"reply_provider":response_provider}),
    )?;
    if leak {
        eprintln!("Acceptance detected secret output; contents withheld");
        std::process::exit(94);
    }
    forward(out)
}
fn login_guard(root: &Path, g: &Value, args: &[String], tail: &[String]) -> Result<()> {
    ensure!(
        tail.first().is_some_and(|s| s == "login"),
        "Guard permits login only"
    );
    let provider = option(tail, "--provider").context("provider required")?;
    let flow = option(tail, "--flow-id").context("flow required")?;
    ensure!(
        ["openai", "claude"].contains(&provider)
            && regex::Regex::new(r"^[A-Za-z0-9_-]{1,64}$")?.is_match(flow),
        "Invalid provider/flow"
    );
    ensure!(
        !tail.iter().any(|s| s == "--complete"
            || s.starts_with("--auth-code")
            || (provider == "claude" && s.starts_with("--callback-url"))),
        "OAuth completion refused"
    );
    let mut record = json!({"provider":provider,"flow_id":flow});
    let payload;
    if tail.iter().any(|s| s == "--callback-url") {
        ensure!(
            option(tail, "--callback-url") == Some("-"),
            "Callback must use stdin"
        );
        payload = bounded_stdin(16384)?;
        let mut states = vec![];
        for p in files(&root.join("nexode/pending-login"))? {
            if p.file_name().is_some_and(|s| s == "openai.json") {
                states.push(field(&read_json(&p)?["login"], "state")?.to_owned());
            }
        }
        auth::callback_valid(&payload, &states)?;
        record["operation"] = "callback_stdin".into();
        record["mismatch_verified"] = true.into();
    } else if tail.iter().any(|s| s == "--print-auth-url") {
        payload = vec![];
        record["operation"] = "start".into();
    } else if tail.iter().any(|s| s == "--cancel") {
        payload = vec![];
        record["operation"] = "cancel".into();
    } else {
        anyhow::bail!("Acceptance guard refused login operation")
    }
    let flag = root.join("fail-next-login");
    if record["operation"] == "start" && flag.exists() {
        let b = fs::read(&flag)?;
        fs::remove_file(flag)?;
        audit(
            root,
            "login-audit.jsonl",
            &json!({"operation":"injected_start_failure","provider":provider,"flow_id":flow,"exit_code":72}),
        )?;
        std::io::stderr().write_all(&b)?;
        std::process::exit(72);
    }
    let out = capture_input(&mut command(root, g, args)?, &payload, TIMEOUT)?;
    record["exit_code"] = json!(out.status.code());
    record["state_mismatch"] = String::from_utf8_lossy(&out.stderr)
        .contains("OAuth state mismatch")
        .into();
    if record["operation"] == "start" && out.status.success() {
        let prompt: Value = serde_json::from_slice(&out.stdout)
            .map_err(|_| anyhow::anyhow!("Invalid authorization prompt"))?;
        let auth_url = field(&prompt, "auth_url")?;
        if provider == "openai" {
            record["auth_url"] = auth_url.into();
        } else {
            let v =
                read_json(&root.join(format!("nexode/pending-login/flows/{flow}/claude.json")))?;
            let verifier = field(&v["login"], "verifier")?;
            let u = url::Url::parse(auth_url)
                .map_err(|_| anyhow::anyhow!("Invalid private authorization URL"))?;
            let q: Vec<_> = u.query_pairs().collect();
            let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
            record["auth_url_sha256"] = auth::digest(auth_url.as_bytes()).into();
            record["auth_url_length"] = auth_url.len().into();
            record["pkce_matches"] = q
                .iter()
                .any(|(k, v)| k == "code_challenge" && v == &challenge)
                .into();
            record["legacy_state_is_verifier"] =
                q.iter().any(|(k, v)| k == "state" && v == verifier).into();
        }
    }
    audit(root, "login-audit.jsonl", &record)?;
    forward(out)
}
fn forward(out: Output) -> Result<()> {
    std::io::stdout().write_all(&out.stdout)?;
    std::io::stderr().write_all(&out.stderr)?;
    if !out.status.success() {
        std::process::exit(out.status.code().unwrap_or(1));
    }
    Ok(())
}
pub(super) fn call(c: &Config, op: &str, mut values: Value) -> Result<Value> {
    values["operation"] = op.into();
    values["owner"] = c.owner.clone().into();
    if let Some(root) = &c.root {
        values["root"] = root.clone().into();
    }
    let helper = c.helper.as_ref().context("remote helper required")?;
    let out = capture_input(
        &mut c.ssh(&format!("exec {} native-ssh-control", quote(helper))),
        &serde_json::to_vec(&values)?,
        TIMEOUT,
    )?;
    ensure!(
        out.status.success(),
        "Remote acceptance {op} failed; private output withheld"
    );
    ensure!(
        out.stdout.len() <= MAX_FRAME
            && auth::token_hashes(&[out.stdout.clone(), out.stderr].concat()).is_empty(),
        "Unsafe remote metadata"
    );
    serde_json::from_slice(&out.stdout).map_err(|_| anyhow::anyhow!("Invalid remote metadata"))
}
pub(super) fn create(c: &Config, mode: &str, mut values: Value) -> Result<(Config, Value)> {
    values["executable"] = c.remote.clone().into();
    values["mode"] = mode.into();
    values["cwd"] = c.cwd.clone().into();
    let created = call(c, "create", values)?;
    let mut c = c.clone();
    c.root = Some(field(&created, "root")?.into());
    c.remote = field(&created, "wrapper")?.into();
    c.socket = Some(field(&created, "socket")?.into());
    println!(
        "{}",
        json!({"isolated_remote_root":c.root,"server_socket":c.socket,"owner":c.owner,"cleanup":"coordinator owns isolated daemon/artifacts"})
    );
    Ok((c, created))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn guard_rejects_arbitrary_commands() {
        assert!(transport_tail(&["--model", "secret"].map(String::from)).is_err());
        assert_eq!(
            transport_tail(&["--no-update", "--cwd", "/a b", "server", "stdio"].map(String::from))
                .unwrap(),
            ["server", "stdio"]
        );
    }
    #[test]
    fn owned_root_requires_marker() {
        let d = tempfile::tempdir().unwrap();
        assert!(owned(d.path(), Some("owner")).is_err());
    }
}

#[cfg(test)]
mod privacy_tests {
    use super::*;
    #[test]
    fn metadata_never_exports_verifiers_and_reports_leaks() {
        let root = tempfile::tempdir().unwrap();
        let pending = root.path().join("nexode/pending-login/flows/test-flow");
        mkdir(&pending).unwrap();
        fs::write(
            pending.join("openai.lock"),
            b"not JSON: lock files are not pending flows",
        )
        .unwrap();
        private_write(&pending.join("openai.json"), &serde_json::to_vec(&json!({"login":{"state":"state","verifier":"never-export-verifier","redirect_uri":"http://localhost/auth/callback"}})).unwrap()).unwrap();
        private_write(
            &pending.join("claude.json"),
            &serde_json::to_vec(&json!({"login":{"verifier":"claude-private-verifier"}})).unwrap(),
        )
        .unwrap();
        fs::write(root.path().join("history.json"), "sensitive-callback").unwrap();
        let v = inspect_login(root.path(), &json!({"needles":["sensitive-callback"]})).unwrap();
        let text = v.to_string();
        assert!(!text.contains("never-export-verifier"));
        assert!(!text.contains("claude-private-verifier"));
        assert_eq!(v["leaks"], json!(["history.json"]));
        let claude = v["pending"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["provider"] == "claude")
            .unwrap();
        assert!(claude.get("state").is_none());
        assert!(claude.get("verifier").is_none());
    }
    #[test]
    fn remote_environment_is_private_and_closes_proxy_bypass() {
        let e = env(Path::new("/private"), true);
        assert_eq!(e["HOME"], "/private/home");
        assert_eq!(e["NO_PROXY"], "");
        assert_eq!(e["no_proxy"], "");
        assert_eq!(e["HTTPS_PROXY"], "http://127.0.0.1:9");
        assert!(!e.contains_key("OPENAI_API_KEY"));
        assert!(!e.contains_key("ANTHROPIC_API_KEY"));
    }
}
