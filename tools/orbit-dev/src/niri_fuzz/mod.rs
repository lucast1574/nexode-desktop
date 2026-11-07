//! Seeded KDL corpus generation checked by the external niri validator.
use anyhow::{Context, Result, ensure};
use clap::Parser;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e3779b97f4a7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        z ^ (z >> 31)
    }
    fn range(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn noise(&mut self) -> String {
        match self.range(10) {
            0 => format!("/* note {} */", "{".repeat((1 + self.range(3)) as usize)),
            1 => format!("/* note {} */", "}".repeat((1 + self.range(3)) as usize)),
            2 => "/* balanced { } */".into(),
            3 => "// comment with {{".into(),
            4 => "// comment with }".into(),
            5 => "/-binds {\n    Mod+T { spawn \"x\"; }\n}".into(),
            6 => "/* /* nested? { */".into(),
            7 => "/*\n multi {\n line }\n*/".into(),
            8 => "// trailing brace at EOL {".into(),
            _ => String::new(),
        }
    }
    fn config(&mut self) -> String {
        let mut parts = vec![];
        for _ in 0..self.range(4) {
            let s = self.noise();
            if !s.is_empty() {
                parts.push(s);
            }
        }
        if self.range(100) < 50 {
            let direction = if self.range(2) == 0 {
                "next-window"
            } else {
                "previous-window"
            };
            parts.push(format!(
                "recent-windows {{\n    binds {{\n        Mod+Tab {{ {direction}; }}\n    }}\n}}"
            ));
        }
        for _ in 0..self.range(3) {
            let s = self.noise();
            if !s.is_empty() {
                parts.push(s);
            }
        }
        if self.range(100) < 85 {
            let body =
                [r#""echo { }""#, r#""echo \" {""#, r##"r#"echo { p\"#"##][self.range(3) as usize];
            parts.push(format!(
                "binds {{\n    Mod+Return {{ spawn \"sh\" \"-c\" {body}; }}\n}}"
            ));
        }
        parts.join("\n") + "\n"
    }
}
fn corpus(
    seed: u64,
    count: u32,
    mut valid: impl FnMut(&str) -> Result<bool>,
) -> Result<Vec<String>> {
    let mut rng = Rng(seed);
    let mut cases = vec![];
    for _ in 0..count {
        let c = rng.config();
        if valid(&c)? {
            cases.push(c);
        }
    }
    Ok(cases)
}
#[derive(Parser)]
struct Args {
    #[arg(default_value_t = 0)]
    seed: u64,
    #[arg(default_value_t = 300)]
    count: u32,
    #[arg(long)]
    output: Option<PathBuf>,
    #[arg(long, default_value = "niri")]
    niri: PathBuf,
}
pub fn run(args: impl Iterator<Item = String>) -> Result<()> {
    let a = Args::parse_from(std::iter::once("fuzz-niri-insert-point".into()).chain(args));
    ensure!(
        a.count > 0 && a.count <= 1_000_000,
        "count must be between 1 and 1000000"
    );
    let output = a.output.unwrap_or(
        dirs::home_dir()
            .context("home unavailable")?
            .join(".nexode/scratch/fuzz719/valid_inputs.json"),
    );
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let scratch = tempfile::tempdir()?;
    let path = scratch.path().join("input.kdl");
    let cases = corpus(a.seed, a.count, |config| {
        fs::write(&path, config)?;
        let result = crate::process::capture(
            Command::new(&a.niri)
                .args(["validate", "--config"])
                .arg(&path),
            Duration::from_secs(10),
        )?;
        Ok(result.status.success())
    })?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer(&mut file, &cases)?;
    file.write_all(b"\n")?;
    file.flush()?;
    file.persist(&output).map_err(|e| e.error)?;
    println!(
        "generated {}, kept {} niri-valid configs",
        a.count,
        cases.len()
    );
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn seeded_corpus_is_reproducible_diverse_and_oracle_filtered() {
        let a = corpus(42, 300, |_| Ok(true)).unwrap();
        let b = corpus(42, 300, |_| Ok(true)).unwrap();
        assert_eq!(a, b);
        assert_ne!(a, corpus(43, 300, |_| Ok(true)).unwrap());
        let joined = a.join("\n");
        for syntax in ["/*", "//", "/-binds", "recent-windows", "r#\"", "\\\""] {
            assert!(joined.contains(syntax), "{syntax}");
        }
        let filtered = corpus(42, 300, |c| Ok(c.contains("recent-windows"))).unwrap();
        assert!(!filtered.is_empty());
        assert!(filtered.len() < a.len());
        assert!(filtered.iter().all(|c| c.contains("recent-windows")));
        assert!(corpus(42, 1, |_| anyhow::bail!("validator failed")).is_err());
    }
}
