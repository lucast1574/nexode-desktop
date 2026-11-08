//! Native tray and per-user login startup. No services, elevated installation,
//! shell commands, or browser runtime are involved.
use eframe::egui::{Context, ViewportCommand};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    time::Duration,
};
use tray_icon::{
    MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    menu::{Menu, MenuEvent, MenuItem},
};

const STARTUP_NAME: &str = "Nexode Orbit";
#[cfg(any(target_os = "macos", test))]
const AGENT_LABEL: &str = "app.nexode.orbit.desktop";

#[derive(Clone, Copy)]
enum Action {
    Show,
    Quit,
}

/// Hold this for the entire lifetime of the egui app. Create it from
/// `App::new` (the native event loop is running by then).
pub struct DesktopShell {
    actions: Receiver<Action>,
    quitting: bool,
    stop: Arc<AtomicBool>,
    wake_worker: Option<std::thread::JoinHandle<()>>,
    #[cfg(not(target_os = "linux"))]
    _tray: TrayIcon,
    #[cfg(target_os = "linux")]
    gtk_worker: Option<std::thread::JoinHandle<()>>,
}

impl DesktopShell {
    pub fn new(ctx: &Context) -> Result<Self, String> {
        let (send, actions) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        #[cfg(not(target_os = "linux"))]
        let tray = make_tray(ctx.clone(), send.clone())?;
        #[cfg(target_os = "linux")]
        let gtk_worker = {
            let ctx = ctx.clone();
            let send = send.clone();
            let stop = stop.clone();
            let (ready_send, ready_receive) = mpsc::sync_channel(1);
            let worker = std::thread::spawn(move || {
                let result = gtk::init()
                    .map_err(|e| format!("No se pudo iniciar la bandeja GTK: {e}"))
                    .and_then(|()| make_tray(ctx, send));
                match result {
                    Ok(tray) => {
                        let _ = ready_send.send(Ok(()));
                        // GTK owns the tray on this thread; egui/winit owns the
                        // main thread. The timeout only handles orderly teardown.
                        gtk::glib::timeout_add_local(Duration::from_millis(250), move || {
                            if stop.load(Ordering::Acquire) {
                                gtk::main_quit();
                                gtk::glib::ControlFlow::Break
                            } else {
                                gtk::glib::ControlFlow::Continue
                            }
                        });
                        gtk::main();
                        drop(tray);
                    }
                    Err(error) => {
                        let _ = ready_send.send(Err(error));
                    }
                }
            });
            ready_receive
                .recv()
                .map_err(|_| "El proceso de bandeja no respondió".to_string())??;
            Some(worker)
        };

        let wake_worker = {
            let ctx = ctx.clone();
            let stop = stop.clone();
            let marker = show_marker();
            std::thread::spawn(move || {
                let mut last = std::fs::read(&marker).unwrap_or_default();
                while !stop.load(Ordering::Acquire) {
                    std::thread::sleep(Duration::from_millis(500));
                    if let Ok(next) = std::fs::read(&marker)
                        && next != last
                    {
                        last = next;
                        let _ = send.send(Action::Show);
                        ctx.request_repaint();
                    }
                }
            })
        };
        if background_requested() {
            ctx.send_viewport_cmd(ViewportCommand::Visible(false));
        }
        Ok(Self {
            actions,
            quitting: false,
            stop,
            wake_worker: Some(wake_worker),
            #[cfg(not(target_os = "linux"))]
            _tray: tray,
            #[cfg(target_os = "linux")]
            gtk_worker,
        })
    }

    /// Call at the beginning of every `App::update`. A tray action requests a
    /// repaint, so a hidden application does not need a rendering timer.
    pub fn update(&mut self, ctx: &Context) {
        while let Ok(action) = self.actions.try_recv() {
            match action {
                Action::Show => {
                    ctx.send_viewport_cmd(ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
                    ctx.send_viewport_cmd(ViewportCommand::Focus);
                }
                Action::Quit => {
                    self.quitting = true;
                    ctx.send_viewport_cmd(ViewportCommand::Close);
                }
            }
        }
        if !self.quitting && ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(ViewportCommand::Visible(false));
        }
    }
}

impl Drop for DesktopShell {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.wake_worker.take() {
            let _ = worker.join();
        }
        #[cfg(target_os = "linux")]
        if let Some(worker) = self.gtk_worker.take() {
            let _ = worker.join();
        }
    }
}

