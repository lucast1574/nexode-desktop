#![cfg(unix)]
mod support;
use serde_json::Value;
use std::{fs, process::Command};
#[test]
fn real_child_exit_preserves_source_and_success_reports_each_run() {
    for fail in [false, true] {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("source.rs");
        fs::write(&path, "original").unwrap();
        let bin = d.path().join("bin");
        fs::create_dir(&bin).unwrap();
        support::executable(
            bin.join("cargo"),
            &format!(
                "#!/bin/sh\nset -eu\n[ \"$1\" = check ]\nprintf run >> calls\n[ \"$(cat source.rs)\" = original ]\nexit {}\n",
                if fail { 19 } else { 0 }
            ),
        );
        let out = Command::new(env!("CARGO_BIN_EXE_orbit-dev"))
            .args(["bench-compile", "check", "--repo"])
            .arg(d.path())
            .args(["--edit", "source.rs", "--runs", "2", "--json"])
            .env("PATH", support::path(&bin))
            .output()
            .unwrap();
        assert_eq!(
            out.status.code(),
            Some(if fail { 19 } else { 0 }),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(fs::read(&path).unwrap(), b"original");
        if !fail {
            let v: Value = serde_json::from_slice(&out.stdout).unwrap();
            assert_eq!(v["times_seconds"].as_array().unwrap().len(), 2);
            assert_eq!(v["edit"], "source.rs");
            assert_eq!(
                fs::read_to_string(d.path().join("calls")).unwrap(),
                "runrun"
            );
        }
    }
}
