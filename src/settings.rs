use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub projects: Vec<PathBuf>,
    /// Older versions discovered folders automatically. Keep those paths for
    /// explicit recovery instead of treating discovery as the user's consent.
    pub previous_projects: Vec<PathBuf>,
    #[serde(default)]
    pub project_setup_version: u8,
    pub selected: Option<PathBuf>,
    pub binary: Option<PathBuf>,
    pub pinned_sessions: Vec<String>,
    pub onboarding_complete: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            projects: vec![],
            previous_projects: vec![],
            project_setup_version: 1,
            selected: None,
            binary: None,
            pinned_sessions: vec![],
            onboarding_complete: false,
        }
    }
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
        Self::load_from(&data_dir())
    }
    fn load_from(directory: &Path) -> Self {
        let mut settings: Self = std::fs::read(directory.join("desktop.json"))
            .ok()
            .and_then(|s| serde_json::from_slice(&s).ok())
            .unwrap_or_default();
        if settings.project_setup_version == 0 {
            for project in std::mem::take(&mut settings.projects)
                .into_iter()
                .chain(settings.selected.take())
            {
                if !settings
                    .previous_projects
                    .iter()
                    .any(|p| same_project(p, &project))
                {
                    settings.previous_projects.push(project);
                }
            }
            settings.project_setup_version = 1;
        }
        // Resolve aliases once during loading/registration, never during a
        // render pass. Missing folders remain recoverable metadata.
        for project in &mut settings.projects {
            *project = std::fs::canonicalize(&*project).unwrap_or_else(|_| project.clone());
        }
        settings.selected = settings.selected.and_then(|p| {
            let p = std::fs::canonicalize(&p).unwrap_or(p);
            settings
                .projects
                .iter()
                .find(|registered| same_project(registered, &p))
                .cloned()
        });
        settings
    }
    pub fn add_project(&mut self, path: PathBuf) -> Result<(), String> {
        let path = std::fs::canonicalize(path)
            .map_err(|e| format!("No se puede abrir la carpeta: {e}"))?;
        if !path.is_dir() {
            return Err("Selecciona una carpeta para añadir el proyecto.".into());
        }
        if !self.projects.iter().any(|p| same_project(p, &path)) {
            self.projects.push(path.clone());
        }
        self.previous_projects.retain(|p| {
            let normalized = std::fs::canonicalize(p).unwrap_or_else(|_| p.clone());
            !same_project(&normalized, &path)
        });
        self.project_setup_version = 1;
        self.selected = Some(path);
        Ok(())
    }
    pub fn is_session_pinned(&self, session_id: &str) -> bool {
        self.pinned_sessions.iter().any(|id| id == session_id)
    }
    pub fn toggle_session_pin(&mut self, session_id: &str) {
        if self.is_session_pinned(session_id) {
            self.pinned_sessions.retain(|id| id != session_id);
        } else if !session_id.is_empty() {
            self.pinned_sessions.push(session_id.to_owned());
        }
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

/// Runtime session paths may carry Windows' verbatim prefix even when the
/// folder picker does not. Identity also ignores separator/case differences on
/// Windows, while preserving case-sensitive Unix paths.
pub fn project_key(path: &Path) -> String {
    normalized_key(&path.to_string_lossy(), cfg!(windows))
}
pub fn same_project(left: &Path, right: &Path) -> bool {
    project_key(left) == project_key(right)
}
pub fn session_matches_project(session: &nexode_sdk::SessionInfo, project: &Path) -> bool {
    session
        .working_dir
        .as_deref()
        .is_some_and(|path| same_project(Path::new(path), project))
}
fn normalized_key(raw: &str, windows: bool) -> String {
    let raw = if windows {
        raw.replace('\\', "/")
    } else {
        raw.to_owned()
    };
    let raw = if windows {
        if let Some(unc) = raw
            .strip_prefix("//?/UNC/")
            .or_else(|| raw.strip_prefix("//?/unc/"))
        {
            format!("//{unc}")
        } else {
            raw.strip_prefix("//?/").unwrap_or(&raw).to_owned()
        }
    } else {
        raw
    };
    let prefix = if raw.starts_with("//") && windows {
        "//"
    } else if raw.starts_with('/') {
        "/"
    } else {
        ""
    };
    let mut components: Vec<&str> = Vec::new();
    for component in raw.split('/') {
        match component {
            "" | "." => {}
            ".." if components
                .last()
                .is_some_and(|last| *last != ".." && !last.ends_with(':')) =>
            {
                components.pop();
            }
            _ => components.push(component),
        }
    }
    let result = format!("{prefix}{}", components.join("/"));
    if windows {
        result.to_lowercase()
    } else {
        result
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
    fn pins_and_onboarding_survive_reload_without_implicit_setup() {
        let directory = tempfile::tempdir().unwrap();
        let mut settings = Settings::default();
        assert!(!settings.onboarding_complete);
        settings.toggle_session_pin("chat-a");
        settings.toggle_session_pin("chat-b");
        settings.toggle_session_pin("chat-a");
        settings.toggle_session_pin("");
        settings.onboarding_complete = true;
        std::fs::write(
            directory.path().join("desktop.json"),
            serde_json::to_vec(&settings).unwrap(),
        )
        .unwrap();
        let loaded = Settings::load_from(directory.path());
        assert_eq!(loaded.pinned_sessions, ["chat-b"]);
        assert!(loaded.onboarding_complete);
        assert!(loaded.projects.is_empty());
        let legacy: Settings =
            serde_json::from_str(r#"{"project_setup_version":1,"projects":[]}"#).unwrap();
        assert!(!legacy.onboarding_complete);
        assert!(legacy.pinned_sessions.is_empty());
    }
    #[test]
    fn general_chat_selection_survives_registered_project_reload() {
        let directory = tempfile::tempdir().unwrap();
        let project = directory.path().join("project");
        std::fs::create_dir(&project).unwrap();
        let mut settings = Settings::default();
        settings.add_project(project).unwrap();
        settings.selected = None;
        std::fs::write(
            directory.path().join("desktop.json"),
            serde_json::to_vec(&settings).unwrap(),
        )
        .unwrap();
        let loaded = Settings::load_from(directory.path());
        assert_eq!(loaded.projects.len(), 1);
        assert!(loaded.selected.is_none());
    }
    #[test]
    fn only_one_window_can_own_the_private_runtime() {
        let dir = tempfile::tempdir().unwrap();
        let first = lock_instance(dir.path()).unwrap().expect("first owner");
        assert!(lock_instance(dir.path()).unwrap().is_none());
        drop(first);
        assert!(lock_instance(dir.path()).unwrap().is_some());
    }

    #[test]
    fn new_install_starts_empty_even_next_to_repositories() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("work/repo/.git")).unwrap();
        let settings = Settings::load_from(&root.path().join("data"));
        assert!(settings.projects.is_empty());
        assert!(settings.previous_projects.is_empty());
        assert!(settings.selected.is_none());
    }

    #[test]
    fn legacy_discovery_is_preserved_for_explicit_recovery() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("original-project");
        std::fs::create_dir(&project).unwrap();
        let raw =
            serde_json::json!({"projects":[project],"selected":project,"binary":"custom-engine"});
        std::fs::write(dir.path().join("desktop.json"), raw.to_string()).unwrap();
        let mut settings = Settings::load_from(dir.path());
        assert!(settings.projects.is_empty());
        assert!(settings.selected.is_none());
        assert_eq!(settings.previous_projects, vec![project.clone()]);
        assert_eq!(settings.binary, Some(PathBuf::from("custom-engine")));
        assert!(project.is_dir());
        settings.add_project(project.clone()).unwrap();
        settings.add_project(project.join(".")).unwrap();
        assert_eq!(settings.projects.len(), 1);
        assert!(settings.previous_projects.is_empty());
        std::fs::write(
            dir.path().join("desktop.json"),
            serde_json::to_vec(&settings).unwrap(),
        )
        .unwrap();
        let restored = Settings::load_from(dir.path());
        assert_eq!(restored.projects.len(), 1);
        assert_eq!(restored.selected, settings.selected);
    }

    #[test]
    fn session_path_identity_handles_windows_verbatim_drive_and_unc() {
        assert_eq!(
            normalized_key(r"\\?\D:\Projects\Orbit", true),
            normalized_key("d:/projects/orbit/", true)
        );
        assert_eq!(
            normalized_key(r"\\?\UNC\Server\Share\Orbit", true),
            normalized_key(r"\\server\share\orbit\", true)
        );
        assert_ne!(
            normalized_key("/work/Orbit", false),
            normalized_key("/work/orbit", false)
        );
        assert_eq!(normalized_key("/work/./repo/", false), "/work/repo");
    }

    #[cfg(unix)]
    #[test]
    fn explicit_symlink_folder_and_runtime_canonical_folder_match() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("project");
        let alias = dir.path().join("alias");
        std::fs::create_dir(&project).unwrap();
        std::os::unix::fs::symlink(&project, &alias).unwrap();
        let mut settings = Settings::default();
        settings.add_project(alias).unwrap();
        // macOS's temporary root itself can be a symlink (/var -> /private/var).
        // Compare against the canonical path the runtime sends, not that alias.
        let runtime_project = std::fs::canonicalize(project).unwrap();
        assert!(same_project(&settings.projects[0], &runtime_project));
    }
}
