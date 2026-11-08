//! Native ARM runtime gate consumes the exact uploaded archive, never a rebuild.
use anyhow::{Context, Result, ensure};
use std::{
    fs,
    io::{Read, Write},
    path::Path,
    process::Command,
    time::Duration,
};
const MEMBER: &str = "nexode-linux-aarch64";
fn version_matches(output: &str, version: &str) -> bool {
    let expected = version.trim_start_matches('v');
    output
        .split(|c: char| !(c.is_alphanumeric() || c == '.' || c == '-' || c == '_'))
        .any(|s| s.trim_start_matches('v') == expected)
}
fn verify(
    archive: &Path,
    version: &str,
    mut execute: impl FnMut(&Path, &[&str]) -> Result<String>,
) -> Result<()> {
    ensure!(
        super::notes::number(version).is_some(),
        "expected explicit preview version"
    );
    let temp = tempfile::tempdir()?;
    let executable = temp.path().join(MEMBER);
    let mut package = tar::Archive::new(flate2::read::GzDecoder::new(fs::File::open(archive)?));
    let mut entries = package.entries()?;
    let mut member = entries.next().context("missing archive member")??;
    ensure!(
        member.path()?.as_ref() == Path::new(MEMBER)
            && member.header().entry_type().is_file()
            && member.header().mode()? & 0o111 != 0,
        "expected exactly one executable ARM archive member"
    );
    let mut header = [0u8; 64];
    member
        .read_exact(&mut header)
        .context("expected AArch64 ELF64 executable")?;
    ensure!(
        &header[..7] == b"\x7fELF\x02\x01\x01"
            && matches!(u16::from_le_bytes([header[16], header[17]]), 2 | 3)
            && u16::from_le_bytes([header[18], header[19]]) == 183,
        "expected AArch64 ELF64 executable"
    );
    let mut file = fs::File::create(&executable)?;
    file.write_all(&header)?;
    std::io::copy(&mut member, &mut file)?;
    drop(member);
    drop(file);
    ensure!(
        entries.next().is_none(),
        "expected exactly one executable ARM archive member"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755))?;
    }
    let output = execute(&executable, &["--no-update", "--no-selfdev", "--version"])?;
    print!("{output}");
    ensure!(
        version_matches(&output, version),
        "uploaded ARM archive has wrong preview version"
    );
    for command in [
        "receive",
        "credentials",
        "router-credentials",
        "cancel-return",
    ] {
        execute(
            &executable,
            &["--no-update", "--no-selfdev", "cloud", command, "--help"],
        )?;
    }
    println!(
        "Native ARM runtime acceptance passed for {}",
        archive.display()
    );
    Ok(())
}
pub fn run(mut args: impl Iterator<Item = String>) -> Result<()> {
    ensure!(
        cfg!(all(target_os = "linux", target_arch = "aarch64")),
        "ARM runtime acceptance requires native ARM64 Linux"
    );
    let archive = args
        .next()
        .context("usage: verify-preview-arm-runtime ARCHIVE")?;
    ensure!(args.next().is_none(), "unexpected arguments");
    let version = std::env::var("NEXODE_BUILD_SEMVER").context("NEXODE_BUILD_SEMVER required")?;
    verify(Path::new(&archive), &version, |binary, args| {
        let out =
            crate::process::capture(Command::new(binary).args(args), Duration::from_secs(60))?;
        ensure!(out.status.success(), "ARM runtime smoke failed: {:?}", args);
        Ok(String::from_utf8(out.stdout)?)
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    fn archive(
        mode: u32,
        kind: tar::EntryType,
        extra: bool,
        wrong: bool,
    ) -> (tempfile::TempDir, Vec<u8>) {
        let tmp = tempfile::tempdir().unwrap();
        let mut bytes = vec![0u8; 64];
        bytes[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
        bytes[16] = 3;
        bytes[18] = if wrong { 62 } else { 183 };
        let file = fs::File::create(tmp.path().join("arm.tar.gz")).unwrap();
        let gz = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        let mut tar = tar::Builder::new(gz);
        let mut header = tar::Header::new_gnu();
        header.set_path(MEMBER).unwrap();
        header.set_size(bytes.len() as u64);
        header.set_mode(mode);
        header.set_entry_type(kind);
        header.set_cksum();
        tar.append(&header, &bytes[..]).unwrap();
        if extra {
            let mut header = tar::Header::new_gnu();
            header.set_size(0);
            header.set_mode(0o644);
            header.set_cksum();
            tar.append_data(&mut header, "unexpected", &[][..]).unwrap();
        }
        tar.into_inner().unwrap().finish().unwrap();
        (tmp, bytes)
    }
    #[test]
    fn exact_bytes_all_smokes_and_failures() {
        let (temp, bytes) = archive(0o755, tar::EntryType::Regular, false, false);
        let path = temp.path().join("arm.tar.gz");
        let mut calls = vec![];
        verify(&path, "v1.0.0-preview.12", |bin, args| {
            assert_eq!(fs::read(bin).unwrap(), bytes);
            calls.push(args.iter().map(|s| s.to_string()).collect::<Vec<_>>());
            Ok("nexode v1.0.0-preview.12 (abc123)\n".into())
        })
        .unwrap();
        assert_eq!(calls.len(), 5);
        assert_eq!(calls[0], ["--no-update", "--no-selfdev", "--version"]);
        assert_eq!(
            calls[4],
            [
                "--no-update",
                "--no-selfdev",
                "cloud",
                "cancel-return",
                "--help"
            ]
        );
        for failure_at in 0..5 {
            let mut calls = 0;
            assert!(
                verify(&path, "v1.0.0-preview.12", |_, _| {
                    let current = calls;
                    calls += 1;
                    ensure!(current != failure_at, "fake failure");
                    Ok("nexode v1.0.0-preview.12".into())
                })
                .is_err()
            );
            assert_eq!(calls, failure_at + 1);
        }
        for output in [
            "nexode v1.0.0-preview.123",
            "nexode v1.0.0-preview.12-extra",
            "nexode 0.99.1",
        ] {
            let mut calls = 0;
            assert!(
                verify(&path, "v1.0.0-preview.12", |_, _| {
                    calls += 1;
                    Ok(output.into())
                })
                .is_err()
            );
            assert_eq!(calls, 1);
        }
    }
    #[test]
    fn rejects_wrong_arch_links_extra_and_nonexecutable_before_execution() {
        for (mode, kind, extra, wrong) in [
            (0o644, tar::EntryType::Regular, false, false),
            (0o755, tar::EntryType::Symlink, false, false),
            (0o755, tar::EntryType::Link, false, false),
            (0o755, tar::EntryType::Regular, true, false),
            (0o755, tar::EntryType::Regular, false, true),
        ] {
            let (temp, _) = archive(mode, kind, extra, wrong);
            assert!(
                verify(
                    &temp.path().join("arm.tar.gz"),
                    "v1.0.0-preview.12",
                    |_, _| panic!("must not execute")
                )
                .is_err()
            );
        }
        if !cfg!(all(target_os = "linux", target_arch = "aarch64")) {
            assert!(
                run(["missing.tar.gz".into()].into_iter())
                    .unwrap_err()
                    .to_string()
                    .contains("native ARM64")
            );
        }
    }
}
