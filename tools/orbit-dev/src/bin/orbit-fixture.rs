//! Native subprocess fixtures used only by maintenance integration tests.
use std::io::{self, Read, Write};
fn main() {
    match std::env::var("ORBIT_FIXTURE_MODE").as_deref() {
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
