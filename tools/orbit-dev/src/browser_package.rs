//! Build browser packages from an external Bridge extension checkout, without Python.
//! Manifest transformation preserves the upstream extension/native-host identity.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};
use zip::write::SimpleFileOptions;
const CHROME_PUBLIC_KEY: &str = "MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEA3Ia0X9n173BX5Rvs+u4ubpuVTsk8GZqHI1BDPSwI4m4prpv9CYftsTw27zptwySBMlJAeiGyMl29dlJu6uZdQcUFJ2BciSJjB+aiUrRCgb+DmvLEMyBbwIcOyNz3DlCPADEqzR7B1vzGEcxemEPZMh3+ZLEZtENMQ3hdLzPAA+RRagQQLDitLIQtfI7rJDNGQqH7bEQV+VruQY+Dka+FXwMq0zNiQRsPQj5rqiKSqy6FuiddfhJ+XS4YlhYq6/ZH9feRkYm+j6pG+k+UPUj7zBT7iVV2IZZfQ6FLlZiRa6LQ5heOaiBEuettn2S+Y9lpX3wyKn87Zx1m8916UvQFewIDAQAB";
fn chromium(base: &Value) -> Value {
    json!({
        "manifest_version":3,"name":base["name"],"version":base["version"],
        "description":base["description"],"icons":base["icons"],"key":CHROME_PUBLIC_KEY,
        "minimum_chrome_version":"116",
        "permissions":["tabs","activeTab","nativeMessaging","notifications","storage","webNavigation","downloads","scripting","alarms","debugger"],
        "optional_permissions":["userScripts"],"host_permissions":["<all_urls>"],
        "background":{"service_worker":"background.js"},"content_scripts":base["content_scripts"],"action":base["browser_action"]
    })
}
fn safari(base: &Value) -> Value {
    let mut manifest = chromium(base);
    let object = manifest.as_object_mut().unwrap();
    for key in ["key", "minimum_chrome_version", "optional_permissions"] {
        object.remove(key);
    }
    object
        .get_mut("permissions")
        .unwrap()
        .as_array_mut()
        .unwrap()
        .retain(|v| {
            !["nativeMessaging", "downloads", "debugger"].contains(&v.as_str().unwrap_or_default())
        });
    object.insert(
        "background".into(),
        json!({"scripts":["background.js"],"persistent":false}),
    );
    object.insert(
        "browser_specific_settings".into(),
        json!({"safari":{"strict_min_version":"17.0"}}),
    );
    manifest
}
fn copy(source: &Path, target: &Path) -> Result<()> {
    let meta = fs::symlink_metadata(source)?;
    ensure!(
        !meta.file_type().is_symlink(),
        "extension assets must not be symlinks: {}",
        source.display()
    );
    if meta.is_dir() {
        fs::create_dir_all(target)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            copy(&entry.path(), &target.join(entry.file_name()))?;
        }
    } else {
        fs::copy(source, target)?;
    }
    Ok(())
}
fn files(dir: &Path, output: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            files(&entry.path(), output)?;
        } else {
            output.push(entry.path());
        }
    }
    Ok(())
}
pub fn package(source: &Path, output: &Path) -> Result<Vec<PathBuf>> {
    let source = fs::canonicalize(source)?;
    fs::create_dir_all(output)?;
    let output = fs::canonicalize(output)?;
    ensure!(
        !output.starts_with(&source) && !source.starts_with(&output),
        "source and output must be separate directories"
    );
    let base: Value = serde_json::from_slice(&fs::read(source.join("manifest.json"))?)?;
    let version = base["version"]
        .as_str()
        .context("missing extension version")?;
    ensure!(
        !version.is_empty() && version.bytes().all(|c| c.is_ascii_digit() || c == b'.'),
        "invalid extension version"
    );
    for name in [
        "background.js",
        "content.js",
        "popup.html",
        "popup.js",
        "LICENSE",
        "icons",
    ] {
        ensure!(source.join(name).exists(), "missing browser asset: {name}");
    }
    let mut archives = Vec::new();
    for (browser, manifest, name) in [
        (
            "firefox",
            base.clone(),
            format!("browser-agent-bridge-{version}.xpi"),
        ),
        (
            "chrome",
            chromium(&base),
            format!("browser-agent-bridge-chrome-{version}.zip"),
        ),
        (
            "safari",
            safari(&base),
            format!("browser-agent-bridge-safari-{version}.zip"),
        ),
    ] {
        let stage = output.join(browser);
        ensure!(!stage.is_symlink(), "output staging directory is a symlink");
        if stage.exists() {
            fs::remove_dir_all(&stage)?;
        }
        fs::create_dir(&stage)?;
        for file in [
            "background.js",
            "content.js",
            "popup.html",
            "popup.js",
            "LICENSE",
            "icons",
        ] {
            copy(&source.join(file), &stage.join(file))?;
        }
        fs::write(
            stage.join("manifest.json"),
            format!("{}\n", serde_json::to_string_pretty(&manifest)?),
        )?;
        let archive = output.join(name);
        ensure!(!archive.is_symlink(), "output archive is a symlink");
        let mut writer = zip::ZipWriter::new(fs::File::create(&archive)?);
        let mut paths = Vec::new();
        files(&stage, &mut paths)?;
        paths.sort();
        for path in paths {
            writer.start_file(
                path.strip_prefix(&stage)?
                    .to_string_lossy()
                    .replace('\\', "/"),
                SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Deflated)
                    .unix_permissions(0o644),
            )?;
            writer.write_all(&fs::read(path)?)?;
        }
        writer.finish()?;
        println!("{}", archive.display());
        archives.push(archive);
    }
    Ok(archives)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn manifests_keep_browser_specific_transport_and_permissions() {
        let base = json!({"name":"Bridge","version":"1.2.3","description":"test","icons":{},"content_scripts":[],"browser_action":{},"manifest_version":2});
        let chrome = chromium(&base);
        let safari = safari(&base);
        assert_eq!(chrome["manifest_version"], 3);
        assert_eq!(chrome["key"], CHROME_PUBLIC_KEY);
        assert_eq!(chrome["background"]["service_worker"], "background.js");
        assert!(safari.get("key").is_none());
        assert!(
            !safari["permissions"]
                .as_array()
                .unwrap()
                .contains(&json!("nativeMessaging"))
        );
        assert_eq!(safari["background"]["persistent"], false);
        assert_eq!(base["manifest_version"], 2);
    }
    #[test]
    fn packages_include_assets_and_license_without_interpreters() {
        let temp = tempfile::tempdir().unwrap();
        let src = temp.path().join("extension");
        fs::create_dir_all(src.join("icons")).unwrap();
        for file in [
            "background.js",
            "content.js",
            "popup.html",
            "popup.js",
            "LICENSE",
            "icons/32.png",
        ] {
            fs::write(src.join(file), b"fixture").unwrap();
        }
        fs::write(src.join("manifest.json"),json!({"name":"Bridge","version":"1.0","description":"test","icons":{},"content_scripts":[],"browser_action":{},"manifest_version":2}).to_string()).unwrap();
        let archives = package(&src, &temp.path().join("dist")).unwrap();
        assert_eq!(archives.len(), 3);
        for path in archives {
            let mut archive = zip::ZipArchive::new(fs::File::open(path).unwrap()).unwrap();
            assert!(archive.by_name("LICENSE").is_ok());
            assert!(archive.by_name("manifest.json").is_ok());
            assert!(archive.by_name("icons/32.png").is_ok());
        }
        assert!(package(&src, &src).is_err());
    }
}
