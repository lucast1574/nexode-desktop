//! Shared, test-only entry point for native subprocess fixtures.
//! This independent workspace avoids recursively locking the application's target.
use std::path::PathBuf;
use std::sync::OnceLock;
pub fn binary() -> &'static PathBuf {
    static BINARY: OnceLock<PathBuf> = OnceLock::new();
    BINARY.get_or_init(|| {
        if let Some(path) = std::env::var_os("ORBIT_TEST_FIXTURE_BIN") {
            let path = PathBuf::from(path)
                .canonicalize()
                .expect("native fixture override");
            assert!(path.is_file(), "native fixture override is not a file");
            return path;
        }
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .find(|p| p.join("tools/orbit-dev/Cargo.toml").is_file())
            .expect("repository root containing native fixtures")
            .to_owned();
        let target = root.join("tools/orbit-dev/target");
        let status =
            std::process::Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
                .args([
                    "build",
                    "--quiet",
                    "--locked",
                    "--bin",
                    "orbit-fixture",
                    "--manifest-path",
                ])
                .arg(root.join("tools/orbit-dev/Cargo.toml"))
                .arg("--target-dir")
                .arg(&target)
                .env("RUSTC_WRAPPER", "")
                .env("CARGO_BUILD_RUSTC_WRAPPER", "")
                .status()
                .expect("build native test fixture");
        assert!(
            status.success(),
            "native test fixture build failed: {status}"
        );
        target
            .join("debug")
            .join(format!("orbit-fixture{}", std::env::consts::EXE_SUFFIX))
    })
}
