use super::*;
type Case = (&'static str, fn(&mut Fixture) -> Result<()>);
const EXPIRY: u64 = 4102444800000;
pub(super) fn tests() -> Vec<Case> {
    vec![
        ("openai_roundtrip", |f| roundtrip(f, "openai")),
        ("claude_roundtrip", |f| roundtrip(f, "claude")),
        ("all_accounts_move_and_return", all_accounts),
        ("equal_expiry_replaces_stale_remote", equal_expiry),
        ("dry_run_copies_no_credentials", dry_run),
        ("expired_access_transfer", |f| {
            f.seed("openai", 1)?;
            f.move_out()?;
            ensure!(
                f.remote.join(f.filename).exists(),
                "expired account not transferred"
            );
            Ok(())
        }),
        ("identityless_refresh_fingerprint", identityless),
        ("broken_transport_redaction", broken),
        ("local_refresh_conflict", local_refresh),
        ("conflicting_remote_login", remote_conflict),
        ("return_conflict_ownership_recovery", return_conflict),
        ("router_one_way_transfer", router),
        ("router_without_oauth", router_without_oauth),
        ("old_remote_rejected_before_transfer", old_remote),
    ]
}
fn roundtrip(f: &mut Fixture, provider: &str) -> Result<()> {
    f.seed(provider, EXPIRY)?;
    f.move_out()?;
    let path = f.remote.join(f.filename);
    let mut data = read(&path)?;
    ensure!(
        data[f.accounts][0]["label"] == format!("{provider}-fox"),
        "wrong account label"
    );
    ensure!(
        mode(&path)? == 0o600 && mode(&f.remote)? == 0o700,
        "insecure credential permissions"
    );
    f.archives_secret_free()?;
    data[f.accounts][0][if provider == "openai" {
        "access_token"
    } else {
        "access"
    }] = json!("synthetic-refreshed-access");
    data[f.accounts][0][f.refresh] = json!(REFRESHED);
    write(path, &data)?;
    f.return_home()?;
    ensure!(
        read(f.local.join(f.filename))?[f.accounts][0][f.refresh] == REFRESHED,
        "refreshed token not returned"
    );
    Ok(())
}
fn all_accounts(f: &mut Fixture) -> Result<()> {
    f.seed("openai", EXPIRY)?;
    let mut data = read(f.local.join(f.filename))?;
    let mut second = data[f.accounts][0].clone();
    second["label"] = json!("openai-otter");
    second["account_id"] = json!("second-account");
    data[f.accounts].as_array_mut().unwrap().push(second);
    write(f.local.join(f.filename), &data)?;
    write(
        f.local.join("auth.json"),
        &json!({"anthropic_accounts":(["claude-fox","claude-otter"].map(|label|json!({"label":label,"access":ACCESS,"refresh":REFRESH,"expires":1,"email":format!("{label}@example.invalid")}))),"active_anthropic_account":"claude-otter"}),
    )?;
    private(&f.local.join("auth.json"), 0o600)?;
    f.move_out()?;
    let stores = [
        ("openai-auth.json", "openai_accounts", "refresh_token"),
        ("auth.json", "anthropic_accounts", "refresh"),
    ];
    for (filename, key, refresh) in stores {
        let path = f.remote.join(filename);
        let mut data = read(&path)?;
        let accounts = data[key].as_array_mut().context("missing accounts")?;
        ensure!(accounts.len() == 2, "account missing in handoff");
        for account in accounts {
            account[refresh] = json!(REFRESHED);
        }
        write(path, &data)?;
    }
    f.archives_secret_free()?;
    f.return_home()?;
    for (filename, key, refresh) in stores {
        let data = read(f.local.join(filename))?;
        ensure!(
            data[key]
                .as_array()
                .context("accounts missing")?
                .iter()
                .all(|a| a[refresh] == REFRESHED),
            "not all tokens returned"
        );
    }
    ensure!(
        read(f.local.join("auth.json"))?["active_anthropic_account"] == "claude-otter",
        "local selection changed"
    );
    Ok(())
}
fn equal_expiry(f: &mut Fixture) -> Result<()> {
    f.seed("openai", EXPIRY)?;
    let mut data = read(f.local.join(f.filename))?;
    data[f.accounts][0][f.refresh] = json!("old-remote-refresh");
    let path = f.remote.join(f.filename);
    write(&path, &data)?;
    private(&path, 0o600)?;
    f.move_out()?;
    ensure!(
        read(path)?[f.accounts][0][f.refresh] == REFRESH,
        "stale remote token not replaced"
    );
    Ok(())
}
fn dry_run(f: &mut Fixture) -> Result<()> {
    f.seed("openai", EXPIRY)?;
    f.transfer("move", &["--dry-run"], true)?;
    ensure!(
        !f.remote.join(f.filename).exists() && !f.local.join(LEASE).exists(),
        "dry-run mutated credentials or lease"
    );
    f.archives_secret_free()
}
fn identityless(f: &mut Fixture) -> Result<()> {
    f.seed("openai", EXPIRY)?;
    let path = f.local.join(f.filename);
    let mut data = read(&path)?;
    let account = data[f.accounts][0]
        .as_object_mut()
        .context("account missing")?;
    account.remove("account_id");
    account.remove("email");
    write(&path, &data)?;
    f.move_out()?;
    let remote = f.remote.join(f.filename);
    let mut data = read(&remote)?;
    data[f.accounts][0][f.refresh] = json!(REFRESHED);
    write(remote, &data)?;
    f.return_home()?;
    ensure!(
        read(path)?[f.accounts][0][f.refresh] == REFRESHED,
        "fingerprint refresh not returned"
    );
    Ok(())
}
fn broken(f: &mut Fixture) -> Result<()> {
    f.seed("openai", EXPIRY)?;
    f.change_transport(
        "case \"$1\" in *credentials*--help*) ;; *credentials*) cat >&2; exit 1 ;; esac",
    )?;
    f.transfer("move", &[], false)?;
    Ok(())
}
fn local_refresh(f: &mut Fixture) -> Result<()> {
    f.seed("openai", EXPIRY)?;
    f.move_out()?;
    let path = f.local.join(f.filename);
    let mut data = read(&path)?;
    data[f.accounts][0][f.refresh] = json!("newer-local-refresh");
    write(&path, &data)?;
    let out = f.transfer("return", &[], false)?;
    ensure!(
        String::from_utf8_lossy(&out.stderr).contains("local OAuth account changed"),
        "missing local refresh conflict"
    );
    ensure!(
        read(path)?[f.accounts][0][f.refresh] == "newer-local-refresh",
        "newer local token overwritten"
    );
    Ok(())
}
fn remote_conflict(f: &mut Fixture) -> Result<()> {
    f.seed("openai", EXPIRY)?;
    let mut data = read(f.local.join(f.filename))?;
    for (key, value) in [
        ("account_id", "different-user"),
        ("access_token", "different-access"),
        ("refresh_token", "different-refresh"),
    ] {
        data[f.accounts][0][key] = json!(value);
    }
    let path = f.remote.join(f.filename);
    write(&path, &data)?;
    private(&path, 0o600)?;
    let original = fs::read(&path)?;
    f.transfer("move", &[], false)?;
    ensure!(
        fs::read(path)? == original,
        "conflicting remote login changed"
    );
    ensure!(
        serde_json::from_slice::<Value>(
            &f.invoke(&["where", "--session", SID, "--json"], true)?
                .stdout
        )? == json!([]),
        "failed move changed ownership"
    );
    Ok(())
}
fn return_conflict(f: &mut Fixture) -> Result<()> {
    f.seed("openai", EXPIRY)?;
    f.move_out()?;
    let path = f.local.join(f.filename);
    let mut data = read(&path)?;
    data[f.accounts][0]["account_id"] = json!("different-local-user");
    write(&path, &data)?;
    let out = f.transfer("return", &[], false)?;
    ensure!(
        String::from_utf8_lossy(&out.stderr).contains("remains on the cloud host"),
        "missing ownership conflict"
    );
    let remote: Value = serde_json::from_slice(
        &f.invoke_at(&f.remote, &["where", "--session", SID, "--json"], true)?
            .stdout,
    )?;
    ensure!(
        remote[0]["location"] == "local",
        "cloud ownership not restored"
    );
    ensure!(
        read(f.remote.join(SESSION))?["migration_epoch"] == remote[0]["epoch"],
        "cloud epoch mismatch"
    );
    ensure!(
        read(&path)?[f.accounts][0]["account_id"] == "different-local-user",
        "local login overwritten"
    );
    let original = fs::read(&path)?;
    let out = f.transfer("return", &["--refs-only", "--keep-local-auth"], true)?;
    ensure!(
        String::from_utf8_lossy(&out.stderr).contains("keeping local OAuth credentials"),
        "missing keep-local diagnostic"
    );
    ensure!(
        fs::read(path)? == original,
        "recovery overwrote local login"
    );
    let local: Value = serde_json::from_slice(
        &f.invoke(&["where", "--session", SID, "--json"], true)?
            .stdout,
    )?;
    ensure!(
        local[0]["location"] == "local"
            && read(f.remote.join(LEASE))?["away_host"] == "returned-complete",
        "recovery ownership incomplete"
    );
    Ok(())
}
fn router(f: &mut Fixture) -> Result<()> {
    f.seed("openai", EXPIRY)?;
    let config = f.seed_router()?;
    f.move_out()?;
    f.archives_secret_free()?;
    for name in ["openrouter.env", "bedrock.env"] {
        let path = config.join(name);
        ensure!(
            fs::read_to_string(&path)?.contains(ROUTER) && mode(&path)? == 0o600,
            "router transfer or permissions incorrect"
        );
    }
    ensure!(mode(&config)? == 0o700, "insecure config directory");
    ensure!(
        fs::read_to_string(config.join("groq.env"))? == "GROQ_API_KEY=cloud-only\n",
        "cloud-only key overwritten"
    );
    ensure!(
        !config.join("unlisted.env").exists() && !config.join("config.toml").exists(),
        "unlisted config transferred"
    );
    let gemini = f.remote.join("gemini_oauth.json");
    ensure!(
        read(&gemini)?["refresh_token"] == ROUTER && mode(&gemini)? == 0o600,
        "Gemini transfer incorrect"
    );
    let pool = read(f.remote.join("account-pool.json"))?;
    ensure!(
        pool["members"] == json!(["gemini", "openrouter", "cloud-only"])
            && pool["excluded"] == json!(["xai"]),
        "router pool merge incorrect"
    );
    fs::write(
        config.join("openrouter.env"),
        "OPENROUTER_API_KEY=cloud-changed\n",
    )?;
    f.return_home()?;
    ensure!(
        fs::read_to_string(f.local.join("config/nexode/openrouter.env"))?.contains(ROUTER),
        "router key flowed back unexpectedly"
    );
    Ok(())
}
fn router_without_oauth(f: &mut Fixture) -> Result<()> {
    f.seed("openai", EXPIRY)?;
    fs::remove_file(f.local.join(f.filename))?;
    let path = f.local.join(SESSION);
    let mut data = read(&path)?;
    data["provider_key"] = json!("openrouter");
    data["model"] = json!("synthetic/test");
    data["route_api_method"] = Value::Null;
    write(path, &data)?;
    let config = f.seed_router()?;
    f.move_out()?;
    ensure!(
        fs::read_to_string(config.join("openrouter.env"))?.contains(ROUTER),
        "router-only credentials missing"
    );
    Ok(())
}
fn old_remote(f: &mut Fixture) -> Result<()> {
    f.seed("openai", EXPIRY)?;
    let config = f.seed_router()?;
    f.change_transport("case \"$1\" in *router-credentials*) echo \"error: unrecognized subcommand router-credentials\" >&2; exit 2 ;; esac")?;
    let out = f.transfer("move", &[], false)?;
    ensure!(
        String::from_utf8_lossy(&out.stderr)
            .contains("does not support the complete cloud handoff protocol"),
        "missing capability diagnostic"
    );
    ensure!(
        !f.remote.join(f.filename).exists() && !f.remote.join("gemini_oauth.json").exists(),
        "credentials transferred before capability check"
    );
    ensure!(
        fs::read_to_string(config.join("openrouter.env"))? == "OPENROUTER_API_KEY=stale\n"
            && f.archives()?.is_empty(),
        "old remote mutated before rejection"
    );
    Ok(())
}
