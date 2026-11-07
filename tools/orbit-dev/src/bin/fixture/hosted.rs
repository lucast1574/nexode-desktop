use anyhow::{Result, ensure};
use std::{fs, io::Write, path::PathBuf, process::Command};
fn env(k: &str) -> String {
    std::env::var(k).unwrap_or_default()
}
fn status(k: &str) -> i32 {
    env(k).parse().unwrap_or(0)
}
pub fn run(name: &str) -> Result<i32> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let record = env("RECORD");
    match name {
        "justrust" => {
            let mut f = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&record)?;
            let allowed = std::env::vars()
                .filter(|(k, _)| {
                    k.starts_with("CARGO_")
                        || k.starts_with("JUSTRUST_")
                        || k.starts_with("NEXODE_BUILD_")
                        || [
                            "NEXODE_RELEASE_BUILD",
                            "RUSTUP_TOOLCHAIN",
                            "RUSTFLAGS",
                            "RUSTC_BOOTSTRAP",
                        ]
                        .contains(&k.as_str())
                })
                .collect::<std::collections::BTreeMap<_, _>>();
            writeln!(f, "{}", serde_json::json!({"args":args,"env":allowed}))?;
            if args == ["remote", "use", "hosted"] {
                return Ok(status("BACKEND_FAILURE"));
            }
            if !Command::new("cargo")
                .args(["metadata", "--format-version", "1", "--offline"])
                .status()?
                .success()
            {
                return Ok(29);
            }
            if !env("BUILD_FAILURE").is_empty() {
                return Ok(23);
            }
            if !env("NO_ARTIFACT").is_empty() {
                return Ok(0);
            }
            let index = args.iter().position(|a| a == "--target").unwrap();
            let target = args[index + 1].trim_end_matches(".2.17");
            let path = PathBuf::from("target").join(target).join("release/nexode");
            fs::create_dir_all(path.parent().unwrap())?;
            let mut header = [0u8; 64];
            header[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
            header[16..18].copy_from_slice(&3u16.to_le_bytes());
            let machine =
                env("MACHINE")
                    .parse::<u16>()
                    .unwrap_or(if target.starts_with("aarch64") {
                        183
                    } else {
                        62
                    });
            header[18..20].copy_from_slice(&machine.to_le_bytes());
            fs::write(
                &path,
                if env("INVALID_ELF").is_empty() {
                    &header[..]
                } else {
                    b"not ELF"
                },
            )?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
            }
        }
        "rustup" => {
            fs::write(format!("{record}.rustup"), serde_json::to_vec(&args)?)?;
            return Ok(status("RUSTUP_FAILURE"));
        }
        "cargo" => {
            ensure!(
                args == ["metadata", "--format-version", "1", "--offline"],
                "unexpected metadata args"
            );
            if env("CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_RUSTFLAGS")
                .contains("-Ztls-model=emulated")
            {
                if env("RUSTUP_TOOLCHAIN") != "nightly-2026-10-08" {
                    eprintln!("error: the option Z is only accepted on the nightly compiler");
                    return Ok(1);
                }
                ensure!(
                    PathBuf::from(format!("{record}.rustup")).exists(),
                    "nightly not installed"
                );
            }
        }
        "readelf" => {
            if !env("READELF_FAILURE").is_empty() {
                return Ok(5);
            }
            let arm = args.last().unwrap().contains("aarch64");
            match args[0].as_str() {
                "-lW" => {
                    println!("  LOAD 0x0 0x0 0x0 0x100 0x100 R E 0x1000");
                    let loader = std::env::var("LOADER").unwrap_or_else(|_| {
                        if arm {
                            "/lib/ld-linux-aarch64.so.1"
                        } else {
                            "/lib64/ld-linux-x86-64.so.2"
                        }
                        .into()
                    });
                    println!("[Requesting program interpreter: {loader}]");
                    if !env("NATIVE_TLS").is_empty() {
                        println!("  TLS 0x0 0x0 0x0 0x10 0x10 R 0x8");
                    }
                }
                "-VW" => {
                    let version = std::env::var("GLIBC_VERSION")
                        .unwrap_or_else(|_| if arm { "2.39" } else { "2.17" }.into());
                    if !version.is_empty() {
                        println!("0x0010: Name: GLIBC_{version} Flags: none Version: 2");
                    }
                }
                "-dW" => {
                    println!("0x00000001 (NEEDED) Shared library: [libc.so.6]");
                    if !env("DYNAMIC_OPENSSL").is_empty() {
                        println!("0x00000001 (NEEDED) Shared library: [libssl.so.3]");
                    }
                }
                "-sW" => {
                    if env("NO_EMUTLS").is_empty() {
                        println!("__emutls_get_address@GCC_4.3.0 __emutls_v._ZNfixture");
                    }
                }
                _ => return Ok(4),
            }
        }
        _ => anyhow::bail!("unknown hosted fixture command"),
    }
    Ok(0)
}
