#![cfg(target_os = "linux")]
mod support;
use std::{fs, path::PathBuf, process::Command};

#[test]
fn heap_churn_creates_both_reports_and_cleans_server_on_failure() {
    for fail in [false, true] {
        let d = tempfile::tempdir().unwrap();
        let server = d.path().join("server");
        support::executable(
            &server,
            &format!(
                "#!/bin/sh\nexec '{}' --heap-server-fixture \"$@\"\n",
                env!("CARGO_BIN_EXE_orbit-fixture")
            ),
        );
        let gdb = d.path().join("gdb");
        support::executable(
            &gdb,
            r#"#!/bin/sh
set -eu
run=$(readlink /proc/$2/cwd)/../run
prof="$run/../prof"
if [ -f "$prof/A.heap" ]; then name=second; else name=first; fi
printf 'heap fixture' > "$prof/$name.heap"
"#,
        );
        let jeprof = d.path().join("jeprof");
        support::executable(
            &jeprof,
            if fail {
                "#!/bin/sh\nexit 7\n"
            } else {
                "#!/bin/sh\nprintf '%s\\n' \"$@\"\n"
            },
        );
        let out = Command::new(env!("CARGO_BIN_EXE_orbit-dev"))
            .args(["heap-diff-idle-sessions", "--binary"])
            .arg(server)
            .arg("--gdb")
            .arg(gdb)
            .arg("--jeprof")
            .arg(jeprof)
            .args(["--sessions", "2", "--rounds", "2", "--settle", "0"])
            .env("REAL_PROVIDER_SECRET", "must-not-inherit")
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.success(), !fail, "{stderr}");
        let root = PathBuf::from(
            stderr
                .lines()
                .find_map(|s| s.strip_prefix("artifacts: "))
                .unwrap(),
        );
        assert!(root.join("prof/A.heap").is_file());
        assert!(root.join("prof/B.heap").is_file());
        if !fail {
            assert!(
                fs::read_to_string(root.join("diff.txt"))
                    .unwrap()
                    .contains("--lines")
            );
            assert!(
                fs::read_to_string(root.join("diff_cum.txt"))
                    .unwrap()
                    .contains("--cum")
            );
        }
        let pid = fs::read_to_string(root.join("run/pid")).unwrap();
        assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists());
        fs::remove_dir_all(root).unwrap();
    }
}
