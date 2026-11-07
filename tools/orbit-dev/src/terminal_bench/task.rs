use super::*;
#[derive(Clone)]
pub(super) struct Task {
    pub path: PathBuf,
    pub name: String,
    pub config: Value,
}
impl Task {
    pub fn load(path: &Path) -> Result<Self> {
        let path = path.canonicalize()?;
        let name = path
            .file_name()
            .context("task name")?
            .to_string_lossy()
            .into_owned();
        safe_name(&name)?;
        let t: toml::Value = toml::from_str(&fs::read_to_string(path.join("task.toml"))?)?;
        let v = serde_json::to_value(t)?;
        for p in [
            "/steps",
            "/verifier/environment",
            "/verifier/collect",
            "/environment/mcp_servers",
            "/environment/skills_dir",
            "/environment/healthcheck",
            "/environment/tpu",
            "/artifacts",
        ] {
            ensure!(
                v.pointer(p)
                    .is_none_or(|x| x.is_null() || x.as_array().is_some_and(Vec::is_empty)),
                "native runner does not support task setting {p}; refusing to silently change benchmark semantics"
            );
        }
        ensure!(
            v.pointer("/verifier/environment_mode")
                .and_then(Value::as_str)
                .is_none_or(|x| x == "shared"),
            "separate verifier environments are unsupported"
        );
        ensure!(
            v.pointer("/environment/os")
                .and_then(Value::as_str)
                .is_none_or(|x| x == "linux"),
            "native runner requires Linux task containers"
        );
        for phase in ["environment", "agent", "verifier"] {
            ensure!(
                v.pointer(&format!("/{phase}/network_mode"))
                    .and_then(Value::as_str)
                    .is_none_or(|s| matches!(s, "public" | "none")),
                "network allowlists are not supported"
            );
            if phase != "environment" {
                let base = if v.pointer("/environment/allow_internet") == Some(&json!(false)) {
                    "none"
                } else {
                    v.pointer("/environment/network_mode")
                        .and_then(Value::as_str)
                        .unwrap_or("public")
                };
                ensure!(
                    v.pointer(&format!("/{phase}/network_mode"))
                        .and_then(Value::as_str)
                        .is_none_or(|mode| mode == base),
                    "phase-specific network changes are unsupported"
                );
            }
        }
        ensure!(
            !path.join("environment/docker-compose.yaml").exists()
                && !path.join("environment/docker-compose.yml").exists(),
            "multi-container tasks unsupported"
        );
        ensure!(
            path.join("instruction.md").is_file() && path.join("tests/test.sh").is_file(),
            "task requires instruction.md and tests/test.sh"
        );
        ensure!(
            path.join("environment/Dockerfile").is_file()
                || v.pointer("/environment/docker_image")
                    .and_then(Value::as_str)
                    .is_some(),
            "missing Dockerfile or docker_image"
        );
        Ok(Self {
            path,
            name,
            config: v,
        })
    }
    pub fn seconds(&self, section: &str, default: f64, mult: f64) -> Result<u64> {
        let n = self
            .config
            .pointer(&format!("/{section}/timeout_sec"))
            .and_then(Value::as_f64)
            .unwrap_or(default)
            * mult;
        ensure!(
            n.is_finite() && n > 0. && n <= 7. * 86400.,
            "invalid {section} timeout"
        );
        Ok(n.ceil() as u64)
    }
}
pub(super) fn rewards(dir: &Path) -> Result<Value> {
    let json_path = dir.join("reward.json");
    let v = if json_path.exists() {
        serde_json::from_slice::<Value>(&fs::read(json_path)?)?
    } else {
        json!({"reward":fs::read_to_string(dir.join("reward.txt"))?.trim().parse::<f64>()?})
    };
    let v = if v.is_number() {
        json!({"reward":v})
    } else {
        v
    };
    ensure!(
        v.as_object().is_some_and(
            |m| !m.is_empty() && m.values().all(|v| v.as_f64().is_some_and(f64::is_finite))
        ),
        "rewards must be finite numbers"
    );
    Ok(v)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reward_precedence_and_strict_numbers() {
        let t = tempfile::tempdir().unwrap();
        fs::write(t.path().join("reward.txt"), "0.25").unwrap();
        assert_eq!(rewards(t.path()).unwrap()["reward"], 0.25);
        fs::write(
            t.path().join("reward.json"),
            r#"{"reward":1,"accuracy":0.5}"#,
        )
        .unwrap();
        assert_eq!(rewards(t.path()).unwrap()["reward"], 1);
        for s in ["true", r#"{"reward":"1"}"#, "{}", "NaN"] {
            fs::write(t.path().join("reward.json"), s).unwrap();
            assert!(rewards(t.path()).is_err());
        }
    }
}
