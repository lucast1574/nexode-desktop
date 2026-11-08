use super::*;
use super::{bridge::Bridge, tui::Tui};
fn snapshot(
    s: &Value,
    provider: &str,
    baseline: &Value,
    expected: &Value,
    imported: bool,
    calls: usize,
) -> Result<()> {
    let entries = s["calls"].as_array().context("calls")?;
    let starts = s["starts"].as_array().context("starts")?;
    ensure!(
        entries.len() == calls
            && starts.len() == calls
            && starts.iter().all(|s| s["provider"] == provider),
        "Unexpected started/incomplete/wrong-provider import"
    );
    for c in entries {
        ensure!(
            c["operation"] == "import"
                && c["provider"] == provider
                && c["selected_tokens_only"] == true
                && c["output_leak"] == false
                && c["reply_provider"] == provider,
            "Unsafe remote import"
        );
    }
    let other = if provider == "openai" {
        "claude"
    } else {
        "openai"
    };
    let path = |p: &str| {
        format!(
            "nexode/{}",
            auth::FILES.iter().find(|(s, _)| *s == p).unwrap().1
        )
    };
    ensure!(
        s["stores"][path(other)] == baseline["stores"][path(other)],
        "Other provider remote store changed"
    );
    let selected = path(provider);
    if imported {
        ensure!(
            s["stores"][&selected]["mode"] == 384
                && s["stores"][&selected]["tokens"] == expected[provider]
                && s["states"][provider] == "available",
            "Selected import missing/changed/not private"
        );
    } else {
        ensure!(
            s["stores"].get(&selected).is_none() && s["states"][provider] == "not_configured",
            "Cancellation transferred credentials"
        );
    }
    Ok(())
}
fn provider(
    config: &Config,
    provider: &str,
    root: &Path,
    e: &BTreeMap<String, String>,
    local: &Value,
    expected: &Value,
) -> Result<()> {
    let (c, _) = remote::create(
        config,
        "import",
        json!({"provider":provider,"expected":expected}),
    )?;
    let result = (|| -> Result<()> {
        let baseline = remote::call(&c, "inspect", json!({}))?;
        snapshot(&baseline, provider, &baseline, expected, false, 0)?;
        let (_, id, sentinel) = bridge::seed(&c, "NEXODE_IMPORT_CONTEXT_")?;
        let mut after = Value::Null;
        for action in [
            "picker_cancel",
            "filtered_cancel",
            "arrow_cancel",
            "cancel",
            "import",
            "repeat",
        ] {
            println!("CHECK {provider} {action}");
            let cancel = action.ends_with("cancel");
            let mut t = Tui::new(&c, e, root, &id, true)?;
            t.wait(&format!("SSH {}", c.host), 0)?;
            t.wait(&sentinel, 0)?;
            let n = if matches!(action, "picker_cancel" | "filtered_cancel") {
                let n = t.command(if action == "filtered_cancel" {
                    "/login --import-local"
                } else {
                    "/login"
                })?;
                t.wait(&format!("Login on {}:", c.host), n)?;
                let label = format!(
                    "Import local {} login",
                    if provider == "openai" {
                        "OpenAI"
                    } else {
                        "Claude"
                    }
                );
                t.write(format!("\x1b[200~{label}\x1b[201~").as_bytes())?;
                t.repaint()?;
                t.wait(&label, n)?;
                let n = t.mark();
                t.write(b"\r")?;
                n
            } else {
                t.command(&format!("/login --import-local {provider}"))?
            };
            t.wait(
                &format!("Import your local {provider} login to {}?", c.host),
                n,
            )?;
            let pending = remote::call(&c, "inspect", json!({}))?;
            snapshot(
                &pending,
                provider,
                &baseline,
                expected,
                action == "repeat",
                usize::from(action == "repeat"),
            )?;
            ensure!(
                auth::scan(root, &auth::allowed())? == *local,
                "Consent changed local source stores"
            );
            ensure!(
                !t.text().contains("-otter"),
                "Picker exposed private account labels"
            );
            let n = if action == "arrow_cancel" {
                let n = t.mark();
                t.write(b"\x1b[A")?;
                t.pump(Duration::from_millis(200))?; // Highlighting Yes alone must not start a transfer.
                snapshot(
                    &remote::call(&c, "inspect", json!({}))?,
                    provider,
                    &baseline,
                    expected,
                    false,
                    0,
                )?;
                t.write(b"\x1b[B\r")?;
                n
            } else {
                t.command(if cancel {
                    "/cancel"
                } else if action == "import" {
                    "yes"
                } else {
                    "confirm"
                })?
            };
            let marker = if cancel {
                "SSH credential import cancelled".to_owned()
            } else if action == "import" {
                format!("SSH login: {provider} imported on the remote host.")
            } else {
                "SSH credential import failed".into()
            };
            t.wait(&marker, n)?;
            t.quit(false)?;
            let s = remote::call(&c, "inspect", json!({}))?;
            snapshot(
                &s,
                provider,
                &baseline,
                expected,
                !cancel,
                if cancel {
                    0
                } else if action == "import" {
                    1
                } else {
                    2
                },
            )?;
            ensure!(
                auth::scan(root, &auth::allowed())? == *local,
                "Local source stores changed"
            );
            if action == "import" {
                let call = s["calls"].as_array().unwrap().last().unwrap();
                ensure!(
                    call["exit_code"] == 0 && call["reply_status"] == "imported",
                    "Real import failed"
                );
                after = s["stores"].clone();
            }
            if action == "repeat" {
                let call = s["calls"].as_array().unwrap().last().unwrap();
                ensure!(
                    call["exit_code"] != 0
                        && call["reply_status"] == "error"
                        && s["stores"] == after,
                    "Repeated import did not refuse without overwrite"
                );
            }
            println!(
                "PASS {provider} {action}: synthetic stdin, isolated stores, private output, SSH/socket cleanup"
            );
        }
        let mut b = Bridge::new(&c)?;
        b.handshake()?;
        let history = b.subscribe(&c.cwd, &unique()?, Some(&id))?;
        ensure!(
            auth::token_hashes(&serde_json::to_vec(&history)?).is_empty(),
            "Synthetic token in remote history"
        );
        b.finish()?;
        ensure!(
            auth::scan(root, &auth::allowed())? == *local,
            "Local store changed after reattach"
        );
        Ok(())
    })();
    match remote::call(&c, "inspect", json!({})) {
        Ok(s) => println!(
            "{}",
            json!({"isolated_remote_root":c.root,"server_socket":c.socket,"owner":c.owner,"processes":s["processes"]})
        ),
        Err(_) => eprintln!(
            "Remote identity inspection failed; inspect previously reported private root/socket"
        ),
    };
    result
}
pub(super) fn run(c: Config) -> Result<()> {
    let d = tempfile::Builder::new()
        .prefix("nexode-ssh-import-")
        .tempdir()?;
    init_root(d.path())?;
    let mut expected = json!({});
    for (p, name) in auth::FILES {
        let (s, _) = auth::fixture(p)?;
        let b = serde_json::to_vec(&s)?;
        private_write(&d.path().join("nexode").join(name), &b)?;
        expected[p] = json!(auth::token_hashes(&b));
    }
    let local = auth::scan(d.path(), &auth::allowed())?;
    let e = env(d.path(), true);
    for (p, _) in auth::FILES {
        let mut c = c.clone();
        c.owner = unique()?;
        provider(&c, p, d.path(), &e, &local, &expected)?;
    }
    println!(
        "{}",
        json!({"status":"passed","providers":["openai","claude"],"cases_per_provider":6,"provider_turns_requested":0,"personal_credentials_copied":false})
    );
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancellation_cannot_hide_started_transfer() {
        let base = json!({"calls":[],"starts":[],"stores":{"nexode/auth.json":{"sha256":"unchanged"}},"states":{"openai":"not_configured"}});
        snapshot(&base, "openai", &base, &json!({}), false, 0).unwrap();
        let mut bad = base.clone();
        bad["starts"] = json!([{"provider":"openai"}]);
        assert!(snapshot(&bad, "openai", &base, &json!({}), false, 0).is_err());
    }
}
