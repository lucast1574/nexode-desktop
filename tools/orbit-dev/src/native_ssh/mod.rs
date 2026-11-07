//! Opt-in native SSH acceptance. All auth fixtures are synthetic and isolated.
mod auth;
mod bridge;
mod cli;
mod import;
mod login;
mod remote;
mod tui;
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Seek, Write},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    time::Duration,
};
const PREFIX: &str = "NEXODE_NATIVE_SSH_";
const MAX_FRAME: usize = 8 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(60);
const FLAGS: &[&str] = &[
    "-T",
    "-o",
    "BatchMode=yes",
    "-o",
    "StrictHostKeyChecking=yes",
    "-o",
    "ForwardAgent=no",
    "-o",
    "ClearAllForwardings=yes",
    "-o",
    "ServerAliveInterval=15",
    "-o",
    "ServerAliveCountMax=2",
    "-o",
    "PermitLocalCommand=no",
    "-o",
    "ForkAfterAuthentication=no",
    "-o",
    "StdinNull=no",
    "-o",
    "RemoteCommand=none",
    "-o",
    "SessionType=default",
    "-o",
    "ControlMaster=no",
    "-S",
    "none",
    "-o",
    "ConnectTimeout=30",
];
#[derive(Clone)]
struct Config {
    binary: PathBuf,
    host: String,
    remote: String,
    cwd: String,
    socket: Option<String>,
    helper: Option<String>,
    root: Option<String>,
    owner: String,
}
fn value(key: &str) -> Option<String> {
    std::env::var(format!("{PREFIX}{key}"))
        .ok()
        .filter(|s| !s.is_empty())
}
fn clean_path(s: &str) -> bool {
    !s.is_empty() && !s.chars().any(|c| c.is_control())
}
fn configured(mode: &str) -> Result<Option<Config>> {
    if mode != "cli" && value(&mode.to_uppercase()).as_deref() != Some("1") {
        println!("SKIP native SSH {mode}: explicit opt-in is unset");
        return Ok(None);
    }
    let remote = if mode == "cli" {
        value("REMOTE_BINARY")
    } else {
        value(&format!("{}_REMOTE_EXECUTABLE", mode.to_uppercase()))
    };
    let inputs = [value("BINARY"), value("HOST"), remote, value("CWD")];
    if mode == "cli" && inputs.iter().all(Option::is_none) {
        println!("SKIP native SSH: explicit host/binaries/cwd are unset");
        return Ok(None);
    }
    ensure!(
        inputs.iter().all(Option::is_some),
        "Incomplete native SSH opt-in configuration"
    );
    let [binary, host, remote, cwd] = inputs.map(Option::unwrap);
    ensure!(
        regex::Regex::new(
            r"^(?:[A-Za-z0-9_][A-Za-z0-9_.-]*@)?[A-Za-z0-9_\[:][A-Za-z0-9_.:\[\]%-]*$"
        )?
        .is_match(&host),
        "Invalid SSH host/alias"
    );
    ensure!(
        clean_path(&remote) && !remote.starts_with('-') && clean_path(&cwd),
        "Invalid remote path"
    );
    let binary = PathBuf::from(binary).canonicalize()?;
    ensure!(
        fs::metadata(&binary)?.permissions().mode() & 0o111 != 0,
        "Local binary is not executable"
    );
    let helper = value("REMOTE_HELPER");
    if mode != "cli" {
        ensure!(
            remote.starts_with('/') && cwd.starts_with('/'),
            "Auth probes require absolute remote ELF and working directory"
        );
        elf(&binary)?;
        ensure!(
            helper
                .as_ref()
                .is_some_and(|s| s.starts_with('/') && clean_path(s)),
            "Set NEXODE_NATIVE_SSH_REMOTE_HELPER to an existing remote orbit-dev ELF"
        );
    }
    Ok(Some(Config {
        binary,
        host,
        remote,
        cwd,
        socket: if mode == "cli" {
            value("SERVER_SOCKET")
        } else {
            None
        },
        helper,
        root: None,
        owner: unique()?,
    }))
}
fn unique() -> Result<String> {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).map_err(|_| anyhow::anyhow!("random source unavailable"))?;
    Ok(b.iter().map(|n| format!("{n:02x}")).collect())
}
fn elf(p: &Path) -> Result<()> {
    let mut b = [0; 4];
    fs::File::open(p)?.read_exact(&mut b)?;
    ensure!(&b == b"\x7fELF", "Expected actual ELF, not a wrapper");
    Ok(())
}
fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\"'\"'"))
}
impl Config {
    fn ssh(&self, command: &str) -> Command {
        let mut c = Command::new("ssh");
        c.args(FLAGS).args(["--", &self.host, command]);
        c
    }
    fn remote_command(&self) -> String {
        let mut a = vec![
            self.remote.as_str(),
            "--no-update",
            "--no-selfdev",
            "--cwd",
            &self.cwd,
        ];
        if let Some(s) = &self.socket {
            a.extend(["--socket", s]);
        }
        a.extend(["server", "stdio"]);
        format!(
            "PATH=\"$HOME/.local/bin:$HOME/.cargo/bin:$PATH\"; export PATH; exec {}",
            a.into_iter().map(quote).collect::<Vec<_>>().join(" ")
        )
    }
    fn local(&self) -> Command {
        let mut c = Command::new(&self.binary);
        c.args([
            "--no-update",
            "--no-selfdev",
            "--ssh",
            &self.host,
            "--ssh-binary",
            &self.remote,
            "--remote-working-dir",
            &self.cwd,
        ]);
        if let Some(s) = &self.socket {
            c.args(["--ssh-server-socket", s]);
        }
        c
    }
}
fn capture_input(c: &mut Command, input: &[u8], timeout: Duration) -> Result<Output> {
    let mut f = tempfile::tempfile()?;
    f.write_all(input)?;
    f.rewind()?;
    crate::process::capture_with_stdin(c, Stdio::from(f), timeout)
}
fn mkdir(p: &Path) -> Result<()> {
    fs::create_dir_all(p)?;
    fs::set_permissions(p, fs::Permissions::from_mode(0o700))?;
    Ok(())
}
fn private_write(p: &Path, bytes: &[u8]) -> Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(p)?;
    f.write_all(bytes)?;
    fs::set_permissions(p, fs::Permissions::from_mode(0o600))?;
    Ok(())
}
fn init_root(p: &Path) -> Result<()> {
    for n in ["home", "nexode", "runtime"] {
        mkdir(&p.join(n))?;
    }
    Ok(())
}
fn env(root: &Path, auth: bool) -> BTreeMap<String, String> {
    let mut e: BTreeMap<String, String> = if auth {
        std::env::vars()
            .filter(|(k, _)| {
                [
                    "PATH",
                    "LOGNAME",
                    "LANG",
                    "LC_ALL",
                    "SSH_AUTH_SOCK",
                    "SSH_AGENT_PID",
                ]
                .contains(&k.as_str())
            })
            .collect()
    } else {
        std::env::vars()
            .filter(|(k, _)| {
                !k.starts_with("NEXODE_")
                    && ![
                        "DISPLAY",
                        "WAYLAND_DISPLAY",
                        "KITTY_LISTEN_ON",
                        "TMUX",
                        "ZELLIJ",
                    ]
                    .contains(&k.as_str())
            })
            .collect()
    };
    for (k, v) in [
        ("NEXODE_HOME", root.join("nexode")),
        ("NEXODE_RUNTIME_DIR", root.join("runtime")),
        ("XDG_RUNTIME_DIR", root.join("runtime")),
    ] {
        e.insert(k.into(), v.display().to_string());
    }
    for (k, v) in [
        ("TERM", "xterm-256color"),
        ("NEXODE_NO_TELEMETRY", "1"),
        ("DO_NOT_TRACK", "1"),
        ("NEXODE_WAKE_MODE", "external"),
        ("NO_COLOR", "0"),
    ] {
        e.insert(k.into(), v.into());
    }
    if auth {
        for (k, v) in [
            ("HOME", root.join("home")),
            ("XDG_CONFIG_HOME", root.join("home/.config")),
            ("XDG_CACHE_HOME", root.join("home/.cache")),
        ] {
            e.insert(k.into(), v.display().to_string());
        }
        for (k, v) in [
            ("NEXODE_NO_BROWSER", "1"),
            ("NO_BROWSER", "1"),
            ("BROWSER", "/bin/false"),
            ("HTTP_PROXY", "http://127.0.0.1:9"),
            ("HTTPS_PROXY", "http://127.0.0.1:9"),
            ("ALL_PROXY", "http://127.0.0.1:9"),
            ("NO_PROXY", ""),
            ("http_proxy", "http://127.0.0.1:9"),
            ("https_proxy", "http://127.0.0.1:9"),
            ("all_proxy", "http://127.0.0.1:9"),
            ("no_proxy", ""),
        ] {
            e.insert(k.into(), v.into());
        }
    }
    e
}
fn files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut out = vec![];
    if !root.exists() {
        return Ok(out);
    }
    for entry in fs::read_dir(root)? {
        let p = entry?.path();
        let m = fs::symlink_metadata(&p)?;
        ensure!(
            !m.is_symlink(),
            "Unexpected symlink in private acceptance state"
        );
        if m.is_dir() {
            out.extend(files(&p)?)
        } else if m.is_file() {
            ensure!(m.len() <= 64 * 1024 * 1024, "Oversized acceptance artifact");
            out.push(p)
        }
    }
    Ok(out)
}
fn no_transcript(home: &Path, id: &str, sentinel: &str) -> Result<()> {
    for p in files(&home.join("sessions"))? {
        ensure!(
            !p.file_name().unwrap().to_string_lossy().contains(id)
                && !fs::read(p)?
                    .windows(sentinel.len())
                    .any(|s| s == sentinel.as_bytes()),
            "Remote transcript leaked into local state"
        );
    }
    Ok(())
}
fn contains(h: &Value, s: &str) -> bool {
    h["messages"]
        .as_array()
        .is_some_and(|a| a.iter().any(|v| v.to_string().contains(s)))
}
pub fn run(name: &str, args: impl Iterator<Item = String>) -> Result<()> {
    let args: Vec<_> = args.collect();
    match name {
        "native-ssh-control" => {
            ensure!(args.is_empty(), "control receives JSON only on stdin");
            remote::control()
        }
        "native-ssh-guard" => remote::guard(args),
        _ => {
            ensure!(
                args.is_empty(),
                "Acceptance uses explicit NEXODE_NATIVE_SSH_* environment; offline checks: cargo test native_ssh"
            );
            let mode = name.trim_start_matches("test-native-ssh-");
            if let Some(c) = configured(mode)? {
                match mode {
                    "cli" => cli::run(c)?,
                    "login" => login::run(c)?,
                    "import" => import::run(c)?,
                    _ => anyhow::bail!("unknown SSH suite"),
                }
            }
            Ok(())
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quote_preserves_shell_metacharacters() {
        let s = "/a' b$(echo unsafe)";
        let a = shlex::split(&format!("exec {} --cwd {}", quote(s), quote(s))).unwrap();
        assert_eq!(a, vec!["exec", s, "--cwd", s]);
    }
    #[test]
    fn history_not_metadata() {
        assert!(!contains(
            &json!({"session_id":"marker","messages":[]}),
            "marker"
        ));
        assert!(contains(
            &json!({"messages":[{"content":"marker"}]}),
            "marker"
        ));
    }
    #[test]
    fn transcript_leak_detection() {
        let d = tempfile::tempdir().unwrap();
        mkdir(&d.path().join("sessions")).unwrap();
        no_transcript(d.path(), "sid", "marker").unwrap();
        fs::write(d.path().join("sessions/leak"), "marker").unwrap();
        assert!(no_transcript(d.path(), "sid", "marker").is_err());
    }
}
