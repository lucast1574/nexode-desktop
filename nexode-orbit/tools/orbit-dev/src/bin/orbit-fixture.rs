//! Native subprocess fixtures used only by maintenance integration tests.
#[path = "fixture/auth.rs"]
mod auth;
#[cfg(unix)]
#[path = "fixture/browser.rs"]
mod browser;
#[path = "fixture/desktop.rs"]
mod desktop;
#[path = "fixture/hosted.rs"]
mod hosted;
#[path = "fixture/mcp.rs"]
mod mcp;
#[path = "fixture/release.rs"]
mod release;
#[path = "fixture/signing.rs"]
mod signing;
#[path = "fixture/transport.rs"]
mod transport;
use std::io::{self, Read, Write};
fn main() {
    #[cfg(unix)]
    if std::env::args().any(|s| s == "--desktop-daemon-fixture") {
        transport::desktop_daemon().unwrap();
        return;
    }
    #[cfg(target_os = "linux")]
    if std::env::args().any(|s| s == "--heap-server-fixture") {
        transport::heap_server().unwrap();
        return;
    }
    let invoked = std::env::args_os()
        .next()
        .and_then(|s| {
            std::path::PathBuf::from(s)
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
        })
        .unwrap_or_default();
    if std::env::var("ORBIT_HOSTED_FIXTURE").as_deref() == Ok("1")
        && ["justrust", "rustup", "cargo", "readelf"].contains(&invoked.as_str())
    {
        std::process::exit(hosted::run(&invoked).unwrap());
    }
    if ["uname", "security", "codesign", "ditto", "xcrun"].contains(&invoked.as_str()) {
        std::process::exit(signing::run(&invoked).unwrap());
    }
    if invoked == "gh" {
        std::process::exit(release::run().unwrap());
    }
    if [
        "Xvfb",
        "openbox",
        "nexode-desktop",
        "import",
        "cargo",
        "xdotool",
    ]
    .contains(&invoked.as_str())
    {
        desktop::run(&invoked).unwrap();
        return;
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|a| a == "--mcp-fixture") {
        mcp::run(
            args.get(1).expect("server"),
            args.get(2).expect("tool names"),
        );
        return;
    }
    #[cfg(unix)]
    if std::env::args_os()
        .next()
        .and_then(|s| {
            std::path::PathBuf::from(s)
                .file_stem()
                .map(|n| n.to_owned())
        })
        .is_some_and(|n| n == "browser")
    {
        std::process::exit(browser::run().unwrap());
    }
    if args.first().is_some_and(|a| a == "--ssh-fixture") {
        transport::ssh(std::path::Path::new(args.get(1).expect("frame file"))).unwrap();
        return;
    }
    #[cfg(unix)]
    if args.first().is_some_and(|a| a == "--bridge-fixture") {
        std::process::exit(transport::bridge(&args[1..]).unwrap());
    }
    if std::env::args_os()
        .next()
        .and_then(|s| {
            std::path::PathBuf::from(s)
                .file_stem()
                .map(|n| n.to_owned())
        })
        .is_some_and(|n| n == "orbit-auth-fixture")
    {
        match auth::run() {
            Ok(status) => std::process::exit(status),
            Err(e) => {
                eprintln!("auth fixture: {e:#}");
                std::process::exit(1);
            }
        }
    }
    match std::env::var("ORBIT_FIXTURE_MODE").as_deref() {
        #[cfg(target_os = "linux")]
        Ok("startup") => match std::env::args().nth(1).as_deref() {
            Some("--help" | "--version") => println!("fixture version"),
            Some("serve") => {
                let socket = std::env::var_os("NEXODE_SOCKET").unwrap();
                let listener = std::os::unix::net::UnixListener::bind(socket).unwrap();
                for connection in listener.incoming() {
                    drop(connection.unwrap());
                }
            }
            _ => {
                let logs =
                    std::path::PathBuf::from(std::env::var_os("NEXODE_HOME").unwrap()).join("logs");
                std::fs::create_dir_all(&logs).unwrap();
                let text = "=== Startup Profile (15ms total)\n[INFO] 1ms 1ms 7% server_check\n[INFO] 3ms 2ms 13% server_spawn_start\n[INFO] 15ms 12ms 80% app_new_for_remote\nremote bootstrap: history after 20ms\n";
                std::fs::write(logs.join("nexode-2026-11-06.log"), text).unwrap();
            }
        },
        Ok("resume-picker") => {
            let mut out = io::stdout().lock();
            write!(out, "\x1b[?1049h(s/S filter)\n1 sessions\n").unwrap();
            out.flush().unwrap();
            let mut key = [0; 1];
            io::stdin().read_exact(&mut key).unwrap();
            assert_eq!(key[0], b'q');
            write!(out, "\x1b[?1049l").unwrap();
            out.flush().unwrap();
        }
        Ok("large-version") => {
            let mut out = io::stdout().lock();
            writeln!(out, "version\tv1.0.0 (abc123)").unwrap();
            out.flush().unwrap();
            write!(out, "details\t{}", "x".repeat(1_000_000)).unwrap();
        }
        _ => {
            eprintln!("unknown ORBIT_FIXTURE_MODE");
            std::process::exit(2);
        }
    }
}