fn make_tray(ctx: Context, sender: Sender<Action>) -> Result<TrayIcon, String> {
    let menu = Menu::new();
    let show = MenuItem::new("Abrir Nexode Orbit", true, None);
    let quit = MenuItem::new("Salir de Nexode Orbit", true, None);
    menu.append_items(&[&show, &quit])
        .map_err(|e| e.to_string())?;
    let show_id = show.id().clone();
    let quit_id = quit.id().clone();
    let menu_sender = sender.clone();
    let menu_ctx = ctx.clone();
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        let action = if event.id == show_id {
            Some(Action::Show)
        } else if event.id == quit_id {
            Some(Action::Quit)
        } else {
            None
        };
        if let Some(action) = action {
            let _ = menu_sender.send(action);
            menu_ctx.request_repaint();
        }
    }));
    TrayIconEvent::set_event_handler(Some(move |event| {
        if matches!(
            event,
            TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            }
        ) {
            let _ = sender.send(Action::Show);
            ctx.request_repaint();
        }
    }));
    let rgba = image::load_from_memory(include_bytes!("../assets/icon.png"))
        .map_err(|e| e.to_string())?
        .resize(32, 32, image::imageops::FilterType::Lanczos3)
        .into_rgba8();
    let (width, height) = rgba.dimensions();
    let icon =
        tray_icon::Icon::from_rgba(rgba.into_raw(), width, height).map_err(|e| e.to_string())?;
    TrayIconBuilder::new()
        .with_tooltip(STARTUP_NAME)
        .with_icon(icon)
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .build()
        .map_err(|e| format!("No se pudo crear la bandeja: {e}"))
}

pub fn background_requested() -> bool {
    std::env::args_os().any(|arg| arg == "--background")
}

fn show_marker() -> PathBuf {
    crate::settings::data_dir().join("show-window.request")
}

/// Invoke when the existing single-instance lock is held. This only signals
/// visibility; it never sends commands or project contents to another process.
pub fn signal_existing_instance() -> std::io::Result<()> {
    let value = format!(
        "{}:{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    std::fs::write(show_marker(), value)
}

#[derive(Clone, Debug, Default)]
pub struct StartupStatus {
    /// Our per-user registration exists. The OS may separately disable it.
    pub registered: bool,
    pub disabled_by_windows: bool,
}

impl StartupStatus {
    pub fn enabled(&self) -> bool {
        self.registered && !self.disabled_by_windows
    }
}

fn executable() -> Result<PathBuf, String> {
    let current = std::env::current_exe().map_err(|e| e.to_string())?;
    #[cfg(target_os = "linux")]
    let path = startup_executable(current, std::env::var_os("APPIMAGE").map(PathBuf::from))?;
    #[cfg(not(target_os = "linux"))]
    let path = current;
    validate_executable(&path)?;
    if !path.is_file() {
        return Err("El ejecutable para iniciar con el sistema ya no existe".into());
    }
    Ok(path)
}

#[cfg(any(target_os = "linux", test))]
fn startup_executable(current: PathBuf, appimage: Option<PathBuf>) -> Result<PathBuf, String> {
    // AppImage's current_exe lives in a temporary /tmp/.mount-* filesystem.
    // The launcher supplies APPIMAGE with its persistent, external file path.
    let selected = appimage.unwrap_or(current);
    validate_executable(&selected)?;
    if !selected.is_file() {
        return Err("La ruta de AppImage/ejecutable para el inicio no existe".into());
    }
    Ok(selected)
}

fn validate_executable(path: &Path) -> Result<(), String> {
    if !path.is_absolute() {
        return Err("El ejecutable de inicio debe tener una ruta absoluta".into());
    }
    let text = path
        .to_str()
        .ok_or("La ruta del ejecutable no es Unicode válido")?;
    if text.contains(['\0', '\n', '\r']) {
        return Err("La ruta contiene caracteres no válidos para el inicio".into());
    }
    Ok(())
}

#[cfg(windows)]
pub fn startup_status() -> Result<StartupStatus, String> {
    use winreg::{RegKey, enums::HKEY_CURRENT_USER};
    let root = RegKey::predef(HKEY_CURRENT_USER);
    let registered = match root.open_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Run") {
        Ok(key) => key.get_value::<String, _>(STARTUP_NAME).is_ok(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(error.to_string()),
    };
    // Task Manager owns this value. Read it to reflect an OS-level disable;
    // normal launches must never reset it or secretly reenable startup.
    let disabled_by_windows = root
        .open_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\StartupApproved\\Run")
        .and_then(|key| key.get_raw_value(STARTUP_NAME))
        .ok()
        .is_some_and(|value| matches!(value.bytes.first(), Some(3 | 7)));
    Ok(StartupStatus {
        registered,
        disabled_by_windows,
    })
}

#[cfg(windows)]
pub fn set_startup(enabled: bool) -> Result<(), String> {
    use winreg::{RegKey, enums::HKEY_CURRENT_USER};
    let root = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = root
        .create_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Run")
        .map_err(|e| e.to_string())?;
    if enabled {
        let path = executable()?;
        let value = windows_command(path.to_str().unwrap());
        key.set_value(STARTUP_NAME, &value)
            .map_err(|e| e.to_string())
    } else {
        match key.delete_value(STARTUP_NAME) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.to_string()),
        }
    }
}

#[cfg(any(windows, test))]
fn windows_command(path: &str) -> String {
    format!("\"{path}\" --background")
}

#[cfg(not(windows))]
fn startup_file() -> Result<PathBuf, String> {
    let dirs = directories::BaseDirs::new().ok_or("No se encontró la carpeta del usuario")?;
    #[cfg(target_os = "macos")]
    {
        Ok(dirs
            .home_dir()
            .join("Library/LaunchAgents")
            .join(format!("{AGENT_LABEL}.plist")))
    }
    #[cfg(not(target_os = "macos"))]
    {
        Ok(dirs.config_dir().join("autostart/nexode-orbit.desktop"))
    }
}

