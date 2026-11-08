use crate::{app::OrbitApp, backend::Command, settings::data_dir, theme, widgets};
use eframe::egui::{self, RichText};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum SettingsTab {
    #[default]
    Agents,
    General,
    Engine,
}

impl OrbitApp {
    /// Page content only; the application owns sidebar and navigation.
    pub fn settings_page(&mut self, ui: &mut egui::Ui, onboarding: bool) -> bool {
        let mut complete = false;
        let width = (ui.available_width() - 32.0).clamp(0.0, 820.0);
        let inset = ((ui.available_width() - width) / 2.0).max(0.0);
        egui::ScrollArea::vertical().id_salt(("settings-page", onboarding)).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.add_space(inset);
                ui.vertical(|ui| {
                    ui.set_width(width);
                    ui.add_space(28.0);
                    widgets::muted(ui, if onboarding { "TU ESPACIO DE TRABAJO" } else { "CONFIGURACIÓN" });
                    ui.add_space(6.0);
                    ui.label(RichText::new(if onboarding { "Conecta tus agentes" } else { "A tu manera." }).size(30.0).strong());
                    ui.add_space(8.0);
                    ui.label(RichText::new(if onboarding {
                        "Trae las cuentas que ya usas o añade tus API keys. Conecta varios proveedores y elige el modelo en cada chat."
                    } else { "Tus conexiones, el comportamiento de Orbit y las opciones del motor." }).color(theme::MUTED));
                    ui.add_space(24.0);
                    if onboarding {
                        ui.horizontal_wrapped(|ui| {
                            if ui.add(egui::Button::new(RichText::new("Continuar a Orbit").color(theme::SIDEBAR)).fill(theme::ACCENT).min_size(egui::vec2(160.0, 40.0))).clicked() { complete = true; }
                            if ui.button("Omitir por ahora").clicked() { complete = true; }
                        });
                        ui.add_space(6.0);
                        widgets::muted(ui, "Podrás añadir o cambiar tus conexiones desde Configuración.");
                        ui.add_space(20.0);
                    }
                    let id = egui::Id::new("orbit-settings-tab");
                    let mut tab = ui.data(|data| data.get_temp::<SettingsTab>(id)).unwrap_or_default();
                    if onboarding { tab = SettingsTab::Agents; } else {
                        ui.horizontal_wrapped(|ui| {
                            for (value, title) in [(SettingsTab::Agents, "Agentes de IA"), (SettingsTab::General, "General"), (SettingsTab::Engine, "Motor")] {
                                ui.selectable_value(&mut tab, value, title);
                            }
                        });
                        ui.data_mut(|data| data.insert_temp(id, tab));
                        ui.add_space(22.0);
                    }
                    match tab {
                        SettingsTab::Agents => {
                            ui.add_enabled_ui(!self.busy && !self.connecting, |ui| {
                                if self.providers.ui(ui, &crate::settings::engine_binary(&self.settings), &data_dir().join("engine"), self.backend.client_snapshot(), &self.usage.accounts) { self.usage.refresh(); }
                            });
                            ui.add_space(18.0);
                            widgets::muted(ui, "No necesitas un plan de pago de Nexode para usar Orbit. Cada proveedor aplica sus propios permisos, límites y costes.");
                        }
                        SettingsTab::General => self.general_settings(ui),
                        SettingsTab::Engine => self.engine_settings(ui),
                    }
                    ui.add_space(32.0);
                });
            });
        });
        complete
    }

    fn general_settings(&mut self, ui: &mut egui::Ui) {
        section(ui, "Bandeja e inicio", |ui| {
            ui.label(if self.desktop_shell.is_some() {
                "Al cerrar la ventana, Orbit permanece en la bandeja. Elige Salir en su menú para terminar."
            } else { "La bandeja no está disponible en esta sesión. Cerrar la ventana termina Orbit." });
            ui.add_space(12.0);
            let mut enabled = self.startup.enabled();
            if ui
                .checkbox(
                    &mut enabled,
                    "Iniciar Orbit en la bandeja al entrar en mi sesión",
                )
                .changed()
            {
                match crate::desktop_shell::set_startup(enabled) {
                    Ok(()) => {
                        self.startup = crate::desktop_shell::startup_status().unwrap_or_default()
                    }
                    Err(error) => self.error = Some(error),
                }
            }
            if self.startup.disabled_by_windows {
                ui.label("Windows desactivó el inicio automático. Puedes habilitarlo en Administrador de tareas, en Aplicaciones de inicio.");
            }
            if widgets::action(ui, widgets::Icon::Refresh, "Actualizar estado").clicked() {
                match crate::desktop_shell::startup_status() {
                    Ok(status) => self.startup = status,
                    Err(error) => self.error = Some(error),
                }
            }
        });
        ui.add_space(18.0);
        section(ui, "Datos y privacidad", |ui| {
            ui.label("El historial de Orbit se guarda en este equipo. Los proveedores conectados reciben el contexto necesario para responder a tus solicitudes.");
            ui.add_space(10.0);
            widgets::muted(ui, format!("Carpeta de datos: {}", data_dir().display()));
        });
    }

    fn engine_settings(&mut self, ui: &mut egui::Ui) {
        section(ui, "Motor local", |ui| {
            ui.label(
                RichText::new(if self.connecting {
                    "Conectando…"
                } else if self.connected {
                    "Conectado"
                } else {
                    "Sin conexión"
                })
                .color(if self.connected {
                    theme::ACCENT
                } else {
                    theme::MUTED
                }),
            );
            ui.label("Orbit detecta el motor incluido. Cambia el ejecutable sólo si quieres usar otra instalación compatible.");
            ui.add_space(16.0);
            ui.strong("Ejecutable de Orbit");
            ui.label(
                self.settings
                    .binary
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "Detección automática".into()),
            );
            ui.add_space(10.0);
            ui.add_enabled_ui(!self.busy && !self.connecting, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Seleccionar ejecutable…").clicked() {
                        self.backend.send(Command::PickBinary);
                    }
                    if ui
                        .button(if self.connected {
                            "Reconectar motor"
                        } else {
                            "Conectar motor"
                        })
                        .clicked()
                    {
                        self.connecting = true;
                        self.error = None;
                        self.backend.send(Command::Connect(self.settings.clone()));
                    }
                });
            });
        });
    }
}

fn section(ui: &mut egui::Ui, title: &str, contents: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(theme::SURFACE)
        .corner_radius(12)
        .inner_margin(22.0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(title).size(18.0).strong());
            ui.add_space(12.0);
            contents(ui);
        });
}
