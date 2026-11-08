#![cfg(unix)]
mod support;
use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
};
use support::*;
struct Fixture {
    temp: tempfile::TempDir,
    binary: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::Builder::new()
            .prefix("orbit hosted package ")
            .tempdir()
            .unwrap();
        let r = temp.path();
        fs::create_dir(r.join("scripts")).unwrap();
        fs::copy(
            root().join("scripts/package_preview_linux_hosted.sh"),
            r.join("scripts/package_preview_linux_hosted.sh"),
        )
        .unwrap();
        let binary = r.join("target/x86_64-unknown-linux-gnu/release/nexode");
        executable(
            &binary,
            "#!/bin/sh\nprintf \"arg=<%s>\\n\" \"$@\"\nprintf \"libs=<%s>\\n\" \"$LD_LIBRARY_PATH\"\n",
        );
        executable(
            r.join("tools/docker"),
            "#!/bin/sh\nprintf \"%s\\n\" \"$@\" > \"$DOCKER_LOG\"\nexit \"${DOCKER_STATUS:-0}\"\n",
        );
        Self { temp, binary }
    }
    fn run(&self, args: &[&str], status: &str) -> Output {
        Command::new("bash")
            .arg(
                self.temp
                    .path()
                    .join("scripts/package_preview_linux_hosted.sh"),
            )
            .args(args)
            .env("PATH", path(&self.temp.path().join("tools")))
            .env("DOCKER_LOG", self.temp.path().join("docker.log"))
            .env("DOCKER_STATUS", status)
            .output()
            .unwrap()
    }
}
#[test]
fn exact_archive_layout_and_wrapper_arguments() {
    use std::io::Read;
    let f = Fixture::new();
    success(f.run(&[], "0"));
    let r = f.temp.path();
    let gzip = flate2::read::GzDecoder::new(
        fs::File::open(r.join("dist/nexode-linux-x86_64.tar.gz")).unwrap(),
    );
    let mut tar = tar::Archive::new(gzip);
    let mut names = Vec::new();
    for entry in tar.entries().unwrap() {
        let mut entry = entry.unwrap();
        let name = entry.path().unwrap().into_owned();
        if name == std::path::Path::new("nexode-linux-x86_64.bin") {
            let mut data = Vec::new();
            entry.read_to_end(&mut data).unwrap();
            assert_eq!(data, fs::read(&f.binary).unwrap());
        }
        names.push(name);
    }
    assert_eq!(
        names,
        vec![
            PathBuf::from("nexode-linux-x86_64"),
            PathBuf::from("nexode-linux-x86_64.bin")
        ]
    );
    std::os::unix::fs::symlink(r.join("dist/nexode-linux-x86_64"), r.join("alias")).unwrap();
    let out = success(
        Command::new(r.join("alias"))
            .args(["--version", "space argument"])
            .env("LD_LIBRARY_PATH", "/existing")
            .output()
            .unwrap(),
    );
    assert!(out.contains("arg=<space argument>"));
    assert!(out.contains(&format!("libs=<{}/dist:/existing>", r.display())));
    assert!(!fs::read_dir(r.join("dist")).unwrap().any(|p| {
        p.unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".hosted-linux-package.")
    }));
}
#[test]
fn container_is_pinned_sandboxed_and_contains_all_smokes() {
    let f = Fixture::new();
    success(f.run(&[], "0"));
    let text = fs::read_to_string(f.temp.path().join("docker.log")).unwrap();
    let args: Vec<_> = text.lines().collect();
    for item in [
        "--read-only",
        "--cap-drop",
        "ALL",
        "--network",
        "none",
        "no-new-privileges",
        "65534:65534",
    ] {
        assert!(args.contains(&item), "{item}");
    }
    assert!(
        args.iter()
            .any(|s| s.contains("manylinux2014_x86_64@sha256:"))
    );
    assert!(
        args.iter()
            .any(|s| s.contains("dst=/input/nexode,readonly"))
    );
    for c in [
        "receive",
        "credentials",
        "router-credentials",
        "cancel-return",
    ] {
        assert!(text.contains(c));
    }
    assert!(!text.contains("NEXODE_API_KEY"));
}
#[test]
fn failed_baseline_does_not_package() {
    let f = Fixture::new();
    assert_eq!(f.run(&[], "42").status.code(), Some(42));
    assert!(!f.temp.path().join("dist").exists());
}
#[test]
fn missing_executable_and_extra_arguments_fail_closed() {
    let f = Fixture::new();
    fs::remove_file(&f.binary).unwrap();
    assert!(!f.run(&[], "0").status.success());
    assert!(!f.run(&["unexpected"], "0").status.success());
    assert!(!f.temp.path().join("docker.log").exists());
}
