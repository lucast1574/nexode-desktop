use anyhow::{Context, Result, ensure};
use std::{
    fs,
    io::{self, Write},
    path::PathBuf,
    time::{Duration, Instant},
};
struct Raw(libc::termios);
impl Raw {
    fn enter() -> Result<Self> {
        let mut original = std::mem::MaybeUninit::<libc::termios>::uninit();
        ensure!(
            unsafe { libc::tcgetattr(0, original.as_mut_ptr()) } == 0,
            "stdin must be a terminal: {}",
            io::Error::last_os_error()
        );
        let original = unsafe { original.assume_init() };
        let mut raw = original;
        unsafe {
            libc::cfmakeraw(&mut raw);
        }
        ensure!(
            unsafe { libc::tcsetattr(0, libc::TCSANOW, &raw) } == 0,
            "cannot enter raw mode"
        );
        let guard = Self(original);
        print!("\x1b[>3u");
        io::stdout().flush()?;
        Ok(guard)
    }
}
impl Drop for Raw {
    fn drop(&mut self) {
        unsafe {
            libc::tcsetattr(0, libc::TCSADRAIN, &self.0);
        }
        let _ = io::stdout().write_all(b"\x1b[<u");
        let _ = io::stdout().flush();
    }
}
pub fn run(mut args: impl Iterator<Item = String>) -> Result<()> {
    let output = PathBuf::from(
        args.next()
            .context("capture-alt-shift-e requires an output path")?,
    );
    ensure!(args.next().is_none(), "unexpected argument");
    let _signals = crate::ci_suites::unix::Signals::install()?;
    let mut bytes = Vec::new();
    {
        let _raw = Raw::enter()?;
        let mut deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline {
            ensure!(
                crate::ci_suites::unix::interrupted().is_none(),
                "capture interrupted"
            );
            let mut poll = libc::pollfd {
                fd: 0,
                events: libc::POLLIN,
                revents: 0,
            };
            let result = unsafe { libc::poll(&mut poll, 1, 200) };
            if result < 0 {
                let e = io::Error::last_os_error();
                if e.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(e.into());
            }
            if result == 0 {
                continue;
            }
            if poll.revents & libc::POLLIN != 0 {
                let mut chunk = [0u8; 1024];
                let n = unsafe { libc::read(0, chunk.as_mut_ptr().cast(), chunk.len()) };
                if n == 0 {
                    break;
                }
                ensure!(
                    n > 0,
                    "terminal read failed: {}",
                    io::Error::last_os_error()
                );
                ensure!(bytes.len() + n as usize <= 1024 * 1024, "capture too large");
                bytes.extend_from_slice(&chunk[..n as usize]);
                deadline = Instant::now() + Duration::from_secs(1);
            } else if poll.revents & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) != 0 {
                break;
            }
        }
    }
    fs::write(
        output,
        format!(
            "{bytes:?}\n{}\n",
            bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
        ),
    )?;
    Ok(())
}
