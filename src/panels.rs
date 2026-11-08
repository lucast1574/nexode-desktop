use crate::{
    app::OrbitApp,
    backend::Command,
    settings::{data_dir, label},
    theme,
    widgets::{self, Icon},
};
use eframe::egui::{self, RichText, Stroke};
use nexode_sdk::PermissionDecision;

impl OrbitApp {
    pub fn sidebar(&mut self, ctx: &egui::Context) {
        if !self.show_sidebar {
            return;
        }
        egui::SidePanel::left("projects")
            .default_width(244.0)
            .width_range(200.0..=350.0)
            .frame(egui::Frame::new().fill(theme::SIDEBAR).inner_margin(16))
            .show(ctx, |ui| {
                ui.add_space(7.0);
                ui.horizontal(|ui| {
                    ui.add(widgets::logo(30.0));
                    ui.label(RichText::new("nexode").size(23.0).strong());
                    widgets::muted(ui, "orbit");
                    if widgets::icon_button(ui, Icon::Panel, "Ocultar proyectos").clicked() {
                        self.show_sidebar = false;
                    }
                });
                ui.add_space(22.0);
                if ui
                    .add_enabled(
                        !self.busy,
                        egui::Button::image_and_text(widgets::icon(Icon::Plus), "Nuevo chat")
                            .min_size(egui::vec2(ui.available_width(), 36.0))
                            .fill(theme::SURFACE),
                    )
                    .clicked()
                {
                    self.new_chat();
                }
                ui.add_space(10.0);
                ui.add(
                    egui::TextEdit::singleline(&mut self.search)
                        .hint_text("Buscar chats")
                        .desired_width(f32::INFINITY)
                        .frame(false),
                );
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    widgets::muted(ui, "PROYECTOS");
                    if widgets::icon_button(ui, Icon::Plus, "Añadir proyecto").clicked() {
                        self.backend.send(Command::PickFolder);
                    }
                });
                let available = (ui.available_height() - 94.0).max(60.0);
                egui::ScrollArea::vertical()
                    .max_height(available)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let projects = self.settings.projects.clone();
                        for project in projects {
                            let selected = self.settings.selected.as_ref() == Some(&project);
                            let button = egui::Button::image_and_text(
                                widgets::icon(Icon::Folder),
                                label(&project),
                            )
                            .selected(selected)
                            .min_size(egui::vec2(ui.available_width(), 34.0));
                            if ui
                                .add_enabled(!self.busy && !self.loading, button)
                                .on_hover_text(project.display().to_string())
                                .clicked()
                            {
                                self.new_chat();
                                self.settings.selected = Some(project.clone());
                                self.save();
                            }
                            let sessions: Vec<_> = self
                                .sessions
                                .iter()
                                .filter(|s| {
                                    s.working_dir.as_deref()
                                        == Some(project.to_string_lossy().as_ref())
                                        && s.title
                                            .as_deref()
                                            .unwrap_or("Nuevo chat")
                                            .to_lowercase()
                                            .contains(&self.search.to_lowercase())
                                })
                                .cloned()
                                .collect();
                            for s in sessions {
                                ui.indent(&s.session_id, |ui| {
                                    let selected = self
                                        .session
                                        .as_ref()
                                        .is_some_and(|a| a.session_id == s.session_id);
                                    let title = s.title.as_deref().unwrap_or("Nuevo chat");
                                    if ui
                                        .add_enabled(
                                            !self.busy && !self.loading,
                                            egui::Button::image_and_text(
                                                widgets::icon(Icon::Chat),
                                                RichText::new(title).size(12.0),
                                            )
                                            .selected(selected)
                                            .truncate()
                                            .min_size(egui::vec2(ui.available_width(), 30.0)),
                                        )
                                        .clicked()
                                    {
                                        self.open(s.clone());
                                    }
                                });
                            }
                            ui.add_space(5.0);
                        }
                        if self.settings.projects.is_empty() {
                            widgets::muted(ui, "Abre una carpeta para comenzar.");
                        }
                    });
                ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                    if widgets::action(ui, Icon::Settings, "Configuración").clicked() {
                        self.show_settings = true;
                    }
                    ui.horizontal(|ui| {
                        let color = if self.connected {
                            theme::ACCENT
                        } else {
                            theme::MUTED
                        };
                        let (rect, _) =
                            ui.allocate_exact_size(egui::vec2(7.0, 10.0), egui::Sense::hover());
                        ui.painter().circle_filled(rect.center(), 2.5, color);
                        widgets::muted(
                            ui,
                            if self.connecting {
                                "Conectando motor…"
                            } else if self.connected {
                                "Motor conectado"
                            } else {
                                "Motor desconectado"
                            },
                        );
                    });
                    ui.separator();
                });
            });
    }
    pub fn header(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("header")
            .exact_height(58.0)
            .frame(
                egui::Frame::new()
                    .fill(theme::CANVAS)
                    .inner_margin(egui::Margin::symmetric(22, 12)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if !self.show_sidebar
                        && widgets::icon_button(ui, Icon::Panel, "Mostrar proyectos").clicked()
                    {
                        self.show_sidebar = true;
                    }
                    ui.add(widgets::icon(Icon::Folder));
                    ui.label(
                        self.settings
                            .selected
                            .as_ref()
                            .map(|p| label(p))
                            .unwrap_or("Selecciona un proyecto".into()),
                    );
                    widgets::muted(ui, " / ");
                    ui.add(
                        egui::Label::new(
                            RichText::new(
                                self.session
                                    .as_ref()
                                    .and_then(|s| s.title.as_deref())
                                    .unwrap_or("Nuevo chat"),
                            )
                            .color(theme::MUTED),
                        )
                        .truncate(),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if widgets::action(ui, Icon::Changes, "Cambios").clicked() {
                            self.show_changes = !self.show_changes;
                            if self.show_changes {
                                self.refresh_changes();
                            }
                        }
                    });
                });
            });
        egui::TopBottomPanel::bottom("status")
            .exact_height(28.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.add(widgets::icon(Icon::Branch));
                    widgets::muted(
                        ui,
                        self.changes
                            .as_ref()
                            .map(|c| c.branch.trim())
                            .unwrap_or("Proyecto local"),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        widgets::muted(ui, "Rust nativo · GPU");
                    });
                });
            });
    }
    pub fn composer(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("composer")
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(theme::CANVAS)
                    .inner_margin(egui::Margin::symmetric(24, 17)),
            )
            .show(ctx, |ui| {
                let max = ui.available_width().min(820.0);
                ui.vertical_centered(|ui| {
                    ui.set_max_width(max);
                    if let Some(error) = self.error.clone() {
                        egui::Frame::new()
                            .fill(egui::Color32::from_rgb(58, 39, 31))
                            .corner_radius(7)
                            .inner_margin(10)
                            .show(ui, |ui| {
                                ui.horizontal_wrapped(|ui| {
                                    ui.colored_label(egui::Color32::from_rgb(226, 178, 151), error);
                                    if widgets::icon_button(ui, Icon::Close, "Cerrar error")
                                        .clicked()
                                    {
                                        self.error = None;
                                    }
                                    if !self.connected && ui.button("Configurar motor").clicked() {
                                        self.show_settings = true;
                                    }
                                });
                            });
                        ui.add_space(8.0);
                    }
                    if let Some((request, description)) = self.permission.clone() {
                        egui::Frame::new()
                            .fill(theme::SURFACE)
                            .inner_margin(12)
                            .show(ui, |ui| {
                                ui.horizontal_wrapped(|ui| {
                                    ui.add(widgets::icon(Icon::Shield));
                                    ui.label(description);
                                    for (text, decision) in [
                                        ("Permitir", PermissionDecision::Allow),
                                        ("Denegar", PermissionDecision::Deny),
                                    ] {
                                        if ui.button(text).clicked()
                                            && let Some(s) = &self.session
                                        {
                                            self.backend.send(Command::Permission(
                                                s.session_id.clone(),
                                                request.clone(),
                                                decision,
                                            ));
                                            self.permission = None;
                                        }
                                    }
                                });
                            });
                    }
                    egui::Frame::new()
                        .fill(theme::SURFACE)
                        .corner_radius(13)
                        .inner_margin(16)
                        .stroke(Stroke::new(1.0_f32, theme::LINE))
                        .show(ui, |ui| {
                            let edit = ui.add(
                                egui::TextEdit::multiline(&mut self.draft)
                                    .id(egui::Id::new("prompt"))
                                    .hint_text(
                                        "Pídele a Orbit que construya, revise o resuelva algo…",
                                    )
                                    .desired_rows(3)
                                    .desired_width(f32::INFINITY)
                                    .frame(false)
                                    .return_key(egui::KeyboardShortcut::new(
                                        egui::Modifiers::SHIFT,
                                        egui::Key::Enter,
                                    )),
                            );
                            if self.focus_composer {
                                edit.request_focus();
                                self.focus_composer = false;
                            }
                            if edit.has_focus()
                                && ui.input_mut(|i| {
                                    i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
                                })
                            {
                                self.send();
                            }
                            ui.add_space(8.0);
                            ui.horizontal(|ui| {
                                ui.add_enabled_ui(
                                    !self.busy && !self.loading && self.session.is_none(),
                                    |ui| {
                                        egui::ComboBox::from_id_salt("project_picker")
                                            .width(160.0)
                                            .selected_text(
                                                self.settings
                                                    .selected
                                                    .as_ref()
                                                    .map(|p| label(p))
                                                    .unwrap_or("Proyecto".into()),
                                            )
                                            .show_ui(ui, |ui| {
                                                for p in &self.settings.projects {
                                                    ui.selectable_value(
                                                        &mut self.settings.selected,
                                                        Some(p.clone()),
                                                        label(p),
                                                    );
                                                }
                                                if widgets::action(ui, Icon::Plus, "Abrir carpeta")
                                                    .clicked()
                                                {
                                                    self.backend.send(Command::PickFolder);
                                                    ui.close();
                                                }
                                            });
                                    },
                                );
                                if let Some(s) = &self.session {
                                    let id = s.session_id.clone();
                                    ui.add_enabled_ui(!self.busy, |ui| {
                                        egui::ComboBox::from_id_salt("models")
                                            .width(165.0)
                                            .selected_text(if self.model.is_empty() {
                                                "Modelo de Orbit"
                                            } else {
                                                &self.model
                                            })
                                            .show_ui(ui, |ui| {
                                                for model in &self.models {
                                                    if ui
                                                        .selectable_label(
                                                            self.model == *model,
                                                            model,
                                                        )
                                                        .clicked()
                                                    {
                                                        self.backend.send(Command::Model(
                                                            id.clone(),
                                                            model.clone(),
                                                        ));
                                                        ui.close();
                                                    }
                                                }
                                                ui.separator();
                                                if widgets::action(
                                                    ui,
                                                    Icon::Refresh,
                                                    "Actualizar modelos",
                                                )
                                                .clicked()
                                                {
                                                    self.backend.send(Command::Models(id.clone()));
                                                    ui.close();
                                                }
                                            });
                                    });
                                } else {
                                    widgets::muted(ui, "Modelo de Orbit");
                                }
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if self.busy {
                                            if widgets::icon_button(
                                                ui,
                                                Icon::Stop,
                                                "Detener respuesta",
                                            )
                                            .clicked()
                                                && let Some(s) = &self.session
                                            {
                                                self.backend
                                                    .send(Command::Cancel(s.session_id.clone()));
                                            }
                                        } else if ui
                                            .add_enabled(
                                                self.connected
                                                    && !self.loading
                                                    && !self.draft.trim().is_empty()
                                                    && self.settings.selected.is_some(),
                                                egui::Button::image(
                                                    widgets::icon(Icon::Send).tint(theme::SIDEBAR),
                                                )
                                                .fill(theme::ACCENT)
                                                .min_size(egui::vec2(34.0, 32.0)),
                                            )
                                            .on_hover_text("Enviar mensaje")
                                            .clicked()
                                        {
                                            self.send();
                                        }
                                    },
                                );
                            });
                        });
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        widgets::muted(ui, "Entorno local · Permisos controlados por Orbit");
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            widgets::muted(
                                ui,
                                "Enter para enviar · Shift + Enter para nueva línea",
                            );
                        });
                    });
                });
            });
    }
    pub fn review_panel(&mut self, ctx: &egui::Context) {
        if !self.show_changes {
            return;
        }
        egui::SidePanel::right("review")
            .default_width(340.0)
            .width_range(250.0..=600.0)
            .frame(egui::Frame::new().fill(theme::SIDEBAR).inner_margin(16))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.add(widgets::icon(Icon::Changes));
                    ui.strong("Cambios del proyecto");
                    if widgets::icon_button(ui, Icon::Close, "Cerrar cambios").clicked() {
                        self.show_changes = false;
                    }
                });
                ui.add_space(24.0);
                ui.horizontal(|ui| {
                    widgets::muted(ui, "ÁRBOL DE TRABAJO");
                    if widgets::icon_button(ui, Icon::Refresh, "Actualizar cambios").clicked() {
                        self.refresh_changes();
                    }
                });
                if self.changes_loading {
                    widgets::muted(ui, "Leyendo Git…");
                }
                if let Some(error) = &self.changes_error {
                    ui.colored_label(egui::Color32::LIGHT_RED, error);
                }
                egui::ScrollArea::both().id_salt("diff").show(ui, |ui| {
                    if let Some(c) = &self.changes {
                        ui.label(
                            RichText::new(if c.status.is_empty() {
                                "No hay cambios pendientes."
                            } else {
                                &c.status
                            })
                            .monospace()
                            .size(12.0),
                        );
                        ui.add_space(16.0);
                        for line in c.diff.lines() {
                            let color = if line.starts_with('+') {
                                egui::Color32::from_rgb(150, 201, 143)
                            } else if line.starts_with('-') {
                                egui::Color32::from_rgb(218, 153, 137)
                            } else {
                                theme::MUTED
                            };
                            ui.add(
                                egui::Label::new(
                                    RichText::new(line).monospace().size(11.0).color(color),
                                )
                                .extend(),
                            );
                        }
                    } else if !self.changes_loading {
                        ui.add_space(80.0);
                        ui.label("El código, a la vista");
                        widgets::muted(ui, "Abre un chat para revisar su proyecto.");
                    }
                });
            });
    }
    pub fn settings_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_settings;
        egui::Window::new("Configuración").open(&mut open).collapsible(false).resizable(false).default_width(490.0).show(ctx,|ui|{
            ui.add(widgets::logo(42.0));ui.heading("Tu espacio, tu motor.");ui.add_space(8.0);
            ui.label("Aplicación nativa en Rust. Tus conversaciones y archivos permanecen en tu equipo.");ui.add_space(12.0);
            ui.label(RichText::new("Ejecutable de Orbit").strong());
            ui.label(self.settings.binary.as_ref().map(|p|p.display().to_string()).unwrap_or("Detectar junto a la aplicación o en PATH".into()));
            if ui.add_enabled(!self.busy&&!self.connecting,egui::Button::new("Seleccionar ejecutable…")).clicked(){self.backend.send(Command::PickBinary);}
            ui.add_space(8.0);
            if ui.add_enabled(!self.busy&&!self.connecting,egui::Button::new(if self.connecting{"Conectando…"}else{"Conectar motor"})).clicked(){self.connecting=true;self.error=None;self.backend.send(Command::Connect(self.settings.clone()));}
            ui.separator();ui.label(RichText::new("Proveedor y credenciales").strong());
            ui.label("Orbit hereda la configuración de tu usuario. Configura el proveedor en Orbit para conversar con modelos reales. Esta ventana no guarda claves.");
            widgets::muted(ui,format!("Datos locales: {}",data_dir().display()));
            ui.add_space(8.0);widgets::muted(ui,"Nexode Desktop 0.2 · Sin navegador ni servidor web");
        });
        self.show_settings = open;
    }
}
