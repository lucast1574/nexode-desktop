//! Native subprocess fixtures used only by maintenance integration tests.
use std::io::{self, Write};
fn main() {
    match std::env::var("ORBIT_FIXTURE_MODE").as_deref() {
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
