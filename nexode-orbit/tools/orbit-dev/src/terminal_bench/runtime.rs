use super::{task::Task, *};
use std::{collections::BTreeMap, io::Write};
pub(super) fn resolve(name: &str, default: &str) -> Result<PathBuf> {
    let p = expand(env(name, default));
    ensure!(p.is_file(), "required file for {name} not found");
    Ok(p)
}
pub(super) fn private_write(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(bytes)?;
    Ok(())
}
pub(super) fn key(name: &str, alias: &str, file_env: &str, default: &str) -> Result<String> {
    for n in [name, alias] {
        if let Ok(s) = std::env::var(n)
            && !s.trim().is_empty()
        {
            return Ok(s.trim().into());
        }
    }
    let raw =
        fs::read_to_string(expand(env(file_env, default))).context("provider key unavailable")?;
    for line in raw
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.starts_with('#'))
    {
        let value = match line.split_once('=') {
            Some((k, v)) if k.trim() == name => v,
            Some(_) => continue,
            None => line,
        };
        let value = value.trim().trim_matches(['\'', '"']);
        if !value.is_empty() {
            return Ok(value.into());
        }
    }
    anyhow::bail!("provider key unavailable")
}
pub(super) fn instruction(t: &Task) -> Result<String> {
    let mut s = env("NEXODE_HARBOR_EXTRA_PREAMBLE", "");
    if std::env::var_os("NEXODE_HARBOR_LEGACY_PREAMBLE").is_some() {
        s = format!("{}\n", include_str!("legacy_preamble.txt"));
    }
    if std::env::var_os("NEXODE_HARBOR_ENABLE_HINTS").is_some()
        && let Ok(p) = std::env::var("NEXODE_HARBOR_TASK_HINTS_FILE")
    {
        let v: Value = serde_json::from_slice(&fs::read(expand(p))?)?;
        if let Some(h) = v[&t.name].as_str() {
            s.push_str("\nAdditional benchmark retry guidance:\n");
            s.push_str(h);
            s.push_str("\n\n");
        }
    }
    s.push_str(&fs::read_to_string(t.path.join("instruction.md"))?);
    Ok(s)
}
pub(super) fn task_env(t: &Task, phase: &str) -> Result<BTreeMap<String, String>> {
    let mut out = BTreeMap::new();
    let re = regex::Regex::new(r"\$\{([A-Za-z_][A-Za-z_0-9]*)(?::-([^}]*))?\}")?;
    if let Some(m) = t
        .config
        .pointer(&format!("/{phase}/env"))
        .and_then(Value::as_object)
    {
        for (k, v) in m {
            ensure!(
                !k.is_empty() && k.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'),
                "invalid environment variable name"
            );
            let mut value = v
                .as_str()
                .context("task environment value must be string")?
                .to_owned();

            let captures = re
                .captures_iter(&value)
                .map(|c| {
                    (
                        c[0].to_owned(),
                        c[1].to_owned(),
                        c.get(2).map(|s| s.as_str().to_owned()),
                    )
                })
                .collect::<Vec<_>>();
            for (full, name, default) in captures {
                value = value.replace(
                    &full,
                    &std::env::var(&name)
                        .ok()
                        .or(default)
                        .with_context(|| format!("missing task environment variable {name}"))?,
                );
            }
            out.insert(k.clone(), value);
        }
    }
    Ok(out)
}
pub(super) fn envfile(dir: &Path, name: &str, vars: &BTreeMap<String, String>) -> Result<PathBuf> {
    let p = dir.join(name);
    let mut s = String::new();
    for (k, v) in vars {
        ensure!(
            !v.contains(['\n', '\r', '\0']),
            "environment value contains a newline or NUL"
        );
        s.push_str(&format!("{k}={v}\n"));
    }
    private_write(&p, s.as_bytes())?;
    Ok(p)
}
pub(super) fn payload(dir: &Path) -> Result<Value> {
    let p = dir.join("result.json");
    if p.is_file() {
        return Ok(serde_json::from_slice(&fs::read(p)?)?);
    }
    let raw = fs::read_to_string(dir.join("events.ndjson"))?;
    let done = raw
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .rfind(|v| v["type"] == "done")
        .context("agent produced no done event")?;
    let result = json!({"session_id":done["session_id"],"provider":done["provider"],"model":done["model"],"text":done["text"],"usage":done.get("usage").cloned().unwrap_or(json!({}))});
    write_json(&p, &result)?;
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn done_payload_and_usage() {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join("events.ndjson"),"noise\n{\"type\":\"done\",\"text\":\"first\"}\n{\"type\":\"done\",\"text\":\"last\",\"usage\":{\"input_tokens\":17}}\n").unwrap();
        let v = payload(d.path()).unwrap();
        assert_eq!(v["text"], "last");
        assert_eq!(v["usage"]["input_tokens"], 17);
        assert!(d.path().join("result.json").exists());
    }
}
