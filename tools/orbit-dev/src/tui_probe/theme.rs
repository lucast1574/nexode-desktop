use super::isolated::{Session, pause};
use anyhow::{Context, Result, ensure};
use clap::Parser;
use std::{collections::BTreeSet, path::PathBuf};
#[derive(Default)]
pub(super) struct Colors {
    tail: Vec<u8>,
    pub values: BTreeSet<[u8; 3]>,
}
impl Colors {
    pub fn feed(&mut self, bytes: &[u8]) -> Result<()> {
        static RGB: std::sync::OnceLock<regex::bytes::Regex> = std::sync::OnceLock::new();
        let re = RGB.get_or_init(|| {
            regex::bytes::Regex::new(r"38;2;(\d{1,3});(\d{1,3});(\d{1,3})[;m]").unwrap()
        });
        self.tail.extend_from_slice(bytes);
        for c in re.captures_iter(&self.tail) {
            let mut rgb = [0; 3];
            let mut valid = true;
            for i in 0..3 {
                match std::str::from_utf8(&c[i + 1])?.parse::<u8>() {
                    Ok(v) => rgb[i] = v,
                    Err(_) => valid = false,
                }
            }
            if valid {
                self.values.insert(rgb);
            }
        }
        ensure!(self.values.len() <= 65536, "too many distinct SGR colors");
        if self.tail.len() > 64 {
            self.tail.drain(..self.tail.len() - 64);
        }
        Ok(())
    }
}
fn validate(dark: &BTreeSet<[u8; 3]>, light: &BTreeSet<[u8; 3]>) -> Result<()> {
    ensure!(
        dark.len() >= 3 && light.len() >= 3,
        "not enough truecolor samples; TUI may not have rendered"
    );
    ensure!(
        dark.contains(&[80, 80, 80]),
        "dark run missing native dim color (80,80,80)"
    );
    ensure!(
        light.contains(&[71, 71, 71]),
        "light run missing readable dim color (71,71,71)"
    );
    ensure!(
        !light.contains(&[80, 80, 80]),
        "light run still emits native dark-theme dim color"
    );
    let min = light
        .iter()
        .map(|[r, g, b]| {
            (0.299 * f64::from(*r) + 0.587 * f64::from(*g) + 0.114 * f64::from(*b)) / 255.0
        })
        .min_by(f64::total_cmp)
        .unwrap();
    ensure!(min <= 0.25, "light run has no dark accents");
    Ok(())
}
#[derive(Parser)]
struct Args {
    #[arg(long)]
    binary: Option<PathBuf>,
    #[arg(long, default_value_t = 12.0)]
    seconds: f64,
}
pub(super) fn run(args: impl Iterator<Item = String>) -> Result<i32> {
    let a = Args::parse_from(std::iter::once("verify-light-theme".into()).chain(args));
    ensure!(
        a.seconds.is_finite() && a.seconds > 0.0 && a.seconds <= 3600.0,
        "invalid capture duration"
    );
    let binary = a
        .binary
        .or_else(|| {
            std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".nexode/builds/current/nexode"))
        })
        .context("provide --binary or HOME")?
        .canonicalize()?;
    let mut captures = vec![];
    for light in [false, true] {
        println!(
            "capturing with {} background reply",
            if light { "white" } else { "black" }
        );
        let session = Session::with_palette(&binary, light)?;
        pause(a.seconds)?;
        let snapshot = session.client.snapshot()?;
        ensure!(!snapshot.eof, "client exited before capture completed");
        println!(
            "{} distinct truecolor foreground colors",
            snapshot.colors.len()
        );
        captures.push(snapshot.colors);
    }
    validate(&captures[0], &captures[1])?;
    println!("PASS: white background detected; muted light-theme text has readable contrast");
    Ok(0)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fragmented_sgr_and_invalid_channels_are_handled() {
        let mut c = Colors::default();
        c.feed(b"\x1b[38;2;71;").unwrap();
        c.feed(b"71;71mtext\x1b[38;2;999;0;0m\x1b[38;2;1;2;3;1m")
            .unwrap();
        assert_eq!(c.values, BTreeSet::from([[71, 71, 71], [1, 2, 3]]));
        assert!(c.tail.len() <= 64);
    }
    #[test]
    fn contrast_contract_rejects_missing_samples_and_unrepaired_dim() {
        let dark = BTreeSet::from([[80, 80, 80], [255, 255, 255], [200, 100, 100]]);
        let mut light = BTreeSet::from([[71, 71, 71], [30, 30, 30], [100, 50, 50]]);
        validate(&dark, &light).unwrap();
        light.insert([80, 80, 80]);
        assert!(validate(&dark, &light).is_err());
        assert!(validate(&BTreeSet::new(), &light).is_err());
    }
}
