//! Native account screen; authentication logic stays in cloud.rs.
use super::*;
use crate::{theme, widgets};
use egui::RichText;

impl Cloud {
    pub fn ui(&mut self, ui: &mut egui::Ui, engine_busy: bool, animate: bool) {
        let ctx = ui.ctx().clone();
        ui.style_mut().override_font_id = Some(egui::FontId::new(
            14.0,
            egui::FontFamily::Name("account".into()),
        ));
        let signed_in = self.account.is_some();
        let compact = ui.available_height() < 760.0;
        ui.spacing_mut().item_spacing.y = if compact { 4.0 } else { 7.0 };
        let width = if signed_in { 640.0 } else { 392.0 };
        let margin = ((ui.available_width() - width) / 2.0).max(20.0);
        egui::ScrollArea::vertical().auto_shrink([false, false]).id_salt("account_page").show(ui, |ui| {
            ui.add_space(if compact { 8.0 } else { 44.0 });
            ui.horizontal(|ui| {
                ui.add_space(margin);
                ui.vertical(|ui| {
                    ui.set_width(width.min(ui.available_width() - 20.0).max(240.0));
                    if signed_in && widgets::action(ui, widgets::Icon::Chat, "Volver a mis chats").clicked() {
                        self.open = false;
                    }
                    ui.vertical_centered(|ui| {
                        crate::mascot::show(ui, if compact { 48.0 } else { 64.0 }, self.receiver.is_some(), animate);
                        ui.add_space(8.0);
                        ui.label(RichText::new("nexode orbit").size(13.0).color(theme::MUTED));
                        ui.add_space(if compact { 8.0 } else { 14.0 });
                        ui.label(RichText::new(if signed_in { "Tu cuenta Nexode" } else { "Bienvenido a Orbit" }).size(if compact { 27.0 } else { 30.0 }).strong());
                        ui.add_space(6.0);
                        ui.label(RichText::new(if signed_in { "Tu workspace, servicios y créditos." } else { "Tus ideas. Tus agentes. Un mismo lugar." }).size(14.0).color(theme::MUTED));
                    });
                    ui.add_space(if compact { 16.0 } else { 24.0 });
                    let busy = self.receiver.is_some();
                    ui.add_enabled_ui((!busy || self.browser_login.is_some()) && !engine_busy, |ui| {
                        if let Some(a) = &self.account {
                            ui.strong(&a.email);
                            ui.label(format!("Workspace: {}", a.workspace));
                            ui.add_space(24.0);
                            ui.strong("Servicios y créditos");
                            if a.usage.is_null() { ui.label("El saldo no está disponible en este momento."); }
                            else { billing_summary(ui, &a.usage); }
                            let plans = a.plans.clone();
                            for plan in plans.iter().filter(|p| p["purchasable"] == true) {
                                let slug = plan["slug"].as_str().unwrap_or_default();
                                let label = format!("{} · {} USD / mes", plan["name"].as_str().unwrap_or(slug), plan["price_monthly"]);
                                if ui.button(&label).clicked() { self.selected_plan = Some((slug.into(), label)); }
                            }
                            if let Some((slug, label)) = self.selected_plan.clone() {
                                ui.add_space(if compact { 8.0 } else { 16.0 });
                                ui.label(format!("{label}. Renovación mensual. Revisa impuestos y total en Stripe."));
                                if ui.button("Preparar checkout seguro").clicked() { self.start(Action::Checkout(slug), &ctx); self.selected_plan = None; }
                            }
                            if let Some(url) = &self.checkout { ui.hyperlink_to("Abrir Stripe y revisar el pago", url); }
                            ui.add_space(24.0);
                            if ui.button("Actualizar cuenta y saldo").clicked() { self.start(Action::Restore, &ctx); }
                            ui.add_space(24.0);
                            if ui.button("Cerrar sesión en este dispositivo").clicked() { self.start(Action::Logout, &ctx); }
                        } else {
                            egui::Frame::new()
                                .fill(egui::Color32::from_rgb(30, 33, 30))
                                .corner_radius(16)
                                .inner_margin(if compact { 20.0 } else { 24.0 })
                                .show(ui, |ui| { self.sign_in_form(ui, &ctx, compact); });
                        }
                    });
                    if busy { ui.add_space(12.0); ui.horizontal(|ui| { ui.spinner(); ui.label("Conectando con Nexode…"); }); }
                    if !self.message.is_empty() { ui.add_space(12.0); ui.label(&self.message); }
                    ui.add_space(if compact { 12.0 } else { 20.0 });
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new("Tu espacio es gratuito. Conecta tus propias cuentas de IA.").size(11.0).color(theme::MUTED));
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.add_space(18.0);
                            ui.add(widgets::icon(widgets::Icon::Shield).fit_to_exact_size(egui::vec2(12.0, 12.0)));
                            ui.label(RichText::new("Credenciales protegidas en este dispositivo").size(11.0).color(theme::MUTED));
                        });
                    });
                    ui.add_space(12.0);
                });
            });
        });
    }

    fn sign_in_form(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, compact: bool) {
        if let Some(browser) = &self.browser_login {
            ui.strong("Confirma que eres tú en Nexode");
            ui.label("Inicia sesión con Google en el navegador y escribe este código para conectar Orbit.");
            ui.add_space(12.0);
            ui.label(
                RichText::new(&browser.user_code)
                    .monospace()
                    .size(28.0)
                    .color(theme::ACCENT),
            );
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if ui.button("Copiar código").clicked() {
                    ui.ctx().copy_text(browser.user_code.clone());
                }
                ui.hyperlink_to("Volver a abrir Nexode", &browser.verification_url);
            });
            ui.label(RichText::new("El código caduca en cinco minutos. Vuelve aquí al terminar; Orbit se conectará automáticamente.").size(12.0).color(theme::MUTED));
            ui.add_space(12.0);
            if ui
                .add_enabled(!browser.committing, egui::Button::new("Cancelar conexión"))
                .clicked()
            {
                self.cancel_browser_login();
            }
            return;
        }
        if ui
            .add_sized(
                [ui.available_width(), 44.0],
                egui::Button::image_and_text(
                    egui::Image::new(egui::include_image!("../assets/google.svg"))
                        .fit_to_exact_size(egui::vec2(18.0, 18.0)),
                    RichText::new("Continuar con Google")
                        .size(14.0)
                        .color(egui::Color32::from_rgb(32, 33, 36)),
                )
                .fill(egui::Color32::from_rgb(248, 249, 248))
                .corner_radius(9),
            )
            .clicked()
        {
            self.start_browser_login(ctx);
        }
        ui.add_space(if compact { 6.0 } else { 12.0 });
        ui.vertical_centered(|ui| {
            ui.label(
                RichText::new("o continúa con tu correo")
                    .size(12.0)
                    .color(theme::MUTED),
            );
        });
        ui.add_space(if compact { 6.0 } else { 12.0 });
        ui.label(RichText::new("Correo electrónico").size(12.0).strong());
        ui.add_sized(
            [ui.available_width(), 40.0],
            egui::TextEdit::singleline(&mut self.email)
                .hint_text("tu@correo.com")
                .margin(egui::vec2(12.0, if compact { 8.0 } else { 12.0 })),
        );
        ui.add_space(8.0);
        ui.label(RichText::new("Contraseña").size(12.0).strong());
        let password = ui.add_sized(
            [ui.available_width(), 40.0],
            egui::TextEdit::singleline(&mut self.password)
                .password(true)
                .hint_text("Introduce tu contraseña")
                .margin(egui::vec2(12.0, if compact { 8.0 } else { 12.0 })),
        );
        ui.add_space(if compact { 8.0 } else { 16.0 });
        let valid = self.email.contains('@') && !self.password.is_empty();
        let enter = password.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let submit = ui
            .add_enabled_ui(valid, |ui| {
                ui.add_sized(
                    [ui.available_width(), 42.0],
                    egui::Button::new(
                        RichText::new("Iniciar sesión")
                            .color(if valid { theme::CANVAS } else { theme::MUTED })
                            .strong(),
                    )
                    .fill(if valid { theme::ACCENT } else { theme::SURFACE })
                    .corner_radius(9),
                )
                .clicked()
            })
            .inner;
        if valid && (submit || enter) {
            let secret = Zeroizing::new(std::mem::take(&mut self.password));
            self.start(Action::Login(self.email.trim().into(), secret), ctx);
        }
        ui.add_space(8.0);
        if ui
            .add_enabled_ui(valid && self.password.len() >= 6, |ui| {
                ui.add_sized(
                    [ui.available_width(), if compact { 32.0 } else { 40.0 }],
                    egui::Button::new("Crear una cuenta Nexode"),
                )
                .clicked()
            })
            .inner
        {
            let secret = Zeroizing::new(std::mem::take(&mut self.password));
            self.start(Action::Register(self.email.trim().into(), secret), ctx);
        }
        ui.add_space(if compact { 4.0 } else { 8.0 });
        ui.vertical_centered(|ui| {
            ui.hyperlink_to(
                "Recuperar mi contraseña",
                "https://cloud.nexode.app/auth/login",
            );
        });
    }
}
