#![allow(dead_code)]
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};
pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
pub fn read(path: &str) -> String {
    fs::read_to_string(root().join(path)).unwrap()
}
pub fn between<'a>(text: &'a str, start: &str, end: &str) -> &'a str {
    text.split_once(start)
        .unwrap_or_else(|| panic!("missing {start}"))
        .1
        .split_once(end)
        .unwrap_or_else(|| panic!("missing {end}"))
        .0
}
pub fn write(path: impl AsRef<Path>, text: &str) {
    let p = path.as_ref();
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, text).unwrap();
}
pub fn executable(path: impl AsRef<Path>, text: &str) {
    write(&path, text);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
}
pub fn shell(script: &str, dir: &Path) -> Command {
    let mut cmd = Command::new("/bin/bash");
    cmd.args(["-eu", "-c", script]).current_dir(dir);
    for (k, _) in std::env::vars_os() {
        if k == "BASH_ENV" || k.to_string_lossy().starts_with("BASH_FUNC_") {
            cmd.env_remove(k);
        }
    }
    cmd
}
pub fn path(dir: &Path) -> std::ffi::OsString {
    let mut paths = vec![dir.to_owned()];
    paths.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    std::env::join_paths(paths).unwrap()
}
pub fn success(out: Output) -> String {
    assert!(
        out.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}
pub fn dedent(text: &str) -> String {
    let n = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.len() - l.trim_start().len())
        .min()
        .unwrap_or(0);
    text.lines()
        .map(|l| if l.len() >= n { &l[n..] } else { "" })
        .collect::<Vec<_>>()
        .join("\n")
}
// Cargo test builds sibling binaries before integration tests. Clippy does not
// populate CARGO_BIN_EXE, so resolve at test runtime as a fallback.
pub fn binary(name: &str) -> std::path::PathBuf {
    if let Some(path) = std::env::var_os(format!("CARGO_BIN_EXE_{name}")) {
        return path.into();
    }
    let exe = std::env::current_exe().unwrap();
    let path = exe
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    assert!(path.is_file(), "Cargo did not build {}", path.display());
    path
}
