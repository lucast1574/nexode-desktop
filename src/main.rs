#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod app;
mod backend;
mod cloud;
mod desktop_shell;
mod ecosystem;
mod mascot;
mod panels;
mod platform_runtime;
mod provider_connections;
mod provider_usage;
mod settings;
mod theme;
mod welcome_content;
mod welcome_web;
mod widgets;
#[cfg(windows)]
mod windows_job;

fn main() -> eframe::Result {
    // WebKitGTK 4.1's process sandbox is opt-in. Enable it before GTK or Wry
    // creates any context (including Wry's private incognito context).
    #[cfg(target_os = "linux")]
    // SAFETY: this is the first operation in main, before any threads, GTK,
    // runtime, network client, or environment-reading worker can be started.
    unsafe {
        std::env::set_var("WEBKIT_FORCE_SANDBOX", "1");
    }
    if std::env::args().any(|arg| arg == "--enable-startup") {
        if let Err(error) = desktop_shell::set_startup(true) {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return Ok(());
    }
    if std::env::args().any(|arg| arg == "--nexode-mcp") {
        if let Err(error) = platform_runtime::run_mcp() {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return Ok(());
    }
    let Some(_instance_lock) = settings::lock_instance(&settings::data_dir())
        .map_err(|e| eframe::Error::AppCreation(Box::new(e)))?
    else {
        if !desktop_shell::background_requested() {
            let _ = desktop_shell::signal_existing_instance();
        }
        return Ok(());
    };
    #[cfg(windows)]
    windows_job::contain_child_processes().map_err(|e| eframe::Error::AppCreation(Box::new(e)))?;
    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon.png")).ok();
    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_title("Nexode Orbit")
        .with_inner_size([1380.0, 880.0])
        .with_min_inner_size([1024.0, 640.0])
        .with_app_id("app.nexode.orbit.desktop");
    if let Some(icon) = icon {
        viewport = viewport.with_icon(icon);
    }
    eframe::run_native(
        "Nexode Orbit",
        eframe::NativeOptions {
            viewport,
            centered: true,
            #[cfg(target_os = "linux")]
            event_loop_builder: Some(Box::new(|builder| {
                // WebKit's child-window host requires X11. Prefer XWayland
                // when available; a pure Wayland desktop keeps egui's native
                // welcome fallback instead of failing to launch.
                if std::env::var_os("DISPLAY").is_some_and(|value| !value.is_empty()) {
                    use winit::platform::x11::EventLoopBuilderExtX11;
                    builder.with_x11();
                }
            })),
            ..Default::default()
        },
        Box::new(|cc| Ok(Box::new(app::OrbitApp::new(cc)))),
    )
}
