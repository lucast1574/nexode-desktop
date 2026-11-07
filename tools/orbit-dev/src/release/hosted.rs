//! Independent ABI checks for downloaded hosted artifacts; never executes them.
use anyhow::{Context, Result, ensure};
use regex::Regex;
use std::{fs::File, io::Read, path::Path, process::Command, time::Duration};
fn version(value: &str) -> Result<()> {
    let n = value
        .strip_prefix('v')
        .unwrap_or(value)
        .strip_prefix("1.0.0-preview.")
        .context("expected v1.0.0-preview.N")?;
    ensure!(
        !n.starts_with('0')
            && n.bytes().all(|b| b.is_ascii_digit())
            && n.parse::<u64>().is_ok_and(|n| n > 0),
        "preview N must be a positive u64 without leading zeroes"
    );
    Ok(())
}
fn inspect(path: &Path, target: &str) -> Result<()> {
    ensure!(
        matches!(
            target,
            "x86_64-unknown-linux-gnu" | "aarch64-unknown-linux-gnu"
        ),
        "unsupported hosted target"
    );
    let arm = target.starts_with("aarch64");
    let mut header = [0; 64];
    File::open(path)?
        .read_exact(&mut header)
        .context("truncated ELF header")?;
    ensure!(
        &header[..7] == b"\x7fELF\x02\x01\x01",
        "expected little-endian ELF64 executable"
    );
    let kind = u16::from_le_bytes([header[16], header[17]]);
    let machine = u16::from_le_bytes([header[18], header[19]]);
    ensure!(
        matches!(kind, 2 | 3) && machine == if arm { 183 } else { 62 },
        "ELF executable target architecture mismatch"
    );
    let readelf = |option| -> Result<String> {
        let out = crate::process::capture(
            Command::new("readelf")
                .arg(option)
                .arg(path)
                .env("LC_ALL", "C"),
            Duration::from_secs(30),
        )?;
        ensure!(out.status.success(), "readelf {option} failed");
        Ok(String::from_utf8(out.stdout)?)
    };
    let program = readelf("-lW")?;
    ensure!(
        Regex::new(r"(?m)^\s*LOAD\s")?.is_match(&program),
        "ELF has no load segment"
    );
    let loader = if arm {
        "/lib/ld-linux-aarch64.so.1"
    } else {
        "/lib64/ld-linux-x86-64.so.2"
    };
    ensure!(
        program.contains(&format!("[Requesting program interpreter: {loader}]")),
        "unexpected GNU/Linux interpreter"
    );
    let versions = readelf("-VW")?;
    let labels = Regex::new(r"\bName:\s+(GLIBC_[^\s]+)")?;
    let valid = Regex::new(r"^GLIBC_\d+(?:\.\d+)+$")?;
    let mut maximum = vec![];
    for cap in labels.captures_iter(&versions) {
        ensure!(
            valid.is_match(&cap[1]),
            "malformed GLIBC version requirement"
        );
        let parts = cap[1][6..]
            .split('.')
            .map(str::parse::<u64>)
            .collect::<std::result::Result<Vec<_>, _>>()?;
        maximum = maximum.max(parts);
    }
    ensure!(!maximum.is_empty(), "missing GLIBC version requirements");
    let baseline = vec![2, if arm { 39 } else { 17 }];
    ensure!(
        maximum <= baseline,
        "GLIBC requirement {maximum:?} exceeds {baseline:?}"
    );
    ensure!(
        !Regex::new(r"\(NEEDED\).*\[lib(?:ssl|crypto)\.")?.is_match(&readelf("-dW")?),
        "unexpected external OpenSSL dependency"
    );
    if arm {
        ensure!(
            !Regex::new(r"(?m)^\s*TLS\s")?.is_match(&program),
            "ARM64 native PT_TLS violates emulated-TLS contract"
        );
        let symbols = readelf("-sW")?;
        ensure!(
            symbols.contains("__emutls_get_address") && symbols.contains("__emutls_v."),
            "ARM64 emulated-TLS symbols missing"
        );
    }
    println!(
        "Validated hosted {target} artifact: {}, GLIBC max {}",
        path.display(),
        maximum
            .iter()
            .map(u64::to_string)
            .collect::<Vec<_>>()
            .join(".")
    );
    println!(
        "Compilation and ABI checks only. Native runtime, baseline smoke, and packaging remain caller gates."
    );
    Ok(())
}
pub fn run(command: &str, args: impl Iterator<Item = String>) -> Result<()> {
    let a: Vec<_> = args.collect();
    if command == "validate-preview-version" {
        ensure!(a.len() == 1, "expected preview version");
        version(&a[0])
    } else {
        ensure!(a.len() == 2, "expected artifact and target");
        inspect(Path::new(&a[0]), &a[1])
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preview_identity_boundaries() {
        for good in ["v1.0.0-preview.1", "1.0.0-preview.18446744073709551615"] {
            version(good).unwrap();
        }
        for bad in [
            "",
            "v1.0.0-preview.0",
            "v1.0.0-preview.01",
            "v1.0.0-preview.18446744073709551616",
            "v1.1.0-preview.2",
            "v1.0.0-preview.1\n",
            "v1.0.0-preview.+1",
        ] {
            assert!(version(bad).is_err(), "{bad}");
        }
    }
}
