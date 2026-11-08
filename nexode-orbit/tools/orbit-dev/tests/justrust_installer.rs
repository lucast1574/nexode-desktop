#![cfg(unix)]
mod support;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
};
use support::*;
struct Fixture {
    temp: tempfile::TempDir,
    stubs: PathBuf,
    install: PathBuf,
    host: &'static str,
    arch: &'static str,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let stubs = temp.path().join("stubs");
        let install = temp.path().join("install space");
        fs::create_dir_all(&stubs).unwrap();
        fs::create_dir_all(&install).unwrap();
        executable(
            stubs.join("uname"),
            "#!/bin/bash\ncase \"$1\" in -s) echo \"$HOST_OS\";; -m) echo \"$HOST_ARCH\";; esac\n",
        );
        executable(
            stubs.join("cygpath"),
            "#!/bin/bash\n[ \"$1\" = -w ] || exit 99\nprintf '%s\\n' 'C:\\fixture\\bin'\n",
        );
        executable(
            stubs.join("curl"),
            "#!/bin/bash\nprintf '%s\\n' \"$*\" >> \"$CURL_LOG\"\nwhile [[ $# -gt 0 ]]; do case \"$1\" in --output) out=$2; shift 2;; *) url=$1; shift;; esac; done\ncp \"$FIXTURES/${url##*/}\" \"$out\"\n",
        );
        Self {
            temp,
            stubs,
            install,
            host: "Linux",
            arch: "x86_64",
        }
    }
    fn archive(&self, target: &str, status: i32, bad: bool, missing: bool) -> String {
        let binary = if target.contains("windows") {
            "justrust.exe"
        } else {
            "justrust"
        };
        let path = self.temp.path().join(format!("justrust-{target}.tar.gz"));
        let gz = flate2::write::GzEncoder::new(
            fs::File::create(&path).unwrap(),
            flate2::Compression::default(),
        );
        let mut tar = tar::Builder::new(gz);
        let data = format!(
            "#!/bin/sh\n[ \"$1\" = --version ] || exit 99\necho justrust-fixture\nexit {status}\n"
        );
        let mut h = tar::Header::new_gnu();
        h.set_size(data.len() as u64);
        h.set_mode(0o755);
        h.set_cksum();
        tar.append_data(
            &mut h,
            format!(
                "justrust-{target}/{}",
                if missing { "other" } else { binary }
            ),
            data.as_bytes(),
        )
        .unwrap();
        // Construct an unselected traversal member without letting the tar builder sanitize it.
        let mut h = tar::Header::new_gnu();
        h.as_mut_bytes()[..10].copy_from_slice(b"../escaped");
        h.set_size(4);
        h.set_mode(0o644);
        h.set_cksum();
        tar.append(&h, &b"oops"[..]).unwrap();
        tar.into_inner().unwrap().finish().unwrap();
        let digest = if bad {
            "0".repeat(64)
        } else {
            format!("{:x}", Sha256::digest(fs::read(&path).unwrap()))
        };
        fs::write(
            path.with_extension("gz.sha256"),
            format!(
                "{digest}  {}\n",
                path.file_name().unwrap().to_str().unwrap()
            ),
        )
        .unwrap();
        binary.into()
    }
    fn command(&self) -> Command {
        let mut cmd = Command::new("bash");
        cmd.arg(root().join("scripts/install_justrust_ci.sh"))
            .env("PATH", path(&self.stubs))
            .env("HOME", self.temp.path())
            .env("JUSTRUST_INSTALL_DIR", &self.install)
            .env("GITHUB_PATH", self.temp.path().join("github_path"))
            .env("JUSTRUST_ALLOW_TEST_URL", "1")
            .env("JUSTRUST_TEST_BASE_URL", "https://trusted.invalid/fixtures")
            .env("FIXTURES", self.temp.path())
            .env("CURL_LOG", self.temp.path().join("curl_log"))
            .env("HOST_OS", self.host)
            .env("HOST_ARCH", self.arch);
        cmd
    }
}
fn good(out: Output) {
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
#[test]
fn all_platform_targets_exact_extraction_and_path_records() {
    let mut f = Fixture::new();
    let mut paths = vec![];
    for (host, arch, target) in [
        ("Linux", "x86_64", "x86_64-unknown-linux-musl"),
        ("Linux", "aarch64", "aarch64-unknown-linux-musl"),
        ("Linux", "arm64", "aarch64-unknown-linux-musl"),
        ("Darwin", "x86_64", "x86_64-apple-darwin"),
        ("Darwin", "arm64", "aarch64-apple-darwin"),
        ("MINGW64_NT-10.0", "x86_64", "x86_64-pc-windows-msvc"),
        ("MINGW64_NT-10.0", "aarch64", "x86_64-pc-windows-msvc"),
        ("MINGW64_NT-10.0", "arm64", "x86_64-pc-windows-msvc"),
    ] {
        f.host = host;
        f.arch = arch;
        let binary = f.archive(target, 0, false, false);
        let out = f.command().output().unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(text.contains("justrust-fixture"));
        assert!(text.contains(target));
        good(out);
        assert!(f.install.join(binary).is_file());
        assert!(!f.temp.path().join("escaped").exists());
        paths.push(if target.contains("windows") {
            r"C:\fixture\bin".to_owned()
        } else {
            f.install.to_str().unwrap().into()
        });
    }
    assert_eq!(
        fs::read_to_string(f.temp.path().join("github_path"))
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        paths
    );
    let log = fs::read_to_string(f.temp.path().join("curl_log")).unwrap();
    assert!(log.contains("--retry 5"));
    assert!(!log.contains("--retry-all-errors"));
}
#[test]
fn rejected_download_and_runtime_validation_preserve_install() {
    for mode in ["checksum", "version", "missing", "malformed"] {
        let f = Fixture::new();
        f.archive(
            "x86_64-unknown-linux-musl",
            if mode == "version" { 7 } else { 0 },
            mode == "checksum",
            mode == "missing",
        );
        if mode == "malformed" {
            fs::write(
                f.temp
                    .path()
                    .join("justrust-x86_64-unknown-linux-musl.tar.gz.sha256"),
                "not-a-digest\n",
            )
            .unwrap();
        }
        fs::write(f.install.join("justrust"), "original").unwrap();
        assert!(!f.command().output().unwrap().status.success(), "{mode}");
        assert_eq!(
            fs::read_to_string(f.install.join("justrust")).unwrap(),
            "original"
        );
        assert!(!f.temp.path().join("github_path").exists());
        assert_eq!(fs::read_dir(&f.install).unwrap().count(), 1);
    }
    for unsupported in [false, true] {
        let mut f = Fixture::new();
        if unsupported {
            f.arch = "riscv64";
        }
        let mut cmd = f.command();
        if !unsupported {
            cmd.env_remove("JUSTRUST_ALLOW_TEST_URL");
        }
        assert!(!cmd.output().unwrap().status.success());
        assert!(!f.temp.path().join("curl_log").exists());
    }
}
#[test]
fn default_location_pinned_url_and_real_shasum_fallback() {
    let f = Fixture::new();
    f.archive("x86_64-unknown-linux-musl", 0, false, false);
    good(
        f.command()
            .env_remove("JUSTRUST_INSTALL_DIR")
            .env_remove("GITHUB_PATH")
            .env_remove("JUSTRUST_TEST_BASE_URL")
            .output()
            .unwrap(),
    );
    assert!(f.temp.path().join(".local/bin/justrust").is_file());
    let log = fs::read_to_string(f.temp.path().join("curl_log")).unwrap();
    assert!(log.contains("/releases/download/v0.1.0-alpha.17/"));
    assert!(!log.contains("/latest/"));
    for name in [
        "bash", "cp", "awk", "tr", "tar", "gzip", "chmod", "mkdir", "mktemp", "mv", "rm", "shasum",
    ] {
        let found = Command::new("sh")
            .args(["-c", "command -v \"$1\"", "lookup", name])
            .output()
            .unwrap();
        assert!(found.status.success());
        std::os::unix::fs::symlink(
            String::from_utf8(found.stdout).unwrap().trim(),
            f.stubs.join(name),
        )
        .unwrap();
    }
    good(f.command().env("PATH", &f.stubs).output().unwrap());
}
