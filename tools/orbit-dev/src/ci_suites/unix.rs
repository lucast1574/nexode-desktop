use anyhow::{Context, Result};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::{
    process::{Child, Command},
    sync::atomic::{AtomicI32, Ordering},
    time::{Duration, Instant},
};
static SIGNAL: AtomicI32 = AtomicI32::new(0);
extern "C" fn record(signal: libc::c_int) {
    SIGNAL.store(signal, Ordering::Relaxed);
}
pub(crate) struct Signals(Vec<(i32, libc::sigaction)>);
impl Signals {
    pub(crate) fn install() -> Result<Self> {
        SIGNAL.store(0, Ordering::Relaxed);
        let mut saved = Self(vec![]);
        for signal in [libc::SIGINT, libc::SIGTERM] {
            let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
            let mut old = unsafe { std::mem::zeroed() };
            action.sa_sigaction = record as *const () as usize;
            unsafe {
                libc::sigemptyset(&mut action.sa_mask);
            }
            if unsafe { libc::sigaction(signal, &action, &mut old) } != 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            saved.0.push((signal, old));
        }
        Ok(saved)
    }
}
impl Drop for Signals {
    fn drop(&mut self) {
        for (signal, old) in &self.0 {
            unsafe {
                libc::sigaction(*signal, old, std::ptr::null_mut());
            }
        }
    }
}
struct Group {
    child: Child,
    armed: bool,
}
impl Group {
    fn signal(&self, signal: i32) {
        unsafe {
            libc::kill(-(self.child.id() as i32), signal);
        }
    }
    fn terminate(&mut self) {
        self.signal(libc::SIGTERM);
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if self.child.try_wait().ok().flatten().is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        // Also reap descendants when the group leader exits first.
        self.signal(libc::SIGKILL);
        let _ = self.child.wait();
        self.armed = false;
    }
}
impl Drop for Group {
    fn drop(&mut self) {
        if self.armed {
            self.terminate();
        }
    }
}
pub(crate) fn execute(command: &mut Command, timeout: Duration) -> Result<i32> {
    command.process_group(0);
    let child = command.spawn().context("start Cargo suite")?;
    let mut group = Group { child, armed: true };
    let deadline = Instant::now() + timeout;
    loop {
        let signal = SIGNAL.load(Ordering::Relaxed);
        if signal != 0 {
            group.terminate();
            return Ok(128 + signal);
        }
        if let Some(status) = group.child.try_wait()? {
            group.signal(libc::SIGKILL);
            group.armed = false;
            return Ok(status
                .code()
                .unwrap_or_else(|| 128 + status.signal().unwrap_or(1)));
        }
        if Instant::now() >= deadline {
            group.terminate();
            return Ok(124);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

pub(crate) fn interrupted() -> Option<i32> {
    let signal = SIGNAL.load(Ordering::Relaxed);
    (signal != 0).then_some(signal)
}
