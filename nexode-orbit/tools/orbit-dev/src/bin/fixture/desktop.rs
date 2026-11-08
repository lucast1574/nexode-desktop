//! Fake X11/Desktop programs for offline capture orchestration tests.
use std::{io::Write, path::Path, time::Duration};
pub fn run(name: &str) -> anyhow::Result<()> {
    match name {
        "Xvfb" => {
            println!("91");
            std::io::stdout().flush()?;
        }
        "openbox" => {}
        "nexode-desktop" => {
            anyhow::ensure!(std::env::var("DISPLAY")? == ":91", "wrong private display");
            anyhow::ensure!(
                std::env::var_os("NEXODE_TEST_SECRET").is_none(),
                "inherited credential"
            );
            anyhow::ensure!(
                std::env::var("NEXODE_DESKTOP_SCREENSHOT")? == "1",
                "offline fixture required"
            );
            std::fs::write(
                std::env::var_os("NEXODE_DESKTOP_STATE").unwrap(),
                "widths=1.00\n",
            )?;
        }
        "import" => {
            let output = std::env::args().next_back().unwrap();
            let path = output.strip_prefix("png:").unwrap();
            let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
            png.extend_from_slice(&[0; 32]);
            std::fs::write(Path::new(path), png)?;
            return Ok(());
        }
        "cargo" | "xdotool" => return Ok(()),
        _ => anyhow::bail!("unexpected desktop fixture program"),
    }
    std::thread::sleep(Duration::from_secs(60));
    Ok(())
}
