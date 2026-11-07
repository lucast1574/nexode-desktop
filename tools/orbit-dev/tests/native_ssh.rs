#![cfg(target_os = "linux")]
#[allow(dead_code)]
#[path = "../src/process.rs"]
mod process;
use serde_json::Value;
use std::{
    fs,
    net::TcpListener,
    os::{
        fd::AsRawFd,
        unix::{fs::PermissionsExt, net::UnixStream, process::CommandExt},
    },
    path::Path,
    process::{Child, Command, Stdio},
    time::Duration,
};
fn tool() -> Command {
    Command::new(env!("CARGO_BIN_EXE_orbit-dev"))
}
#[test]
fn unset_opt_in_is_offline_and_partial_configuration_fails() {
    for mode in ["cli", "login", "import"] {
        let o = tool()
            .arg(format!("test-native-ssh-{mode}"))
            .env_clear()
            .env("PATH", "/nonexistent")
            .output()
            .unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        assert!(String::from_utf8_lossy(&o.stdout).contains("SKIP"));
    }
    let o = tool()
        .arg("test-native-ssh-cli")
        .env_clear()
        .env("PATH", "/nonexistent")
        .env("NEXODE_NATIVE_SSH_HOST", "must-not-connect")
        .output()
        .unwrap();
    assert!(!o.status.success());
}
fn q(p: &Path) -> String {
    format!("'{}'", p.display().to_string().replace('\'', "'\"'\"'"))
}
fn executable(p: &Path, s: String) {
    fs::write(p, s).unwrap();
    fs::set_permissions(p, fs::Permissions::from_mode(0o700)).unwrap();
}
struct Server(Child);
impl Drop for Server {
    fn drop(&mut self) {
        unsafe {
            libc::kill(-(self.0.id() as i32), libc::SIGKILL);
        }
        let _ = self.0.wait();
    }
}
fn stop_socket(p: &Path) {
    if let Ok(s) = UnixStream::connect(p) {
        let mut c: libc::ucred = unsafe { std::mem::zeroed() };
        let mut n = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        if unsafe {
            libc::getsockopt(
                s.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                (&mut c as *mut libc::ucred).cast(),
                &mut n,
            )
        } == 0
            && c.uid == unsafe { libc::getuid() }
        {
            unsafe {
                libc::kill(c.pid, libc::SIGTERM);
            }
        }
    }
}
#[test]
#[ignore = "requires a built runtime and real local sshd; set ORBIT_NATIVE_SSH_BINARY and ORBIT_NATIVE_SSH_SSHD"]
fn real_cli_login_and_import_over_isolated_openssh() {
    let binary =
        fs::canonicalize(std::env::var("ORBIT_NATIVE_SSH_BINARY").expect("runtime ELF required"))
            .unwrap();
    let sshd = std::env::var("ORBIT_NATIVE_SSH_SSHD").unwrap_or("/usr/sbin/sshd".into());
    let root = tempfile::Builder::new()
        .prefix("orbit-native-ssh-")
        .tempdir_in("/tmp")
        .unwrap();
    let r = root.path();
    for name in ["bin", "home", "runtime", "workspace"] {
        fs::create_dir(r.join(name)).unwrap();
    }
    for name in ["host_key", "client_key"] {
        let o = Command::new("ssh-keygen")
            .args(["-q", "-t", "ed25519", "-N", "", "-f"])
            .arg(r.join(name))
            .output()
            .unwrap();
        assert!(o.status.success());
    }
    fs::copy(r.join("client_key.pub"), r.join("authorized_keys")).unwrap();
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let key = fs::read_to_string(r.join("host_key.pub")).unwrap();
    let key = key.split_whitespace().take(2).collect::<Vec<_>>().join(" ");
    fs::write(r.join("known_hosts"), format!("[127.0.0.1]:{port} {key}\n")).unwrap();
    let username =
        String::from_utf8(Command::new("id").arg("-un").output().unwrap().stdout).unwrap();
    fs::write(r.join("config"),format!("Host native-acceptance\n HostName 127.0.0.1\n Port {port}\n User {}\n IdentityFile {}/client_key\n IdentitiesOnly yes\n StrictHostKeyChecking yes\n UserKnownHostsFile {}/known_hosts\n",username.trim(),r.display(),r.display())).unwrap();
    executable(
        &r.join("bin/ssh"),
        format!(
            "#!/bin/sh\nexec /usr/bin/ssh -F {} \"$@\"\n",
            q(&r.join("config"))
        ),
    );
    fs::write(r.join("sshd_config"),format!("Port {port}\nListenAddress 127.0.0.1\nHostKey {0}/host_key\nPidFile {0}/sshd.pid\nAuthorizedKeysFile {0}/authorized_keys\nPasswordAuthentication no\nKbdInteractiveAuthentication no\nUsePAM no\nStrictModes no\nLogLevel ERROR\n",r.display())).unwrap();
    let log = fs::File::create(r.join("sshd.log")).unwrap();
    let _server = Server(
        Command::new(sshd)
            .args(["-D", "-e", "-f"])
            .arg(r.join("sshd_config"))
            .stdin(Stdio::null())
            .stdout(log.try_clone().unwrap())
            .stderr(log)
            .process_group(0)
            .spawn()
            .unwrap(),
    );
    std::thread::sleep(Duration::from_millis(300));
    let check = Command::new(r.join("bin/ssh"))
        .args(["-o", "BatchMode=yes", "native-acceptance", "printf ready"])
        .output()
        .unwrap();
    assert!(
        check.status.success(),
        "SSH fixture: {}",
        String::from_utf8_lossy(&check.stderr)
    );
    let wrapper = r.join("remote-nexode");
    executable(
        &wrapper,
        format!(
            "#!/bin/sh\nexec env -i PATH=/usr/bin:/bin HOME={0} NEXODE_HOME={0} NEXODE_RUNTIME_DIR={1} NEXODE_NO_TELEMETRY=1 DO_NOT_TRACK=1 OPENAI_BASE_URL=http://127.0.0.1:9/v1 ANTHROPIC_BASE_URL=http://127.0.0.1:9 OPENAI_API_KEY=sk-local-fixture ANTHROPIC_API_KEY=sk-ant-local-fixture {2} \"$@\"\n",
            q(&r.join("home")),
            q(&r.join("runtime")),
            q(&binary)
        ),
    );
    for mode in ["cli", "login", "import"] {
        if std::env::var("ORBIT_NATIVE_SSH_SUITE").is_ok_and(|s| s != mode) {
            continue;
        }
        let mut c = tool();
        c.arg(format!("test-native-ssh-{mode}"))
            .env_clear()
            .env("PATH", format!("{}/bin:/usr/bin:/bin", r.display()))
            .env("HOME", r.join("home"))
            .env("LANG", "C.UTF-8")
            .env("NEXODE_NATIVE_SSH_BINARY", &binary)
            .env("NEXODE_NATIVE_SSH_HOST", "native-acceptance")
            .env("NEXODE_NATIVE_SSH_REMOTE_BINARY", &wrapper)
            .env("NEXODE_NATIVE_SSH_CWD", r.join("workspace"))
            .env(
                "NEXODE_NATIVE_SSH_SERVER_SOCKET",
                r.join("runtime/server.sock"),
            )
            .env("NEXODE_NATIVE_SSH_LOGIN", "1")
            .env("NEXODE_NATIVE_SSH_IMPORT", "1")
            .env("NEXODE_NATIVE_SSH_LOGIN_REMOTE_EXECUTABLE", &binary)
            .env("NEXODE_NATIVE_SSH_IMPORT_REMOTE_EXECUTABLE", &binary)
            .env(
                "NEXODE_NATIVE_SSH_REMOTE_HELPER",
                env!("CARGO_BIN_EXE_orbit-dev"),
            );
        let output = process::capture(&mut c, Duration::from_secs(900)).unwrap();
        let text = String::from_utf8_lossy(&output.stdout);
        for line in text.lines() {
            if let Ok(v) = serde_json::from_str::<Value>(line)
                && let (Some(root), Some(socket)) = (
                    v["isolated_remote_root"].as_str(),
                    v["server_socket"].as_str(),
                )
            {
                let root = Path::new(root);
                if root.starts_with("/tmp")
                    && root
                        .file_name()
                        .unwrap()
                        .to_string_lossy()
                        .starts_with("run-")
                    && root.join(".acceptance-owned").exists()
                {
                    stop_socket(Path::new(socket));
                }
            }
        }
        stop_socket(&r.join("runtime/server.sock"));
        fs::write(
            format!("/tmp/orbit-native-ssh-{mode}-result.log"),
            [output.stdout.clone(), output.stderr.clone()].concat(),
        )
        .unwrap();
        assert!(
            output.status.success(),
            "{mode}: {}\n{}\nfixture: {}",
            text,
            String::from_utf8_lossy(&output.stderr),
            r.display()
        );
        println!("{text}");
    }
}

