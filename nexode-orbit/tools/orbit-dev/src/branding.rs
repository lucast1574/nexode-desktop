use anyhow::{Context, Result, ensure};
use std::{
    fs,
    path::{Path, PathBuf},
};
const SKIP: &[&str] = &[
    ".git",
    "target",
    "node_modules",
    "dist",
    "__pycache__",
    ".venv",
];
fn walk(path: &Path, paths: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        if SKIP.iter().any(|skip| entry.file_name() == *skip) {
            continue;
        }
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            continue;
        }
        paths.push(entry.path());
        if kind.is_dir() {
            walk(&entry.path(), paths)?;
        }
    }
    Ok(())
}
fn cargo_paths(value: &toml::Value, parent: &Path) -> Result<()> {
    if let Some(table) = value.as_table() {
        if let Some(path) = table.get("path").and_then(toml::Value::as_str) {
            ensure!(
                parent.join(path).exists(),
                "missing Cargo path: {}",
                parent.join(path).display()
            );
        }
        for child in table.values() {
            cargo_paths(child, parent)?;
        }
    } else if let Some(array) = value.as_array() {
        for child in array {
            cargo_paths(child, parent)?;
        }
    }
    Ok(())
}
pub fn check(root: &Path) -> Result<()> {
    let mut paths = Vec::new();
    walk(root, &mut paths)?;
    let legacy = ["j", "code"].concat();
    let mut count = 0;
    for path in paths {
        let relative = path.strip_prefix(root)?;
        ensure!(
            !relative.to_string_lossy().to_lowercase().contains(&legacy),
            "legacy product path: {}",
            relative.display()
        );
        if !path.is_file() {
            continue;
        }
        if ["NOTICE", "LICENSE", "sdk/typescript/LICENSE"]
            .iter()
            .any(|p| relative == Path::new(p))
        {
            continue;
        }
        let bytes = fs::read(&path)?;
        let Ok(text) = std::str::from_utf8(&bytes) else {
            continue;
        };
        count += 1;
        ensure!(
            !text.to_lowercase().contains(&legacy),
            "legacy product identifier: {}",
            relative.display()
        );
        if path.file_name().is_some_and(|n| n == "Cargo.toml") {
            let manifest: toml::Value = text.parse()?;
            if let Some(members) = manifest
                .get("workspace")
                .and_then(|w| w.get("members"))
                .and_then(toml::Value::as_array)
            {
                for member in members {
                    let member = member.as_str().context("invalid workspace member")?;
                    ensure!(
                        path.parent()
                            .unwrap()
                            .join(member)
                            .join("Cargo.toml")
                            .is_file(),
                        "missing workspace member: {member}"
                    );
                }
            }
            cargo_paths(&manifest, path.parent().unwrap())?;
        }
        if relative.starts_with("sdk") && path.file_name().is_some_and(|n| n == "package.json") {
            let data: serde_json::Value = serde_json::from_str(text)?;
            ensure!(
                data["private"] == true,
                "SDK package must remain private until publication is configured: {}",
                relative.display()
            );
        }
    }
    let manifest: toml::Value = fs::read_to_string(root.join("Cargo.toml"))?.parse()?;
    ensure!(
        manifest["package"]["name"].as_str() == Some("nexode"),
        "unexpected root package"
    );
    ensure!(
        manifest["bin"]
            .as_array()
            .context("missing binaries")?
            .iter()
            .any(|b| b.get("name").and_then(toml::Value::as_str) == Some("nexode")),
        "missing nexode executable"
    );
    for asset in [
        "nexode-mark.svg",
        "nexode-wordmark.svg",
        "nexode-orbit-banner.svg",
    ] {
        let xml = fs::read_to_string(root.join("assets/brand").join(asset))?;
        let doc = roxmltree::Document::parse(&xml)?;
        ensure!(
            doc.root_element().tag_name().name() == "svg",
            "invalid SVG: {asset}"
        );
    }
    for (asset, signature) in [
        ("Nexode.icns", b"icns".as_slice()),
        ("Nexode.ico", b"\0\0\x01\0".as_slice()),
        ("Nexode.png", b"\x89PNG".as_slice()),
    ] {
        ensure!(
            fs::read(root.join("assets/app-icons").join(asset))?.starts_with(signature),
            "invalid image signature: {asset}"
        );
    }
    println!("Brand audit passed: {count} text files, Cargo paths, SDK metadata and assets.");
    Ok(())
}
