use super::*;
use std::{
    process::{Command, Stdio},
    time::{Duration, Instant},
};
pub(super) struct Container {
    pub(super) engine: String,
    pub(super) name: String,
    pub(super) image: Option<String>,
}
impl Drop for Container {
    fn drop(&mut self) {
        let _ = Command::new(&self.engine)
            .args(["rm", "-f", "--", &self.name])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        if let Some(image) = &self.image {
            let _ = Command::new(&self.engine)
                .args(["image", "rm", "--", image])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }
}
pub(super) fn command(engine: &str, args: &[String], seconds: u64, log: &Path) -> Result<i32> {
    let out = fs::File::create(log)?;
    let err = fs::File::create(log.with_extension("stderr.txt"))?;
    let mut c = Command::new(engine);
    c.args(args).stdin(Stdio::null()).stdout(out).stderr(err);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        c.process_group(0);
    }
    let mut child = c.spawn().context("start container engine")?;
    let end = Instant::now() + Duration::from_secs(seconds);
    loop {
        if let Some(s) = child.try_wait()? {
            return Ok(s.code().unwrap_or(1));
        }
        if Instant::now() >= end {
            #[cfg(unix)]
            unsafe {
                libc::kill(-(child.id() as i32), libc::SIGKILL);
            };
            let _ = child.kill();
            let _ = child.wait();
            return Ok(124);
        }
        std::thread::sleep(Duration::from_millis(30));
    }
}
pub(super) fn ok(engine: &str, args: Vec<String>, seconds: u64, log: &Path) -> Result<()> {
    let n = command(engine, &args, seconds, log)?;
    ensure!(
        n == 0,
        "container operation failed ({n}); see {}",
        log.display()
    );
    Ok(())
}
pub(super) fn words(s: &[&str]) -> Vec<String> {
    s.iter().map(|s| (*s).into()).collect()
}
pub(super) fn cp(c: &Container, from: &Path, to: &str, log: &Path) -> Result<()> {
    ok(
        &c.engine,
        vec![
            "cp".into(),
            from.to_string_lossy().into(),
            format!("{}:{to}", c.name),
        ],
        60,
        log,
    )
}
pub(super) fn shell(c: &Container, script: &str, log: &Path) -> Result<()> {
    ok(
        &c.engine,
        words(&["exec", "--user", "root", &c.name, "sh", "-c", script]),
        60,
        log,
    )
}
