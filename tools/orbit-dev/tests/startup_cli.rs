#![cfg(target_os = "linux")]
mod support;
use std::process::Command;
#[test]
fn startup_uses_native_fixture_without_external_path_tools_and_enforces_budgets() {
    for fail in [false, true] {
        let mut c = Command::new(support::binary("orbit-dev"));
        c.env_clear()
            .env("PATH", "")
            .env("ORBIT_FIXTURE_MODE", "startup")
            .arg("bench-startup")
            .arg(support::binary("orbit-fixture"))
            .args([
                "--runs",
                "1",
                "--check",
                "--max-help-ms",
                "1000",
                "--max-version-ms",
                "1000",
                "--max-server-ready-ms",
                "1000",
            ]);
        if fail {
            c.args(["--max-cold-total-ms", "1"]);
        }
        let o = c.output().unwrap();
        assert_eq!(
            o.status.code(),
            Some(if fail { 2 } else { 0 }),
            "{}\n{}",
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        );
        assert!(String::from_utf8_lossy(&o.stdout).contains("cold startup total median: 15.0 ms"));
    }
}
