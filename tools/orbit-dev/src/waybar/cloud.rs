use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{fs, path::Path};
fn pid_live(path: &Path) -> bool {
    let Some(pid) = fs::read_to_string(path)
        .ok()
        .and_then(|s| s.trim().parse::<i32>().ok())
        .filter(|n| *n > 0)
    else {
        return false;
    };
    #[cfg(unix)]
    {
        unsafe {
            libc::kill(pid, 0) == 0
                || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
        }
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        false
    }
}
pub(super) fn probe(root: &Path) -> Value {
    match counts(root, pid_live) {
        Ok((streaming, alive)) => json!({"streaming":streaming,"alive":alive}),
        Err(_) => unknown("cloud presence registry unreadable"),
    }
}
fn counts(root: &Path, live: impl Fn(&Path) -> bool) -> Result<(u64, u64)> {
    let active = root.join("active_pids");
    if !active.exists() {
        return Ok((0, 0));
    }
    let mut alive = 0;
    let mut streaming = 0;
    for entry in fs::read_dir(active)? {
        let p = entry?.path();
        if live(&p) {
            alive += 1;
            if live(&root.join("streaming_pids").join(p.file_name().unwrap())) {
                streaming += 1;
            }
        }
    }
    Ok((streaming, alive))
}
pub(super) fn unknown(detail: &str) -> Value {
    json!({"streaming":null,"alive":null,"detail":detail})
}
fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}
pub(super) fn query(
    home: &Path,
    remote_binary: &str,
    mut run: impl FnMut(&[String]) -> Result<String>,
) -> Value {
    let inner = || -> Result<Value> {
        let binary = home
            .join(".local/bin/nexode")
            .to_string_lossy()
            .into_owned();
        let args = |items: &[&str]| items.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let status = run(&args(&[
            &binary,
            "--no-update",
            "--no-selfdev",
            "cloud",
            "machine",
            "status",
        ]))?;
        let offline = ["`: stopped", "`: sleeping", "`: not provisioned"]
            .iter()
            .any(|s| status.contains(s));
        ensure!(
            offline || status.contains("`: running"),
            "machine state unavailable"
        );
        let root = home.join(".nexode/cloud");
        let mut hosts = vec![];
        if !offline {
            hosts.push((root.join("ssh_config"), "nexode-cloud".into()));
        }
        let inventory: Value = serde_json::from_str(&run(&args(&[
            &binary,
            "--no-update",
            "--no-selfdev",
            "cloud",
            "ephemeral",
            "list",
            "--json",
        ]))?)?;
        for machine in inventory.as_array().context("invalid inventory")? {
            match machine["state"].as_str().context("missing machine state")? {
                "running" => {
                    let id = machine["id"].as_str().context("machine id missing")?;
                    ensure!(
                        id.strip_prefix("cm_").is_some_and(|tail| !tail.is_empty()
                            && tail.bytes().all(|b| b.is_ascii_alphanumeric())),
                        "invalid machine id"
                    );
                    hosts.push((
                        root.join("ephemeral").join(id).join("ssh_config"),
                        format!("nexode-ephemeral-{id}"),
                    ));
                }
                "terminated" | "stopped" | "sleeping" | "expired" => {}
                _ => anyhow::bail!("machine transitioning"),
            }
        }
        let mut alive = 0u64;
        let mut streaming = 0u64;
        for (config, host) in hosts {
            let command = format!("{} waybar-status --cloud-probe", quote(remote_binary));
            let counts: Value = serde_json::from_str(&run(&args(&[
                "ssh",
                "-F",
                &config.to_string_lossy(),
                "-T",
                "-o",
                "BatchMode=yes",
                "-o",
                "ConnectTimeout=5",
                "-o",
                "PermitLocalCommand=no",
                "-o",
                "ClearAllForwardings=yes",
                &host,
                &command,
            ]))?)?;
            let a = counts["alive"]
                .as_u64()
                .context("cloud population unavailable")?;
            let s = counts["streaming"]
                .as_u64()
                .context("cloud population unavailable")?;
            ensure!(s <= a, "invalid population");
            alive = alive.checked_add(a).context("population overflow")?;
            streaming = streaming.checked_add(s).context("population overflow")?;
        }
        Ok(
            json!({"streaming":streaming,"alive":alive,"detail":"managed + ephemeral cloud, including detached and swarm agents"}),
        )
    };
    let mut inner = inner;
    inner().unwrap_or_else(|_| {
        unknown("cloud status unavailable (requires native orbit-dev on running hosts)")
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn counts_sessions_instead_of_shared_pids_and_ignores_stale_markers() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir(root.join("active_pids")).unwrap();
        fs::create_dir(root.join("streaming_pids")).unwrap();
        for name in ["root", "worker"] {
            fs::write(
                root.join("active_pids").join(name),
                std::process::id().to_string(),
            )
            .unwrap();
        }
        fs::write(
            root.join("streaming_pids/worker"),
            std::process::id().to_string(),
        )
        .unwrap();
        for bad in ["0", "-1", "2147483648", "invalid"] {
            fs::write(root.join("active_pids").join(bad), bad).unwrap();
        }
        fs::write(
            root.join("streaming_pids/orphan"),
            std::process::id().to_string(),
        )
        .unwrap();
        assert_eq!(probe(root), json!({"streaming":1,"alive":2}));
        assert_eq!(counts(root, |_| false).unwrap(), (0, 0));
    }
    #[test]
    fn offline_never_connects_and_running_hosts_sum_validated_counts() {
        let home = Path::new("/fixture");
        let mut calls = vec![];
        let r = query(home, "orbit-dev", |args| {
            calls.push(args.to_vec());
            Ok(if args.iter().any(|a| a == "ephemeral") {
                "[]"
            } else {
                "Nexode Cloud machine `nexode-cloud`: stopped"
            }
            .into())
        });
        assert_eq!(r["alive"], 0);
        assert_eq!(calls.len(), 2);
        assert!(calls.iter().all(|a| a[0] != "ssh"));
        let mut ssh = 0;
        let r = query(home, "/opt/orbit dev", |a| {
            if a[0] == "ssh" {
                ssh += 1;
                assert_eq!(
                    a.last().unwrap(),
                    "'/opt/orbit dev' waybar-status --cloud-probe"
                );
                Ok("{\"streaming\":2,\"alive\":5}".into())
            } else if a.iter().any(|a| a == "ephemeral") {
                Ok(r#"[{"id":"cm_abc123","state":"running"},{"id":"cm_dead","state":"terminated"}]"#.into())
            } else {
                Ok("Nexode Cloud machine `nexode-cloud`: running".into())
            }
        });
        assert_eq!(ssh, 2);
        assert_eq!(r["streaming"], 4);
        assert_eq!(r["alive"], 10);
        assert!(query(home, "orbit-dev", |_| anyhow::bail!("timeout"))["alive"].is_null());
    }
}
