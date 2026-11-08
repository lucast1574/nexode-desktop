//! Linux process-tree PSS/private memory and CPU accounting.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};
fn stat(text: &str) -> Result<(u32, u64)> {
    let fields: Vec<_> = text
        .rsplit_once(") ")
        .context("malformed process stat")?
        .1
        .split_whitespace()
        .collect();
    let parent = fields.get(1).context("missing parent")?.parse()?;
    let user = fields
        .get(11)
        .context("missing user ticks")?
        .parse::<u64>()?;
    let system = fields
        .get(12)
        .context("missing system ticks")?
        .parse::<u64>()?;
    Ok((
        parent,
        user.checked_add(system).context("CPU tick overflow")?,
    ))
}
fn memory(text: &str) -> Result<(f64, f64, f64)> {
    let mut values = BTreeMap::new();
    for line in text.lines() {
        if let Some((key, rest)) = line.split_once(':')
            && ["Pss", "Rss", "Private_Clean", "Private_Dirty"].contains(&key)
        {
            let mut fields = rest.split_whitespace();
            let kb = fields
                .next()
                .context("missing memory value")?
                .parse::<u64>()?;
            ensure!(fields.next() == Some("kB"), "unexpected smaps unit");
            values.insert(key, kb as f64 / 1024.0);
        }
    }
    Ok((
        *values.get("Pss").context("missing PSS")?,
        *values.get("Rss").context("missing RSS")?,
        values.get("Private_Clean").unwrap_or(&0.0) + values.get("Private_Dirty").unwrap_or(&0.0),
    ))
}
pub(super) fn snapshot(roots: &[u32]) -> Result<Value> {
    snapshot_at(
        Path::new("/proc"),
        roots,
        unsafe { libc::sysconf(libc::_SC_CLK_TCK) } as f64,
    )
}
fn snapshot_at(root: &Path, roots: &[u32], tick_hz: f64) -> Result<Value> {
    ensure!(tick_hz > 0.0, "invalid system clock tick rate");
    let mut records = BTreeMap::new();
    let mut children: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for entry in fs::read_dir(root)?.flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        if let Ok(text) = fs::read_to_string(entry.path().join("stat"))
            && let Ok((parent, cpu)) = stat(&text)
        {
            records.insert(pid, cpu);
            children.entry(parent).or_default().push(pid);
        }
    }
    let mut seen = BTreeSet::new();
    let mut stack = roots.to_vec();
    while let Some(pid) = stack.pop() {
        if seen.insert(pid) {
            stack.extend(children.get(&pid).into_iter().flatten().copied());
        }
    }
    let (mut pss, mut rss, mut private, mut cpu, mut processes) = (0.0, 0.0, 0.0, 0.0, 0u64);
    for pid in seen {
        if let Some(ticks) = records.get(&pid)
            && let Ok(text) = fs::read_to_string(root.join(pid.to_string()).join("smaps_rollup"))
            && let Ok((p, r, v)) = memory(&text)
        {
            pss += p;
            rss += r;
            private += v;
            cpu += *ticks as f64 / tick_hz;
            processes += 1;
        }
    }
    Ok(
        json!({"pss_mib":pss,"rss_mib":rss,"private_mib":private,"cpu_seconds":cpu,"processes":processes}),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tree_totals_deduplicate_roots_and_include_descendants() {
        let d = tempfile::tempdir().unwrap();
        for (pid, parent) in [(1, 0), (2, 1), (3, 2), (4, 0)] {
            let p = d.path().join(pid.to_string());
            fs::create_dir(&p).unwrap();
            let mut fields = vec!["0".to_owned(); 20];
            fields[0] = "S".into();
            fields[1] = parent.to_string();
            fields[11] = "100".into();
            fields[12] = "50".into();
            fs::write(
                p.join("stat"),
                format!("{pid} (name ) nested) {}", fields.join(" ")),
            )
            .unwrap();
            fs::write(
                p.join("smaps_rollup"),
                "Pss: 1024 kB\nRss: 2048 kB\nPrivate_Clean: 256 kB\nPrivate_Dirty: 256 kB\n",
            )
            .unwrap();
        }
        let v = snapshot_at(d.path(), &[1, 2], 100.0).unwrap();
        assert_eq!(v["processes"], 3);
        assert_eq!(v["pss_mib"], 3.0);
        assert_eq!(v["private_mib"], 1.5);
        assert_eq!(v["cpu_seconds"], 4.5);
        assert_eq!(snapshot_at(d.path(), &[99], 100.0).unwrap()["processes"], 0);
    }
    #[test]
    fn real_process_has_private_memory_and_missing_pid_is_ignored() {
        let v = snapshot(&[std::process::id()]).unwrap();
        assert!(v["processes"].as_u64().unwrap() >= 1);
        assert!(v["pss_mib"].as_f64().unwrap() > 0.0);
        assert!(v["rss_mib"].as_f64().unwrap() >= v["pss_mib"].as_f64().unwrap());
        assert!(v["pss_mib"].as_f64().unwrap() >= v["private_mib"].as_f64().unwrap());
        assert_eq!(snapshot(&[999999999]).unwrap()["processes"], 0);
        assert!(memory("Pss: 2 MB\nRss: 4 kB").is_err());
    }
}
