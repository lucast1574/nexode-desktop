use crate::{
    app::OrbitApp,
    backend::Command,
    settings::{label, project_key, same_project, session_matches_project},
    theme,
    widgets::{self, Icon},
};
use eframe::egui::{self, RichText};
use nexode_sdk::SessionInfo;

impl OrbitApp {
    pub fn sidebar(&mut self, ctx: &egui::Context) {
        if !self.show_sidebar {
            return;
        }
        egui::SidePanel::left("projects")
            .default_width(250.0)
            .width_range(220.0..=350.0)
            .frame(egui::Frame::new().fill(theme::SIDEBAR).inner_margin(14))
            .show(ctx, |ui| {
                egui::TopBottomPanel::bottom("sidebar-account-footer")
                    .frame(egui::Frame::new().fill(theme::SIDEBAR).inner_margin(0))
                    .show_inside(ui, |ui| self.account_footer(ui));
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
                        !self.busy && !self.loading,
                        egui::Button::image_and_text(widgets::icon(Icon::Plus), "Nuevo chat")
                            .min_size(egui::vec2(ui.available_width(), 36.0))
                            .fill(theme::SURFACE),
                    )
                    .clicked()
                {
                    self.settings.selected = None;
                    self.save();
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
                    widgets::muted(ui, "CONVERSACIONES");
                    if widgets::icon_button(ui, Icon::Plus, "Añadir proyecto").clicked() {
                        self.backend.send(Command::PickFolder);
                    }
                    ui.add_enabled_ui(self.connected && !self.busy && !self.loading, |ui| {
                        if widgets::icon_button(ui, Icon::Refresh, "Actualizar chats").clicked() {
                            self.backend.send(Command::ListSessions);
                        }
                    });
                });
                let available = ui.available_height().max(0.0);
                let selected_key = self.settings.selected.as_deref().map(project_key);
                let selection_memory = ui.make_persistent_id("sidebar-last-selected-project");
                let reveal_selected = ui.data_mut(|data| {
                    let previous = data.get_temp::<Option<String>>(selection_memory);
                    let changed = previous.as_ref() != Some(&selected_key);
                    data.insert_temp(selection_memory, selected_key);
                    changed
                });
                egui::ScrollArea::vertical()
                    .max_height(available)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let search = self.search.trim().to_lowercase();
                        let matches_search = |s: &SessionInfo| {
                            s.title
                                .as_deref()
                                .unwrap_or("Nuevo chat")
                                .to_lowercase()
                                .contains(&search)
                        };
                        let pinned: Vec<_> = self
                            .settings
                            .pinned_sessions
                            .iter()
                            .filter_map(|id| self.sessions.iter().find(|s| &s.session_id == id))
                            .filter(|s| matches_search(s))
                            .cloned()
                            .collect();
                        if !pinned.is_empty() {
                            widgets::muted(ui, "FIJADOS");
                            for session in pinned {
                                self.sidebar_chat_row(ui, session);
                            }
                            ui.add_space(14.0);
                        }
                        widgets::muted(ui, "CHATS GENERALES");
                        let general_dir = crate::app::general_chat_dir();
                        let general: Vec<_> = self
                            .sessions
                            .iter()
                            .filter(|s| session_matches_project(s, &general_dir))
                            .filter(|s| {
                                !self.settings.is_session_pinned(&s.session_id) && matches_search(s)
                            })
                            .cloned()
                            .collect();
                        if general.is_empty() {
                            widgets::muted(
                                ui,
                                if search.is_empty() {
                                    "Tus conversaciones aparecerán aquí."
                                } else {
                                    "Sin coincidencias."
                                },
                            );
                        }
                        for session in general {
                            self.sidebar_chat_row(ui, session);
                        }
                        ui.add_space(14.0);
                        widgets::muted(ui, "PROYECTOS");
                        let projects = self.settings.projects.clone();
                        for project in projects {
                            let selected = self
                                .settings
                                .selected
                                .as_ref()
                                .is_some_and(|p| same_project(p, &project));
                            let all_sessions: Vec<_> = self
                                .sessions
                                .iter()
                                .filter(|s| session_matches_project(s, &project))
                                .cloned()
                                .collect();
                            let search = self.search.trim().to_lowercase();
                            let sessions: Vec<_> = all_sessions
                                .iter()
                                .filter(|s| {
                                    !self.settings.is_session_pinned(&s.session_id)
                                        && s.title
                                            .as_deref()
                                            .unwrap_or("Nuevo chat")
                                            .to_lowercase()
                                            .contains(&search)
                                })
                                .cloned()
                                .collect();
                            let mut collapse =
                                egui::collapsing_header::CollapsingState::load_with_default_open(
                                    ui.ctx(),
                                    ui.make_persistent_id(("project-chats", project_key(&project))),
                                    selected,
                                );
                            if !search.is_empty() && !sessions.is_empty() {
                                collapse.set_open(true);
                            }
                            let header = ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 3.0;
                                collapse
                                    .show_toggle_button(
                                        ui,
                                        egui::collapsing_header::paint_default_icon,
                                    )
                                    .on_hover_text("Mostrar u ocultar chats del proyecto");
                                let button = egui::Button::image_and_text(
                                    widgets::icon(Icon::Folder),
                                    format!("{} · {}", label(&project), all_sessions.len()),
                                )
                                .selected(selected)
                                .truncate()
                                .min_size(egui::vec2(
                                    (ui.available_width() - 32.0).max(40.0),
                                    34.0,
                                ));
                                if ui
                                    .add_enabled(!self.busy && !self.loading, button)
                                    .on_hover_text(project.display().to_string())
                                    .clicked()
                                {
                                    collapse.set_open(true);
                                    self.settings.selected = Some(project.clone());
                                    self.save();
                                    if self.connected
                                        && let Some(session) = all_sessions.first()
                                    {
                                        self.open(session.clone());
                                    } else {
                                        self.new_chat();
                                    }
                                }
                                ui.add_enabled_ui(
                                    self.connected && !self.busy && !self.loading,
                                    |ui| {
                                        if widgets::icon_button(
                                            ui,
                                            Icon::Plus,
                                            "Nuevo chat en este proyecto",
                                        )
                                        .clicked()
                                        {
                                            collapse.set_open(true);
                                            self.settings.selected = Some(project.clone());
                                            self.save();
                                            self.new_chat();
                                        }
                                    },
                                );
                            });
                            if selected && reveal_selected {
                                header.response.scroll_to_me(Some(egui::Align::Center));
                            }
                            collapse.show_body_indented(&header.response, ui, |ui| {
                                if sessions.is_empty() {
                                    widgets::muted(
                                        ui,
                                        if all_sessions.is_empty() {
                                            "Aún no hay chats."
                                        } else if search.is_empty() {
                                            "Tus chats están en Fijados."
                                        } else {
                                            "Sin coincidencias."
                                        },
                                    );
                                    if all_sessions.is_empty()
                                        && ui
                                            .add_enabled(
                                                self.connected && !self.busy && !self.loading,
                                                egui::Button::new("Crear primer chat"),
                                            )
                                            .clicked()
                                    {
                                        self.settings.selected = Some(project.clone());
                                        self.save();
                                        self.new_chat();
                                    }
                                }
                                for session in sessions {
                                    self.sidebar_chat_row(ui, session);
                                }
                            });
                            ui.add_space(5.0);
                        }
                        if self.settings.projects.is_empty() {
                            ui.add_space(12.0);
                            widgets::muted(ui, "Las carpetas son opcionales.");
                            if widgets::action(ui, Icon::Folder, "Añadir proyecto").clicked() {
                                self.backend.send(Command::PickFolder);
                            }
                        }
                        if !self.settings.previous_projects.is_empty() {
                            ui.add_space(14.0);
                            egui::CollapsingHeader::new(format!(
                                "Recuperar proyectos anteriores ({})",
                                self.settings.previous_projects.len()
                            ))
                            .default_open(false)
                            .show(ui, |ui| {
                                widgets::muted(ui, "Elige cuáles quieres recuperar.");
                                for path in self.settings.previous_projects.clone() {
                                    if ui
                                        .add_enabled(
                                            !self.busy && !self.loading,
                                            egui::Button::new(format!("Añadir {}", label(&path)))
                                                .truncate(),
                                        )
                                        .on_hover_text(path.display().to_string())
                                        .clicked()
                                    {
                                        match self.settings.add_project(path) {
                                            Ok(()) => {
                                                self.save();
                                                let previous = self
                                                    .settings
                                                    .selected
                                                    .as_ref()
                                                    .and_then(|p| {
                                                        self.sessions
                                                            .iter()
                                                            .find(|s| session_matches_project(s, p))
                                                    })
                                                    .cloned();
                                                if self.connected
                                                    && let Some(session) = previous
                                                {
                                                    self.open(session);
                                                } else {
                                                    self.new_chat();
                                                }
                                            }
                                            Err(error) => self.error = Some(error),
                                        }
                                    }
                                }
                            });
                        }
                    });
            });
    }
    fn sidebar_chat_row(&mut self, ui: &mut egui::Ui, session: SessionInfo) {
        ui.push_id(&session.session_id, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let selected = self
                    .session
                    .as_ref()
                    .is_some_and(|s| s.session_id == session.session_id);
                let title = session.title.as_deref().unwrap_or("Nuevo chat");
                let button = egui::Button::image_and_text(
                    widgets::icon(Icon::Chat),
                    RichText::new(title).size(12.0),
                )
                .selected(selected)
                .truncate()
                .min_size(egui::vec2((ui.available_width() - 30.0).max(40.0), 30.0));
                if ui
                    .add_enabled(self.connected && !self.busy && !self.loading, button)
                    .on_hover_text(title)
                    .clicked()
                {
                    self.open(session.clone());
                }
                let pinned = self.settings.is_session_pinned(&session.session_id);
                if ui
                    .add(
                        egui::Button::image(widgets::icon(Icon::Pin))
                            .selected(pinned)
                            .frame(false)
                            .min_size(egui::vec2(26.0, 28.0)),
                    )
                    .on_hover_text(if pinned {
                        "Desfijar chat"
                    } else {
                        "Fijar chat"
                    })
                    .clicked()
                {
                    self.settings.toggle_session_pin(&session.session_id);
                    self.save();
                }
            });
        });
    }

    fn account_footer(&mut self, ui: &mut egui::Ui) {
        ui.spacing_mut().item_spacing.y = 3.0;
        ui.separator();
        ui.add_space(4.0);
        self.usage.ui(ui);
        ui.add_space(8.0);
        if sidebar_navigation(ui, Icon::Code, "Servicios", self.show_services).clicked() {
            self.show_services = true;
            self.cloud.open = false;
            self.show_settings = false;
        }
        if sidebar_navigation(ui, Icon::Settings, "Configuración", self.show_settings).clicked() {
            self.show_settings = true;
            self.cloud.open = false;
            self.show_services = false;
        }
        if sidebar_navigation(ui, Icon::Shield, "Mi cuenta", self.cloud.open).clicked() {
            self.cloud.open = true;
            self.show_services = false;
            self.show_settings = false;
        }
        ui.horizontal(|ui| {
            let color = if self.connected {
                theme::ACCENT
            } else {
                theme::MUTED
            };
            let (rect, _) = ui.allocate_exact_size(egui::vec2(7.0, 10.0), egui::Sense::hover());
            ui.painter().circle_filled(rect.center(), 2.5, color);
            widgets::muted(
                ui,
                if self.connecting {
                    "Conectando motor…"
                } else if self.connected {
                    "Orbit conectado"
                } else {
                    "Motor desconectado"
                },
            );
        });
    }
}

fn sidebar_navigation(
    ui: &mut egui::Ui,
    icon: Icon,
    label: &str,
    selected: bool,
) -> egui::Response {
    ui.add(
        egui::Button::image_and_text(widgets::icon(icon), RichText::new(label).size(12.0))
            .selected(selected)
            .min_size(egui::vec2(ui.available_width(), 30.0)),
    )
}
