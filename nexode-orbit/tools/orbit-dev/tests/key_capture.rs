#![cfg(unix)]
#[path = "../src/pty.rs"]
mod pty;
use std::{
    fs,
    time::{Duration, Instant},
};
mod tests {
    use super::*;
    #[test]
    fn real_pty_capture_receives_enhanced_keyboard_bytes() {
        // Exercise the actual executable so the raw-mode guard and its signal
        // handlers do not modify the test runner's terminal or global state.
        let tool = env!("CARGO_BIN_EXE_orbit-dev");
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("capture");
        let mut command = std::process::Command::new(tool);
        command.arg("capture-alt-shift-e").arg(&path);
        let mut pty = crate::pty::Terminal::spawn(&mut command, 24, 80).unwrap();
        assert!(pty.pid() > 0);
        assert!(pty.status().unwrap().is_none());
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut output = vec![];
        while !output.windows(5).any(|w| w == b"\x1b[>3u") {
            assert!(Instant::now() < deadline, "keyboard protocol not enabled");
            output.extend(pty.read(Duration::from_millis(20)).unwrap());
        }
        pty.write(b"\x1b[101;4u").unwrap();
        while !path.exists() {
            assert!(Instant::now() < deadline);
            output.extend(pty.read(Duration::from_millis(20)).unwrap());
        }
        assert!(
            fs::read_to_string(path)
                .unwrap()
                .contains("1b5b3130313b3475")
        );
    }
}
