use super::*;
use std::{
    io::Write,
    os::unix::fs::{PermissionsExt, symlink},
};

fn archive(script: &str) -> Vec<u8> {
    let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    let mut tar = tar::Builder::new(encoder);
    let mut header = tar::Header::new_gnu();
    header.set_size(script.len() as u64);
    header.set_mode(0o755);
    header.set_cksum();
    tar.append_data(&mut header, "nexode", script.as_bytes())
        .unwrap();
    tar.into_inner().unwrap().finish().unwrap()
}
fn digest(data: &[u8]) -> String {
    format!("{:x}", Sha256::digest(data))
}
fn run_script(home: &Path, data: &[u8], script: &str) -> std::process::Output {
    let mut child = Command::new("bash")
        .args(["-c", script])
        .env("HOME", home)
        .env(
            "PATH",
            format!(
                "{}:{}",
                home.join("bin").display(),
                std::env::var("PATH").unwrap()
            ),
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(data).unwrap();
    child.wait_with_output().unwrap()
}
fn release(home: &Path, data: &[u8]) -> PathBuf {
    home.join(format!(
        ".local/share/nexode/cloud-releases/runtime-{}",
        digest(data)
    ))
}
fn executable(path: &Path, text: &str) {
    fs::write(path, text).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}
#[test]
fn stage_idempotent_preserves_modified_release_and_lock() {
    let d = tempfile::tempdir().unwrap();
    let home = d.path();
    let data = archive("#!/bin/sh\nexit 0\n");
    let script = install_script(&digest(&data), false).unwrap();
    assert!(!script.contains("sudo"));
    assert!(!script.contains("/opt/nexode-cloud"));
    for _ in 0..2 {
        let out = run_script(home, &data, &script);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let path = release(home, &data).join("nexode");
    fs::write(&path, "changed").unwrap();
    assert!(!run_script(home, &data, &script).status.success());
    assert_eq!(fs::read_to_string(path).unwrap(), "changed");
    let lock = release(home, &data)
        .parent()
        .unwrap()
        .join(".runtime-update-lock");
    fs::create_dir(&lock).unwrap();
    assert!(!run_script(home, &data, &script).status.success());
    assert!(lock.is_dir());
    assert!(fs::read_dir(lock.parent().unwrap()).unwrap().all(|e| {
        !e.unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".upload.")
    }));
}
#[test]
fn invalid_checksum_and_failed_candidate_never_publish() {
    assert!(install_script("'; echo bad", false).is_err());
    for (body, corrupt) in [
        ("#!/bin/sh\nexit 0\n", true),
        ("#!/bin/sh\nexit 42\n", false),
    ] {
        let d = tempfile::tempdir().unwrap();
        let data = archive(body);
        let sum = if corrupt {
            "0".repeat(64)
        } else {
            digest(&data)
        };
        let out = run_script(d.path(), &data, &install_script(&sum, false).unwrap());
        assert!(!out.status.success());
        assert!(!release(d.path(), &data).exists());
    }
}
#[test]
fn activation_retains_old_target_and_rolls_back_failed_probe() {
    for reject in [false, true] {
        let d = tempfile::tempdir().unwrap();
        let home = d.path();
        let old = home.join("old-runtime");
        executable(&old, "#!/bin/sh\nexit 0\n");
        let backend = home.join("backend");
        symlink(&old, &backend).unwrap();
        fs::create_dir(home.join("bin")).unwrap();
        executable(
            &home.join("bin/sudo"),
            "#!/bin/sh\n[ \"$1\" = -n ] && shift\nexec \"$@\"\n",
        );
        let data = archive(if reject {
            "#!/bin/sh\n[ \"$(basename \"$0\")\" != backend ]\n"
        } else {
            "#!/bin/sh\nexit 0\n"
        });
        let script = install_script(&digest(&data), true)
            .unwrap()
            .replace("/opt/nexode-cloud/nexode", backend.to_str().unwrap());
        let out = run_script(home, &data, &script);
        assert_eq!(
            out.status.success(),
            !reject,
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(backend.canonicalize().unwrap() == old, reject);
        assert!(old.exists());
        if reject {
            assert!(String::from_utf8_lossy(&out.stderr).contains("restored previous backend"));
        } else {
            assert!(String::from_utf8_lossy(&out.stdout).contains("Daemon unchanged"));
        }
        let backups: Vec<_> = fs::read_dir(release(home, &data).parent().unwrap())
            .unwrap()
            .filter_map(|e| {
                let e = e.unwrap();
                e.file_name()
                    .to_string_lossy()
                    .starts_with("previous-backend-")
                    .then_some(e.path())
            })
            .collect();
        assert_eq!(backups.len(), 1);
        assert_eq!(
            fs::read_to_string(&backups[0]).unwrap().trim(),
            old.to_str().unwrap()
        );
    }
}
#[test]
fn bundle_executes_without_host_library_paths() {
    let d = tempfile::tempdir().unwrap();
    let (path, sum) = bundle(Path::new("/bin/bash"), d.path()).unwrap();
    assert_eq!(digest(&fs::read(path).unwrap()), sum);
    let out = Command::new(d.path().join("payload/nexode"))
        .args(["-c", "printf 'native bundle\\n'"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.stdout, b"native bundle\n");
}

#[test]
fn upload_rewinds_only_for_staging_transport_failure() {
    for (activate, first_code, expected_code, count) in
        [(false, 255, 0, 2), (false, 1, 1, 1), (true, 255, 255, 1)]
    {
        let d = tempfile::tempdir().unwrap();
        let archive = d.path().join("archive");
        fs::write(&archive, "verified bundle bytes").unwrap();
        let ssh = d.path().join("ssh");
        executable(
            &ssh,
            &format!(
                "#!/bin/sh\nset -eu\nd='{}'\nif [ -f \"$d/upload1\" ]; then cat > \"$d/upload2\"; exit 0; fi\ncat > \"$d/upload1\"\nexit {first_code}\n",
                d.path().display()
            ),
        );
        let args = Args {
            binary: "unused".into(),
            host: "fixture".into(),
            ssh_config: None,
            activate_launcher: activate,
        };
        assert_eq!(
            upload(
                &archive,
                "echo fixture",
                &args,
                Path::new("unused"),
                &ssh,
                Duration::ZERO
            )
            .unwrap(),
            expected_code
        );
        assert_eq!(
            fs::read(d.path().join("upload1")).unwrap(),
            b"verified bundle bytes"
        );
        assert_eq!(d.path().join("upload2").exists(), count == 2);
        if count == 2 {
            assert_eq!(
                fs::read(d.path().join("upload2")).unwrap(),
                b"verified bundle bytes"
            );
        }
    }
}
