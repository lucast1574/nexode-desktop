//! Native subprocess fixtures used only by maintenance integration tests.
use std::io::{self, Read, Write};
fn main() {
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
