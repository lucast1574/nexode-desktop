use super::*;
use super::{bridge::Bridge, tui::Tui};
pub(super) fn run(c: Config) -> Result<()> {
    let root = tempfile::Builder::new()
        .prefix("nexode-native-ssh-")
        .tempdir()?;
    init_root(root.path())?;
    let e = env(root.path(), false);
    let p = capture_input(
        &mut c.ssh(&c.remote_command()),
        b"{\"type\":\"ping\",\"id\":99}\n",
        TIMEOUT,
    )?;
    ensure!(p.status.success(), "SSH pipeline failed on stdin EOF");
    let frames: Vec<Value> = p
        .stdout
        .split(|b| *b == b'\n')
        .filter(|s| !s.is_empty())
        .map(serde_json::from_slice)
        .collect::<std::result::Result<_, _>>()?;
    ensure!(
        frames
            .first()
            .is_some_and(|v| v["kind"] == "nexode-native-stdio")
            && frames.iter().any(|v| v["type"] == "pong" && v["id"] == 99),
        "Pipeline discarded handshake or final pong"
    );
    let (h, id, sentinel) = bridge::seed(&c, "NEXODE_SSH_CONTEXT_")?;
    let reattach = || -> Result<()> {
        let mut b = Bridge::new(&c)?;
        b.handshake()?;
        let v = b.subscribe(
            h["working_dir"].as_str().context("cwd")?,
            &unique()?,
            Some(&id),
        )?;
        ensure!(
            v["session_id"] == id && contains(&v, &sentinel),
            "Fresh attach lost remote context"
        );
        b.finish()
    };
    reattach()?;
    no_transcript(&root.path().join("nexode"), &id, &sentinel)?;
    for cwd in [
        format!("{}/missing-{}", c.cwd, unique()?),
        "/dev/null".into(),
    ] {
        let mut bad = c.clone();
        bad.cwd = cwd;
        let p = crate::process::capture(
            bad.local().env_clear().envs(&e).current_dir(root.path()),
            TIMEOUT,
        )?;
        let text = tui::visible(&[p.stdout, p.stderr].concat());
        ensure!(
            !p.status.success() && !text.contains("Remote:"),
            "Invalid remote working directory silently fell back"
        );
    }
    for args in [
        vec!["--model", "must-not-run"],
        vec!["--tools", "none"],
        vec!["--resume"],
    ] {
        let p = crate::process::capture(
            c.local()
                .args(args)
                .env_clear()
                .envs(&e)
                .current_dir(root.path()),
            Duration::from_secs(15),
        )?;
        ensure!(
            !p.status.success()
                && !tui::visible(&[p.stdout, p.stderr].concat())
                    .contains("Connecting local Nexode UI"),
            "Unsupported flags were not rejected before SSH"
        );
    }
    for sighup in [false, true] {
        let mut t = Tui::new(&c, &e, root.path(), &id, false)?;
        t.wait(&format!("SSH {}", c.host), 0)?;
        t.wait(&sentinel, 0)?;
        let text = t.text().to_lowercase();
        for m in [
            "welcome to nexode",
            "choose your provider",
            "let's get you set up",
            "sign in to get started",
        ] {
            ensure!(!text.contains(m), "Unexpected local onboarding");
        }
        t.quit(sighup)?;
        no_transcript(&root.path().join("nexode"), &id, &sentinel)?;
        reattach()?;
    }
    println!(
        "{}",
        json!({"status":"passed","host":c.host,"session_id":id,"sentinel":sentinel,"remote_version":h["version"],"remote_working_dir":h["working_dir"],"provider_turns_requested":0,"tui_exit_modes":["quit","sighup"]})
    );
    Ok(())
}