fn input(command: &mut Command, bytes: &[u8]) -> std::process::Output {
    use std::io::{Seek, Write};
    let mut f = tempfile::tempfile().unwrap();
    f.write_all(bytes).unwrap();
    f.rewind().unwrap();
    process::capture_with_stdin(command, Stdio::from(f), Duration::from_secs(10)).unwrap()
}
#[test]
fn native_remote_guard_refuses_oauth_completion_and_unsafe_import() {
    for mode in ["login", "import"] {
        let owner = format!("test-{}-{mode}", std::process::id());
        let request = serde_json::json!({"operation":"create","mode":mode,"owner":owner,"executable":"/bin/false","cwd":"/tmp","provider":"openai","expected":{"openai":[],"claude":[]}});
        let o = input(
            tool().arg("native-ssh-control"),
            &serde_json::to_vec(&request).unwrap(),
        );
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        let created: Value = serde_json::from_slice(&o.stdout).unwrap();
        let root = Path::new(created["root"].as_str().unwrap());
        assert_eq!(
            fs::read_to_string(root.join(".acceptance-owned")).unwrap(),
            owner
        );
        let mut cases = if mode == "login" {
            vec![
                vec![
                    "login",
                    "--provider",
                    "claude",
                    "--flow-id",
                    "safe",
                    "--callback-url",
                    "-",
                ],
                vec![
                    "login",
                    "--provider",
                    "claude",
                    "--flow-id",
                    "safe",
                    "--callback-url=anything",
                ],
                vec![
                    "login",
                    "--provider",
                    "openai",
                    "--flow-id",
                    "safe",
                    "--auth-code",
                    "-",
                ],
                vec![
                    "login",
                    "--provider",
                    "openai",
                    "--flow-id",
                    "safe",
                    "--complete",
                ],
                vec![
                    "login",
                    "--provider",
                    "openai",
                    "--flow-id",
                    "safe",
                    "--callback-url",
                    "inline-secret",
                ],
                vec![
                    "login",
                    "--provider",
                    "openai",
                    "--flow-id",
                    "../escape",
                    "--print-auth-url",
                ],
            ]
        } else {
            vec![
                vec!["login", "--provider", "openai"],
                vec![
                    "auth",
                    "import",
                    "--provider",
                    "openai",
                    "--stdin",
                    "--json",
                    "--overwrite",
                ],
                vec![
                    "auth",
                    "import",
                    "--provider",
                    "openai",
                    "--token",
                    "NEXODE_SYNTHETIC_IMPORT_access_0123456789abcdef0123456789abcdef",
                ],
                vec!["run", "must-not-run"],
            ]
        };
        cases.push(vec![
            "__remote-inventory",
            "/outside-selected-workspace",
            "",
        ]);
        cases.push(vec!["__remote-inventory", "/tmp", "../../escape"]);
        for args in cases {
            let o = input(
                tool()
                    .args(["native-ssh-guard", root.to_str().unwrap()])
                    .args(args),
                b"must-not-reach-cli",
            );
            assert_eq!(o.status.code(), Some(93));
            let output = [o.stdout, o.stderr].concat();
            assert!(!String::from_utf8_lossy(&output).contains("inline-secret"));
            assert!(!String::from_utf8_lossy(&output).contains("NEXODE_SYNTHETIC_IMPORT_"));
        }
        let o = input(
            tool().args([
                "native-ssh-guard",
                root.to_str().unwrap(),
                "__remote-inventory",
                "/tmp",
                "",
            ]),
            b"",
        );
        assert_eq!(
            o.status.code(),
            Some(1),
            "Allowed read-only inventory reaches the chosen /bin/false ELF"
        );
        // A wrong owner cannot inspect or clean any state, including the legacy fixture.
        let before = fs::read(root.join(".acceptance-owned")).unwrap();
        let o = input(
            tool().arg("native-ssh-control"),
            &serde_json::to_vec(
                &serde_json::json!({"operation":"cleanup_pending","root":root,"owner":"wrong"}),
            )
            .unwrap(),
        );
        assert!(!o.status.success());
        assert_eq!(fs::read(root.join(".acceptance-owned")).unwrap(), before);
        assert!(root.starts_with(format!("/tmp/nexode-{mode}-acceptance")));
        fs::remove_dir_all(root).unwrap();
    }
}
