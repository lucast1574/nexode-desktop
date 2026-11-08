//! Private instance additions. Project files and unrelated MCP entries are preserved.
use serde_json::{Value, json};
use std::path::Path;
pub fn prepare(home: &Path) -> Result<(), String> {
    std::fs::create_dir_all(home).map_err(|e| e.to_string())?;
    let path = home.join("mcp.json");
    let config: Value = match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|_| "La configuración MCP privada no es JSON válido.".to_string())?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => json!({"mcpServers":{}}),
        Err(e) => return Err(e.to_string()),
    };
    // A locked/unavailable vault must not lock users out of the free harness.
    // Account UI reports the credential-store error and allows a later retry.
    let enabled = std::env::var_os("ORBIT_NATIVE_TEST").is_none()
        && crate::cloud::credential().ok().flatten().is_some();
    let config = configure_mcp(
        config,
        enabled,
        &std::env::current_exe().map_err(|e| e.to_string())?,
    )?;
    std::fs::write(
        &path,
        serde_json::to_vec_pretty(&config).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    std::fs::write(home.join("prompt-overlay.md"), crate::ecosystem::guidance())
        .map_err(|e| e.to_string())?;
    Ok(())
}
fn configure_mcp(mut config: Value, enabled: bool, executable: &Path) -> Result<Value, String> {
    let servers = config
        .as_object_mut()
        .ok_or("Configuración MCP inválida.")?
        .entry("mcpServers")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or("mcpServers debe ser un objeto.")?;
    if enabled {
        servers.insert(
            "nexode".into(),
            json!({"command":executable,"args":["--nexode-mcp"]}),
        );
    } else {
        servers.remove("nexode");
    }
    Ok(config)
}
/// The key is injected only into the official MCP child, never the harness environment.
pub fn run_mcp() -> Result<(), String> {
    let key = crate::cloud::credential()?.ok_or("Inicia sesión en Nexode desde Orbit.")?;
    let bundled = std::env::current_exe()
        .map_err(|e| e.to_string())?
        .parent()
        .ok_or("Carpeta de aplicación no disponible.")?
        .join("mcp");
    let node = bundled.join(if cfg!(windows) { "node.exe" } else { "node" });
    if node.is_file() && bundled.join("dist/index.js").is_file() {
        let status = std::process::Command::new(node)
            .arg(bundled.join("dist/index.js"))
            .env("NEXODE_API_KEY", key.as_str())
            .env_remove("NEXODE_API_URL")
            .status()
            .map_err(|_| "No se pudo iniciar el conector Nexode incluido.".to_string())?;
        return if status.success() {
            Ok(())
        } else {
            Err("El conector Nexode terminó con error.".into())
        };
    }
    #[cfg(windows)]
    let mut command = {
        let mut c = std::process::Command::new("cmd.exe");
        c.args(["/D", "/C", "npx.cmd"]);
        c
    };
    #[cfg(not(windows))]
    let mut command = std::process::Command::new("npx");
    let status = command
        .args([
            "--yes",
            "--package=github:lucast1574/nexode-mcp#v0.1.26",
            "nexode-mcp",
        ])
        .env("NEXODE_API_KEY", key.as_str())
        .env_remove("NEXODE_API_URL")
        .status()
        .map_err(|_| {
            "El conector Nexode MCP requiere Node.js 20 o superior y npx en PATH.".to_string()
        })?;
    if status.success() {
        Ok(())
    } else {
        Err("El conector Nexode MCP terminó con error.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn connector_contains_no_secret_and_logout_preserves_other_servers() {
        let initial = json!({"mcpServers":{"other":{"command":"other-tool"}}});
        let enabled =
            configure_mcp(initial.clone(), true, Path::new("/app/nexode-desktop")).unwrap();
        assert_eq!(enabled["mcpServers"]["nexode"]["args"][0], "--nexode-mcp");
        assert!(enabled["mcpServers"]["nexode"].get("env").is_none());
        assert_eq!(
            configure_mcp(enabled, false, Path::new("/app/nexode-desktop")).unwrap(),
            initial
        );
        assert!(configure_mcp(json!([]), true, Path::new("/app")).is_err());
    }
}