#[cfg(not(windows))]
pub fn startup_status() -> Result<StartupStatus, String> {
    let path = startup_file()?;
    let registered = match std::fs::read_to_string(path) {
        Ok(contents) => {
            #[cfg(target_os = "macos")]
            {
                contents.contains(AGENT_LABEL)
            }
            #[cfg(not(target_os = "macos"))]
            {
                !contents.lines().any(|line| {
                    matches!(
                        line.trim(),
                        "Hidden=true" | "X-GNOME-Autostart-enabled=false"
                    )
                })
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(error.to_string()),
    };
    Ok(StartupStatus {
        registered,
        disabled_by_windows: false,
    })
}

#[cfg(not(windows))]
pub fn set_startup(enabled: bool) -> Result<(), String> {
    let path = startup_file()?;
    if !enabled {
        return match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.to_string()),
        };
    }
    let executable = executable()?;
    #[cfg(target_os = "macos")]
    let contents = launch_agent(executable.to_str().unwrap());
    #[cfg(not(target_os = "macos"))]
    let contents = desktop_entry(executable.to_str().unwrap());
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(path, contents).map_err(|e| e.to_string())
}

#[cfg(any(target_os = "macos", test))]
fn launch_agent(executable: &str) -> String {
    let escaped = executable
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;");
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>Label</key><string>{AGENT_LABEL}</string>
<key>ProgramArguments</key><array><string>{escaped}</string><string>--background</string></array>
<key>RunAtLoad</key><true/>
<key>ProcessType</key><string>Interactive</string>
</dict></plist>
"#
    )
}

#[cfg(any(target_os = "linux", test))]
fn desktop_entry(executable: &str) -> String {
    // Desktop Entry Exec quoting has two parsing layers (string then Exec).
    // Escape backslash twice, then reserved quote/$/backtick for both layers.
    let escaped = executable
        .replace('\\', "\\\\\\\\")
        .replace('"', "\\\\\"")
        .replace('$', "\\\\$")
        .replace('`', "\\\\`")
        .replace('%', "%%");
    format!(
        "[Desktop Entry]\nType=Application\nName=Nexode Orbit\nComment=Harness de Nexode\nExec=\"{escaped}\" --background\nTerminal=false\nStartupNotify=false\nX-GNOME-Autostart-enabled=true\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_startup_quotes_executable_with_spaces() {
        assert_eq!(
            windows_command(r"C:\Program Files\Nexode Orbit\nexode-desktop.exe"),
            r#""C:\Program Files\Nexode Orbit\nexode-desktop.exe" --background"#
        );
    }

    #[test]
    fn linux_startup_escapes_exec_field_codes_and_shell_characters() {
        let entry = desktop_entry("/home/me/100%/$apps/`native`/orbit\"name");
        assert!(entry.contains("100%%"));
        assert!(entry.contains("\\\\$apps"));
        assert!(entry.contains("\\\\`native\\\\`"));
        assert!(entry.contains("orbit\\\\\"name"));
        assert!(entry.contains("Terminal=false"));
    }

    #[test]
    fn mac_startup_is_a_direct_argument_array_not_a_shell_command() {
        let plist = launch_agent("/Applications/Nexode & Orbit.app/Contents/MacOS/orbit");
        assert!(plist.contains("Nexode &amp; Orbit.app"));
        assert!(plist.contains("<string>--background</string>"));
        assert!(!plist.contains("/bin/sh"));
        assert!(!plist.contains("KeepAlive"));
    }

    #[test]
    fn os_disabled_startup_is_not_reported_enabled() {
        assert!(
            !StartupStatus {
                registered: true,
                disabled_by_windows: true
            }
            .enabled()
        );
        assert!(
            StartupStatus {
                registered: true,
                disabled_by_windows: false
            }
            .enabled()
        );
    }

    #[test]
    fn startup_rejects_relative_paths_and_newline_injection() {
        assert!(validate_executable(Path::new("relative.exe")).is_err());
        let path = std::env::current_dir().unwrap().join("app\nHidden=false");
        assert!(validate_executable(&path).is_err());
    }

    #[test]
    fn appimage_startup_uses_existing_persistent_image_not_mount() {
        let directory = tempfile::tempdir().unwrap();
        let image = directory.path().join("Nexode 100%.AppImage");
        std::fs::write(&image, b"test fixture").unwrap();
        let mounted = directory.path().join(".mount/orbit");
        let selected = startup_executable(mounted.clone(), Some(image.clone())).unwrap();
        assert_eq!(selected, image);
        assert!(desktop_entry(selected.to_str().unwrap()).contains("100%%.AppImage"));
        assert!(startup_executable(mounted.clone(), Some(mounted)).is_err());
        assert!(startup_executable(image, Some(PathBuf::from("relative.AppImage"))).is_err());
    }
}
