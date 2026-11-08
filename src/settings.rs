use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub projects: Vec<PathBuf>,
    pub selected: Option<PathBuf>,
    pub binary: Option<PathBuf>,
}
pub fn data_dir() -> PathBuf {
    std::env::var_os("ORBIT_DESKTOP_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            directories::BaseDirs::new()
                .expect("No user home directory")
                .data_local_dir()
                .join("nexode-desktop")
        })
}

/// One desktop owns this private runtime directory at a time. Kernel locks
/// disappear on a crash, unlike a PID or "already running" marker file.
pub fn lock_instance(directory: &Path) -> std::io::Result<Option<std::fs::File>> {
    std::fs::create_dir_all(directory)?;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(directory.join("desktop.lock"))?;
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(std::fs::TryLockError::WouldBlock) => Ok(None),
        Err(std::fs::TryLockError::Error(error)) => Err(error),
    }
}
impl Settings {
    pub fn load() -> Self {
        let mut settings: Self = std::fs::read(data_dir().join("desktop.json"))
            .ok()
            .and_then(|s| serde_json::from_slice(&s).ok())
            .unwrap_or_default();
        if settings.projects.is_empty() {
            let root = std::env::var_os("ORBIT_PROJECTS")
                .map(PathBuf::from)
                .or_else(|| {
                    std::env::current_dir()
                        .ok()
                        .and_then(|p| p.parent().map(Path::to_owned))
                });
            if let Some(root) = root.and_then(|p| std::fs::read_dir(p).ok()) {
                settings.projects = root
                    .flatten()
                    .map(|e| e.path())
                    .filter(|p| p.join(".git").exists())
                    .collect();
                settings.projects.sort();
            }
        }
        settings.selected = settings
            .selected
            .filter(|p| p.is_dir())
            .or_else(|| {
                settings
                    .projects
                    .iter()
                    .find(|p| p.file_name().is_some_and(|n| n == "nexode-orbit"))
                    .cloned()
            })
            .or_else(|| settings.projects.first().cloned());
        settings
    }
    pub fn save(&self) -> Result<(), String> {
        std::fs::create_dir_all(data_dir()).map_err(|e| e.to_string())?;
        std::fs::write(
            data_dir().join("desktop.json"),
            serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())
    }
}
pub fn engine_binary(settings: &Settings) -> PathBuf {
    if let Some(path) = std::env::var_os("NEXODE_BINARY") {
        return path.into();
    }
    if let Some(path) = &settings.binary {
        return path.clone();
    }
    let name = if cfg!(windows) {
        "nexode.exe"
    } else {
        "nexode"
    };
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_owned))
    {
        let bundled = dir.join(name);
        if bundled.is_file() {
            return bundled;
        }
    }
    if let Some(base) = directories::BaseDirs::new() {
        let paths = [
            base.data_local_dir().join("nexode/bin").join(name),
            base.home_dir().join(".local/bin").join(name),
        ];
        if let Some(p) = paths.into_iter().find(|p| p.is_file()) {
            return p;
        }
    }
    name.into()
}
pub fn label(path: &Path) -> String {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_one_window_can_own_the_private_runtime() {
        let dir = tempfile::tempdir().unwrap();
        let first = lock_instance(dir.path()).unwrap().expect("first owner");
        assert!(lock_instance(dir.path()).unwrap().is_none());
        drop(first);
        assert!(lock_instance(dir.path()).unwrap().is_some());
    }
}
