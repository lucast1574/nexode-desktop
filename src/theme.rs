use eframe::egui::{self, Color32, FontId, Stroke, TextStyle};
pub const CANVAS: Color32 = Color32::from_rgb(25, 27, 25);
pub const SIDEBAR: Color32 = Color32::from_rgb(20, 22, 20);
pub const SURFACE: Color32 = Color32::from_rgb(35, 39, 35);
pub const MUTED: Color32 = Color32::from_rgb(143, 153, 136);
pub const TEXT: Color32 = Color32::from_rgb(226, 233, 221);
pub const ACCENT: Color32 = Color32::from_rgb(16, 220, 121);
pub const LINE: Color32 = Color32::from_rgb(43, 48, 40);
pub fn apply(ctx: &egui::Context) {
    let mut style = egui::Style {
        visuals: egui::Visuals::dark(),
        ..Default::default()
    };
    style.visuals.panel_fill = CANVAS;
    style.visuals.window_fill = SURFACE;
    style.visuals.extreme_bg_color = SIDEBAR;
    style.visuals.override_text_color = Some(TEXT);
    style.visuals.selection.bg_fill = Color32::from_rgb(49, 79, 55);
    style.visuals.selection.stroke = Stroke::new(0.0_f32, TEXT);
    style.visuals.hyperlink_color = ACCENT;
    for widget in [
        &mut style.visuals.widgets.inactive,
        &mut style.visuals.widgets.hovered,
        &mut style.visuals.widgets.active,
        &mut style.visuals.widgets.open,
    ] {
        widget.bg_stroke = Stroke::NONE;
        widget.corner_radius = 6.into();
    }
    style.visuals.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
    style.visuals.widgets.inactive.bg_fill = SURFACE;
    style.visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(44, 51, 40);
    style.visuals.widgets.active.weak_bg_fill = Color32::from_rgb(55, 65, 49);
    style.visuals.widgets.open.weak_bg_fill = Color32::from_rgb(48, 58, 43);
    style.visuals.window_stroke = Stroke::new(1.0_f32, LINE);
    style.spacing.item_spacing = egui::vec2(9.0, 10.0);
    style.spacing.button_padding = egui::vec2(10.0, 8.0);
    style.spacing.interact_size.y = 30.0;
    style
        .text_styles
        .insert(TextStyle::Body, FontId::proportional(14.0));
    style
        .text_styles
        .insert(TextStyle::Button, FontId::proportional(13.0));
    style
        .text_styles
        .insert(TextStyle::Small, FontId::proportional(11.0));
    style
        .text_styles
        .insert(TextStyle::Heading, FontId::proportional(27.0));
    style.animation_time = 0.0;
    ctx.set_style(style);
    egui_extras::install_image_loaders(ctx);
}
