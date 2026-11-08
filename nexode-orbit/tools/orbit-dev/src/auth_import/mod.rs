//! Synthetic built-CLI auth import matrix; never uses real credentials/providers.
#[cfg(unix)]
mod unix;
use anyhow::Result;
pub fn run(args: impl Iterator<Item = String>) -> Result<()> {
    #[cfg(unix)]
    {
        unix::run(args)
    }
    #[cfg(not(unix))]
    {
        let _ = args;
        anyhow::bail!("auth import permission/PTY acceptance requires Unix")
    }
}
