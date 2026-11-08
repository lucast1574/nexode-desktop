use crate::theme;
use eframe::egui::{self, Image, RichText};

pub fn logo(size: f32) -> Image<'static> {
    Image::new(egui::include_image!("../assets/nexode.svg"))
        .fit_to_exact_size(egui::vec2(size, size))
}
#[derive(Clone, Copy)]
pub enum Icon {
    Plus,
    Folder,
    Chat,
    Changes,
    Settings,
    Refresh,
    Send,
    Stop,
    Close,
    Panel,
    Branch,
    Code,
    Search,
    Shield,
}
pub fn icon(kind: Icon) -> Image<'static> {
    let (name, path) = match kind {
        Icon::Plus => ("plus", r#"<path d="M12 5v14M5 12h14"/>"#),
        Icon::Folder => (
            "folder",
            r#"<path d="M3 7V5a2 2 0 0 1 2-2h5l2 3h7a2 2 0 0 1 2 2v11a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V7Z"/>"#,
        ),
        Icon::Chat => (
            "chat",
            r#"<path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2Z"/>"#,
        ),
        Icon::Changes => (
            "changes",
            r#"<path d="M6 3v12m12 6V9M3 12l3 3 3-3m6 0 3-3 3 3M6 3h12M6 21h12"/>"#,
        ),
        Icon::Settings => (
            "settings",
            r#"<path d="M4 6h16M4 12h16M4 18h16"/><circle cx="8" cy="6" r="2"/><circle cx="16" cy="12" r="2"/><circle cx="10" cy="18" r="2"/>"#,
        ),
        Icon::Refresh => ("refresh", r#"<path d="M20 7A8 8 0 1 0 20 17M20 3v5h-5"/>"#),
        Icon::Send => ("send", r#"<path d="M12 20V4m-6 6 6-6 6 6"/>"#),
        Icon::Stop => (
            "stop",
            r#"<rect x="6" y="6" width="12" height="12" rx="2"/>"#,
        ),
        Icon::Close => ("close", r#"<path d="m6 6 12 12M6 18 18 6"/>"#),
        Icon::Panel => (
            "panel",
            r#"<rect x="3" y="3" width="18" height="18" rx="2"/><path d="M9 3v18"/>"#,
        ),
        Icon::Branch => (
            "branch",
            r#"<circle cx="6" cy="5" r="2"/><circle cx="6" cy="19" r="2"/><circle cx="18" cy="5" r="2"/><path d="M6 7v10m12-10a10 10 0 0 1-10 10"/>"#,
        ),
        Icon::Code => (
            "code",
            r#"<path d="m8 6-6 6 6 6m8-12 6 6-6 6m-3-16-2 20"/>"#,
        ),
        Icon::Search => (
            "search",
            r#"<circle cx="10" cy="10" r="6"/><path d="m15 15 6 6"/>"#,
        ),
        Icon::Shield => (
            "shield",
            r#"<path d="m12 2 9 4v6c0 6-9 10-9 10S3 18 3 12V6Zm-5 10 3 3 7-7"/>"#,
        ),
    };
    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="24" height="24" fill="none" stroke="#a9b5a1" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">{path}</svg>"##
    );
    Image::from_bytes(format!("bytes://{name}.svg"), svg.into_bytes())
        .fit_to_exact_size(egui::vec2(16.0, 16.0))
}
pub fn icon_button(ui: &mut egui::Ui, kind: Icon, label: &str) -> egui::Response {
    ui.add(egui::Button::image(icon(kind)).frame(false))
        .on_hover_text(label)
}
pub fn action(ui: &mut egui::Ui, kind: Icon, text: &str) -> egui::Response {
    ui.add(egui::Button::image_and_text(icon(kind), text))
}
pub fn muted(ui: &mut egui::Ui, text: impl Into<String>) {
    ui.label(RichText::new(text).color(theme::MUTED).size(11.0));
}
