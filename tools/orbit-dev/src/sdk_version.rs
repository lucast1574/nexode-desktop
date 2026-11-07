use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::{fs, path::Path};
#[derive(Debug, PartialEq)]
pub struct Release {
    pub version: String,
    pub tag: &'static str,
}
fn stable(s: &str) -> bool {
    let parts: Vec<_> = s.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
}
pub fn release_version(base: &str, runtime: &str) -> Result<Release> {
    ensure!(stable(base), "expected stable SDK base version");
    let runtime = runtime
        .strip_prefix('v')
        .context("expected v-prefixed runtime tag")?;
    if let Some((version, n)) = runtime.split_once("-preview.") {
        ensure!(
            stable(version)
                && !n.is_empty()
                && !n.starts_with('0')
                && n.bytes().all(|b| b.is_ascii_digit()),
            "unsupported preview tag"
        );
        return Ok(Release {
            version: format!("{base}-preview.{n}"),
            tag: "preview",
        });
    }
    ensure!(stable(runtime), "unsupported runtime tag");
    Ok(Release {
        version: base.to_owned(),
        tag: "latest",
    })
}
pub fn version_packages(root: &Path, runtime: &str) -> Result<Release> {
    let path = root.join("sdk/typescript/package.json");
    let mut main: Value = serde_json::from_slice(&fs::read(&path)?)?;
    let release = release_version(
        main["version"].as_str().context("SDK version missing")?,
        runtime,
    )?;
    let mut updates = Vec::new();
    let mut names = std::collections::BTreeSet::new();
    // Validate all inputs before writing any manifest.
    for entry in fs::read_dir(root.join("sdk/npm"))? {
        let path = entry?.path().join("package.json");
        let mut pkg: Value = serde_json::from_slice(&fs::read(&path)?)?;
        let name = pkg["name"]
            .as_str()
            .context("runtime name missing")?
            .to_owned();
        ensure!(
            main["optionalDependencies"].get(&name).is_some(),
            "unreferenced runtime: {name}"
        );
        ensure!(names.insert(name.clone()), "duplicate runtime: {name}");
        pkg["version"] = release.version.clone().into();
        main["optionalDependencies"][&name] = release.version.clone().into();
        updates.push((path, pkg));
    }
    ensure!(
        names.len()
            == main["optionalDependencies"]
                .as_object()
                .context("runtime dependencies missing")?
                .len(),
        "missing runtime package"
    );
    main["version"] = release.version.clone().into();
    updates.push((path, main));
    for (path, value) in updates {
        fs::write(path, format!("{}\n", serde_json::to_string_pretty(&value)?))?;
    }
    Ok(release)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn previews_do_not_advance_latest() {
        assert_eq!(
            release_version("1.2.0", "v1.0.0-preview.30").unwrap(),
            Release {
                version: "1.2.0-preview.30".into(),
                tag: "preview"
            }
        );
        assert_eq!(release_version("1.2.0", "v1.0.0").unwrap().tag, "latest");
        for tag in [
            "master",
            "v1.0.0-rc.1",
            "v1.0.0-preview.0",
            "v1.0.0-preview.30;evil",
            "v1.0.0-preview.01",
        ] {
            assert!(release_version("1.2.0", tag).is_err());
        }
    }
    fn fixture() -> tempfile::TempDir {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join("sdk/typescript")).unwrap();
        let mut deps = serde_json::Map::new();
        for platform in [
            "darwin-arm64",
            "darwin-x64",
            "linux-arm64",
            "linux-x64",
            "win32-arm64",
            "win32-x64",
        ] {
            let dir = temp.path().join("sdk/npm").join(platform);
            fs::create_dir_all(&dir).unwrap();
            let name = format!("@nexode/agent-{platform}");
            deps.insert(name.clone(), "1.2.0".into());
            fs::write(
                dir.join("package.json"),
                serde_json::json!({"name":name,"version":"1.2.0","private":true}).to_string(),
            )
            .unwrap();
        }
        fs::write(temp.path().join("sdk/typescript/package.json"),serde_json::json!({"name":"@nexode/agent-sdk","version":"1.2.0","optionalDependencies":deps}).to_string()).unwrap();
        temp
    }
    #[test]
    fn all_platform_versions_are_pinned() {
        let temp = fixture();
        version_packages(temp.path(), "v1.0.0-preview.42").unwrap();
        let main: Value = serde_json::from_slice(
            &fs::read(temp.path().join("sdk/typescript/package.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(main["version"], "1.2.0-preview.42");
        for entry in fs::read_dir(temp.path().join("sdk/npm")).unwrap() {
            let pkg: Value = serde_json::from_slice(
                &fs::read(entry.unwrap().path().join("package.json")).unwrap(),
            )
            .unwrap();
            assert_eq!(pkg["version"], main["version"]);
            assert_eq!(
                main["optionalDependencies"][pkg["name"].as_str().unwrap()],
                main["version"]
            );
            assert_eq!(pkg["private"], true);
        }
    }
    #[test]
    fn invalid_runtime_does_not_partially_rewrite_manifests() {
        let temp = fixture();
        let path = temp.path().join("sdk/typescript/package.json");
        let before = fs::read(&path).unwrap();
        fs::write(
            temp.path().join("sdk/npm/linux-x64/package.json"),
            "{invalid",
        )
        .unwrap();
        assert!(version_packages(temp.path(), "v1.0.0-preview.2").is_err());
        assert_eq!(before, fs::read(path).unwrap());
    }
}
