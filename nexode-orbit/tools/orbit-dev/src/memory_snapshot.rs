//! Linux proportional memory accounting; never sum shared RSS as unique memory.
use anyhow::{Context, Result, ensure};
use clap::Parser;
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, path::Path};
const KEYS: [(&str, &str); 7] = [
    ("Rss", "rss_mb"),
    ("Pss", "pss_mb"),
    ("Anonymous", "anon_mb"),
    ("Shared_Clean", "shared_clean_mb"),
    ("Private_Clean", "private_clean_mb"),
    ("Private_Dirty", "private_dirty_mb"),
    ("Swap", "swap_mb"),
];
fn rounded(n: f64) -> f64 {
    (n * 10.0).round_ties_even() / 10.0
}
fn option<'a>(argv: &'a [String], name: &str) -> Option<&'a str> {
    argv.iter().enumerate().find_map(|(i, arg)| {
        if arg == name {
            argv.get(i + 1).map(String::as_str)
        } else {
            arg.strip_prefix(&format!("{name}="))
        }
    })
}
fn classify(argv: &[String], socket: &str) -> Option<&'static str> {
    let name = Path::new(argv.first()?).file_name()?.to_str()?;
    if !name.starts_with("nexode") {
        return None;
    }
    let main = option(argv, "--socket").unwrap_or(socket) == socket;
    let mut args = argv.iter().skip(1);
    let mut sub = None;
    while let Some(arg) = args.next() {
        if [
            "--socket",
            "--resume",
            "-C",
            "--chdir",
            "--model",
            "--provider",
        ]
        .contains(&arg.as_str())
        {
            args.next();
        } else if !arg.starts_with('-') {
            sub = Some(arg.as_str());
            break;
        }
    }
    if sub == Some("serve") {
        return Some(if main { "server" } else { "server_aux" });
    }
    let resumed = option(argv, "--resume").is_some_and(|s| s.starts_with("session_"));
    if matches!(sub, Some("auth" | "login" | "logout" | "self-dev")) && !resumed {
        return None;
    }
    if !main {
        Some("client_aux")
    } else if resumed || argv.iter().any(|a| a == "--fresh-spawn") {
        Some("client_session")
    } else {
        Some("client_interactive")
    }
}
fn smaps(text: &str) -> Result<BTreeMap<&'static str, f64>> {
    let mut values = BTreeMap::new();
    for (key, field) in KEYS {
        let value = if let Some(line) = text
            .lines()
            .find_map(|l| l.strip_prefix(&format!("{key}:")))
        {
            let mut parts = line.split_whitespace();
            let kb: u64 = parts.next().context("missing smaps value")?.parse()?;
            ensure!(parts.next() == Some("kB"), "unsupported smaps unit");
            rounded(kb as f64 / 1024.0)
        } else {
            0.0
        };
        values.insert(field, value);
    }
    Ok(values)
}
fn totals(rows: &[Value]) -> Value {
    let mut out = json!({"count": rows.len()});
    for field in [
        "rss_mb",
        "pss_mb",
        "anon_mb",
        "private_dirty_mb",
        "shared_clean_mb",
        "swap_mb",
    ] {
        out[field] = json!(rounded(rows.iter().filter_map(|r| r[field].as_f64()).sum()));
    }
    out
}
fn snapshot(proc_root: &Path, socket: &str, include_aux: bool) -> Result<Value> {
    let mut server = vec![];
    let mut clients = vec![];
    let mut auxiliary = vec![];
    for entry in fs::read_dir(proc_root)? {
        let entry = entry?;
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|s| s.parse::<u32>().ok())
        else {
            continue;
        };
        let Ok(raw) = fs::read(entry.path().join("cmdline")) else {
            continue;
        };
        let argv: Vec<String> = raw
            .split(|b| *b == 0)
            .filter(|s| !s.is_empty())
            .map(|s| String::from_utf8_lossy(s).into_owned())
            .collect();
        let Some(role) = classify(&argv, socket) else {
            continue;
        };
        if role.ends_with("aux") && !include_aux {
            continue;
        }
        let Ok(text) = fs::read_to_string(entry.path().join("smaps_rollup")) else {
            continue;
        };
        if text.trim().is_empty() {
            continue;
        }
        let mut row = serde_json::to_value(
            smaps(&text).with_context(|| format!("invalid smaps for PID {pid}"))?,
        )?;
        row["pid"] = json!(pid);
        row["role"] = json!(role);
        row["cmd"] = json!(argv.join(" "));
        row["session_id"] = json!(option(&argv, "--resume").filter(|s| s.starts_with("session_")));
        row["socket_path"] = json!(option(&argv, "--socket"));
        if role == "server" {
            server.push(row);
        } else if role.ends_with("aux") {
            auxiliary.push(row);
        } else {
            clients.push(row);
        }
    }
    for group in [&mut server, &mut clients, &mut auxiliary] {
        group.sort_by_key(|r| r["pid"].as_u64());
    }
    let primary: Vec<_> = server.iter().chain(&clients).cloned().collect();
    Ok(
        json!({"socket":socket,"totals":{"server":totals(&server),"clients":totals(&clients),"primary":totals(&primary),"auxiliary":totals(&auxiliary)},"server":server,"clients":clients,"auxiliary":auxiliary}),
    )
}
fn human(report: &Value) {
    for (name, key) in [
        ("Server", "server"),
        ("Clients", "clients"),
        ("Auxiliary", "auxiliary"),
    ] {
        let mut rows: Vec<_> = report[key].as_array().unwrap().iter().collect();
        if key == "auxiliary" && rows.is_empty() {
            continue;
        }
        rows.sort_by(|a, b| {
            a["role"]
                .as_str()
                .cmp(&b["role"].as_str())
                .then_with(|| {
                    b["pss_mb"]
                        .as_f64()
                        .unwrap()
                        .total_cmp(&a["pss_mb"].as_f64().unwrap())
                })
                .then_with(|| a["pid"].as_u64().cmp(&b["pid"].as_u64()))
        });
        let t = &report["totals"][key];
        println!(
            "\n{name} ({})\ntotal: RSS {:.1} MB | PSS {:.1} MB | Anon {:.1} MB | Private dirty {:.1} MB",
            rows.len(),
            t["rss_mb"].as_f64().unwrap(),
            t["pss_mb"].as_f64().unwrap(),
            t["anon_mb"].as_f64().unwrap(),
            t["private_dirty_mb"].as_f64().unwrap()
        );
        for r in rows {
            println!(
                "  pid {:<7} {:<48} RSS {:>6.1} | PSS {:>6.1} | Anon {:>6.1} | PrivDirty {:>6.1}",
                r["pid"],
                r["session_id"]
                    .as_str()
                    .unwrap_or(r["role"].as_str().unwrap()),
                r["rss_mb"].as_f64().unwrap(),
                r["pss_mb"].as_f64().unwrap(),
                r["anon_mb"].as_f64().unwrap(),
                r["private_dirty_mb"].as_f64().unwrap()
            );
        }
    }
    let t = &report["totals"]["primary"];
    println!(
        "\nPrimary total (server + clients)\nRSS {:.1} MB | PSS {:.1} MB | Anon {:.1} MB | Private dirty {:.1} MB | Shared clean {:.1} MB\nOvercount if you sum RSS instead of PSS: {:.1} MB",
        t["rss_mb"].as_f64().unwrap(),
        t["pss_mb"].as_f64().unwrap(),
        t["anon_mb"].as_f64().unwrap(),
        t["private_dirty_mb"].as_f64().unwrap(),
        t["shared_clean_mb"].as_f64().unwrap(),
        t["rss_mb"].as_f64().unwrap() - t["pss_mb"].as_f64().unwrap()
    );
}
#[derive(Parser)]
struct Args {
    #[arg(long)]
    json: bool,
    #[arg(long)]
    include_aux: bool,
    #[arg(long)]
    socket: Option<String>,
}
pub fn run(args: impl Iterator<Item = String>) -> Result<()> {
    let args = Args::parse_from(std::iter::once("memory-snapshot".into()).chain(args));
    ensure!(
        cfg!(target_os = "linux"),
        "memory-snapshot requires Linux /proc/smaps_rollup"
    );
    #[cfg(unix)]
    let uid = unsafe { libc::getuid() };
    #[cfg(not(unix))]
    let uid = 0;
    let socket = args
        .socket
        .unwrap_or_else(|| format!("/run/user/{uid}/nexode.sock"));
    let report = snapshot(Path::new("/proc"), &socket, args.include_aux)?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        human(&report);
    }
    Ok(())
}
#[cfg(test)]
mod tests;
