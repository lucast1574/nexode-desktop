//! Owned Unix pseudoterminal; cleanup never targets an existing user session.
use anyhow::{Context, Result, ensure};
use std::{
    fs::File,
    io::{Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::process::CommandExt,
    },
    process::{Child, Command, ExitStatus, Stdio},
    time::{Duration, Instant},
};
pub struct Terminal {
    master: File,
    child: Child,
    pub eof: bool,
}
impl Terminal {
    pub fn spawn(command: &mut Command, rows: u16, cols: u16) -> Result<Self> {
        let (mut master, mut slave) = (-1, -1);
        let size = libc::winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        ensure!(
            unsafe {
                libc::openpty(
                    &mut master,
                    &mut slave,
                    std::ptr::null_mut(),
                    std::ptr::null(),
                    &size,
                )
            } == 0,
            "openpty: {}",
            std::io::Error::last_os_error()
        );
        let master = unsafe { File::from_raw_fd(master) };
        let slave = unsafe { File::from_raw_fd(slave) };
        let mut term = std::mem::MaybeUninit::<libc::termios>::uninit();
        ensure!(
            unsafe { libc::tcgetattr(slave.as_raw_fd(), term.as_mut_ptr()) } == 0,
            "read PTY attributes"
        );
        let mut term = unsafe { term.assume_init() };
        term.c_lflag &= !(libc::ECHO | libc::ICANON);
        term.c_iflag &= !(libc::ICRNL | libc::IXON);
        term.c_cc[libc::VMIN] = 1;
        term.c_cc[libc::VTIME] = 0;
        ensure!(
            unsafe { libc::tcsetattr(slave.as_raw_fd(), libc::TCSANOW, &term) } == 0,
            "set PTY attributes"
        );
        let flags = unsafe { libc::fcntl(master.as_raw_fd(), libc::F_GETFL) };
        ensure!(
            flags >= 0
                && unsafe {
                    libc::fcntl(master.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK)
                } >= 0,
            "set nonblocking PTY"
        );
        command
            .stdin(Stdio::from(slave.try_clone()?))
            .stdout(Stdio::from(slave.try_clone()?))
            .stderr(Stdio::from(slave));
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                if libc::ioctl(0, libc::TIOCSCTTY, 0) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = command.spawn().context("launch PTY child")?;
        Ok(Self {
            master,
            child,
            eof: false,
        })
    }
    pub fn read(&mut self, timeout: Duration) -> Result<Vec<u8>> {
        let mut poll = libc::pollfd {
            fd: self.master.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let ready = unsafe {
            libc::poll(
                &mut poll,
                1,
                timeout.as_millis().min(i32::MAX as u128) as i32,
            )
        };
        if ready < 0 {
            let e = std::io::Error::last_os_error();
            if e.kind() == std::io::ErrorKind::Interrupted {
                return Ok(Vec::new());
            }
            return Err(e.into());
        }
        if ready == 0 {
            return Ok(Vec::new());
        }
        let mut output = Vec::new();
        let mut buf = [0_u8; 65536];
        loop {
            match self.master.read(&mut buf) {
                Ok(0) => {
                    self.eof = true;
                    break;
                }
                Ok(n) => {
                    output.extend_from_slice(&buf[..n]);
                    ensure!(
                        output.len() <= 16 * 1024 * 1024,
                        "PTY output burst exceeded 16 MiB"
                    );
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(e) if e.raw_os_error() == Some(libc::EIO) => {
                    self.eof = true;
                    break;
                }
                Err(e) => return Err(e.into()),
            }
        }
        Ok(output)
    }
    pub fn write(&mut self, bytes: &[u8]) -> Result<()> {
        self.master.write_all(bytes)?;
        Ok(())
    }
    pub fn pid(&self) -> u32 {
        self.child.id()
    }
    // Some integration tests import this module without the SSH harness.
    #[allow(dead_code)]
    pub fn resize(&self, rows: u16, cols: u16) -> Result<()> {
        let size = libc::winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        ensure!(
            unsafe { libc::ioctl(self.master.as_raw_fd(), libc::TIOCSWINSZ, &size) } == 0,
            "resize PTY: {}",
            std::io::Error::last_os_error()
        );
        Ok(())
    }
    pub fn status(&mut self) -> Result<Option<ExitStatus>> {
        Ok(self.child.try_wait()?)
    }
    pub fn stop(&mut self) -> Result<()> {
        if self.child.try_wait()?.is_some() {
            return Ok(());
        }
        unsafe {
            libc::kill(-(self.child.id() as i32), libc::SIGTERM);
        }
        let until = Instant::now() + Duration::from_secs(2);
        while Instant::now() < until {
            if self.child.try_wait()?.is_some() {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        unsafe {
            libc::kill(-(self.child.id() as i32), libc::SIGKILL);
        }
        self.child.wait()?;
        Ok(())
    }
}
impl Drop for Terminal {
    fn drop(&mut self) {
        if let Err(e) = self.stop() {
            eprintln!("PTY cleanup: {e:#}");
        }
    }
}
