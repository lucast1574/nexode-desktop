#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod app;
mod backend;
mod panels;
mod settings;
mod theme;
mod widgets;
#[cfg(windows)]
mod windows_job;

fn main() -> eframe::Result {
    let Some(_instance_lock) = settings::lock_instance(&settings::data_dir())
        .map_err(|e| eframe::Error::AppCreation(Box::new(e)))?
    else {
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
            ..Default::default()
        },
        Box::new(|cc| Ok(Box::new(app::OrbitApp::new(cc)))),
    )
}
