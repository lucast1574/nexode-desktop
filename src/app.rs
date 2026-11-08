use crate::{
    backend::{Backend, Command, Event, GitChanges, Operation},
    settings::Settings,
    theme,
    widgets::{self, Icon},
};
use eframe::egui::{self, RichText};
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use nexode_sdk::{ApiEvent, HistoryMessage, SessionInfo};
use std::{collections::BTreeMap, path::PathBuf, sync::mpsc::Receiver};

#[derive(Default)]
pub struct StreamText {
    parts: Vec<(String, String)>,
}
impl StreamText {
    pub fn update(&mut self, id: Option<&str>, text: &str, replace: bool) {
        let key = id.unwrap_or("legacy");
        if let Some((_, part)) = self.parts.iter_mut().find(|(id, _)| id == key) {
            if replace {
                *part = text.into()
            } else {
                part.push_str(text)
            }
        } else {
            self.parts.push((key.into(), text.into()));
        }
    }
    pub fn text(&self) -> String {
        self.parts
            .iter()
            .filter(|(_, s)| !s.is_empty())
            .map(|(_, s)| s.as_str())
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}
pub struct OrbitApp {
    pub providers: crate::provider_connections::ProviderConnections,
    pub desktop_shell: Option<crate::desktop_shell::DesktopShell>,
    pub startup: crate::desktop_shell::StartupStatus,
    pub cloud: crate::cloud::Cloud,
    pub usage: crate::provider_usage::UsageMonitor,
    pub show_services: bool,
    pub service_query: String,
    pub cloud_reconnect_pending: bool,
    pub settings: Settings,
    pub backend: Backend,
    receiver: Receiver<Event>,
    pub sessions: Vec<SessionInfo>,
    pub session: Option<SessionInfo>,
    pub messages: Vec<HistoryMessage>,
    pub draft: String,
    pub search: String,
    pub connected: bool,
    pub connecting: bool,
    pub busy: bool,
    pub loading: bool,
    pub show_sidebar: bool,
    pub show_changes: bool,
    pub show_settings: bool,
    pub models: Vec<String>,
    pub model: String,
    pub error: Option<String>,
    pub changes: Option<GitChanges>,
    pub changes_loading: bool,
    pub changes_error: Option<String>,
    pub activity: String,
    pub permission: Option<(String, String)>,
    pub stream: StreamText,
    pub tools: BTreeMap<String, (String, String)>,
    pub markdown: CommonMarkCache,
    pub pending: Option<String>,
    pub focus_composer: bool,
}
impl OrbitApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::apply(&cc.egui_ctx);
        let ctx = cc.egui_ctx.clone();
        let (backend, receiver) = Backend::new(move || ctx.request_repaint());
        let settings = Settings::load();
        let shell = crate::desktop_shell::DesktopShell::new(&cc.egui_ctx);
        let shell_error = shell.as_ref().err().cloned();
        backend.send(Command::Connect(settings.clone()));
        Self {
            providers: Default::default(),
            desktop_shell: shell.ok(),
            startup: crate::desktop_shell::startup_status().unwrap_or_default(),
            cloud: crate::cloud::Cloud::new(&cc.egui_ctx),
            usage: Default::default(),
            show_services: false,
            service_query: String::new(),
            cloud_reconnect_pending: false,
            settings,
            backend,
            receiver,
            sessions: vec![],
            session: None,
            messages: vec![],
            draft: String::new(),
            search: String::new(),
            connected: false,
            connecting: true,
            busy: false,
            loading: false,
            show_sidebar: true,
            show_changes: false,
            show_settings: false,
            models: vec![],
            model: String::new(),
            error: shell_error,
            changes: None,
            changes_loading: false,
            changes_error: None,
            activity: String::new(),
            permission: None,
            stream: Default::default(),
            tools: Default::default(),
            markdown: Default::default(),
            pending: None,
            focus_composer: false,
        }
    }
    pub fn save(&mut self) {
        if let Err(e) = self.settings.save() {
            self.error = Some(e);
        }
    }
    pub fn new_chat(&mut self) {
        if self.busy {
            return;
        }
        self.session = None;
        self.pending = None;
        self.messages.clear();
        self.stream = Default::default();
        self.models.clear();
        self.model.clear();
        self.changes = None;
        self.error = None;
        self.loading = false;
        self.tools.clear();
        self.focus_composer = true;
    }
    pub fn open(&mut self, session: SessionInfo) {
        if self.busy || self.loading {
            return;
        }
        self.pending = Some(session.session_id.clone());
        self.loading = true;
        self.messages.clear();
        self.error = None;
        self.changes = None;
        self.models.clear();
        self.tools.clear();
        self.stream = Default::default();
        self.settings.selected = session.working_dir.as_ref().map(PathBuf::from);
        self.backend.send(Command::Open(session.session_id.clone()));
        self.session = Some(session);
        self.save();
    }
    pub fn send(&mut self) {
        if self.busy || self.loading || !self.connected || self.draft.trim().is_empty() {
            return;
        }
        let Some(path) = self.settings.selected.clone() else {
            self.error = Some("Selecciona una carpeta de proyecto".into());
            return;
        };
        let content = std::mem::take(&mut self.draft);
        self.messages.push(HistoryMessage {
            role: "user".into(),
            content: content.clone(),
            response_stats: None,
        });
        self.busy = true;
        self.error = None;
        self.activity = "Preparando el contexto".into();
        self.stream = Default::default();
        self.tools.clear();
        self.backend.send(Command::Send {
            session: self.session.as_ref().map(|s| s.session_id.clone()),
            path,
            content,
        });
    }
    pub fn refresh_changes(&mut self) {
        if let Some(s) = &self.session
            && let Some(path) = &s.working_dir
        {
            self.changes_loading = true;
            self.changes_error = None;
            self.backend
                .send(Command::Changes(s.session_id.clone(), path.into()));
        }
    }
    fn matches(&self, id: &str) -> bool {
        self.session.as_ref().is_some_and(|s| s.session_id == id)
    }
    fn events(&mut self) {
        while let Ok(event) = self.receiver.try_recv() {
            match event {
                Event::Connected(sessions) => {
                    self.connected = true;
                    self.connecting = false;
                    self.sessions = sessions;
                    self.error = None;
                }
                Event::Created(session) => {
                    self.sessions.insert(0, session.clone());
                    self.session = Some(session);
                }
                Event::Opened(session, history) => {
                    if self.pending.as_deref() == Some(&session.session_id) {
                        self.messages = history;
                        self.session = Some(session);
                        self.loading = false;
                        self.pending = None;
                        if self.show_changes {
                            self.refresh_changes();
                        }
                    }
                }
                Event::Models(id, models, current) => {
                    if self.matches(&id) {
                        self.models = models;
                        self.model = current.unwrap_or_default();
                    }
                }
                Event::Finished(id, history, sessions) => {
                    self.usage.refresh();
                    self.sessions = sessions;
                    if self.matches(&id) {
                        self.messages = history;
                        self.busy = false;
                        self.permission = None;
                        self.stream = Default::default();
                        self.activity.clear();
                        if self.show_changes {
                            self.refresh_changes();
                        }
                    }
                }
                Event::Stream(id, event) => {
                    if self.matches(&id) {
                        self.stream_event(event);
                    }
                }
                Event::Changes(id, changes) => {
                    if self.matches(&id) {
                        self.changes = Some(changes);
                        self.changes_loading = false;
                    }
                }
                Event::Folder(path) => {
                    if !self.settings.projects.contains(&path) {
                        self.settings.projects.push(path.clone());
                    }
                    if !self.busy {
                        self.new_chat();
                        self.settings.selected = Some(path);
                    }
                    self.save();
                }
                Event::Binary(path) => {
                    self.settings.binary = Some(path);
                    self.save();
                }
                Event::Error(op, message) => {
                    if op == Operation::Changes {
                        self.changes_error = Some(message);
                        self.changes_loading = false;
                        continue;
                    }
                    self.error = Some(message);
                    match op {
                        Operation::Connect => {
                            self.connected = false;
                            self.connecting = false;
                        }
                        Operation::Open => self.loading = false,
                        Operation::Turn => {
                            self.busy = false;
                            self.permission = None;
                            self.activity.clear();
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    fn stream_event(&mut self, event: ApiEvent) {
        match event {
            ApiEvent::TextDelta {
                message_id, text, ..
            } => {
                self.stream.update(message_id.as_deref(), &text, false);
                self.activity = "Escribiendo".into();
            }
            ApiEvent::TextReplace {
                message_id, text, ..
            } => self.stream.update(message_id.as_deref(), &text, true),
            ApiEvent::ToolStart { call_id, name, .. }
            | ApiEvent::ToolExec { call_id, name, .. } => {
                self.activity = format!("Ejecutando {name}");
                self.tools.insert(call_id, (name, "En curso".into()));
            }
            ApiEvent::ToolDone {
                call_id,
                name,
                output,
                error,
                ..
            } => {
                self.activity = format!("Completado: {name}");
                self.tools.insert(call_id, (name, error.unwrap_or(output)));
            }
            ApiEvent::PermissionRequest {
                request_id,
                description,
                ..
            } => self.permission = Some((request_id, description)),
            ApiEvent::Error { message, .. } => self.error = Some(message),
            _ => {}
        }
    }
    fn conversation(&mut self, ui: &mut egui::Ui) {
        let width = (ui.available_width() - 40.0).clamp(100.0, 820.0);
        let margin = ((ui.available_width() - width) / 2.0).max(20.0);
        let welcome_top = (ui.available_height() * 0.22).max(24.0);
        egui::ScrollArea::vertical()
            .id_salt("conversation")
            .stick_to_bottom(true)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.add_space(margin);
                    ui.vertical(|ui| {
                        ui.set_width(width);
                        egui::Frame::new()
                            .inner_margin(egui::Margin::symmetric(0, 24))
                            .show(ui, |ui| {
                                ui.set_max_width(width);
                                if self.messages.is_empty() && !self.loading {
                                    self.welcome(ui, welcome_top);
                                }
                                if self.loading {
                                    widgets::muted(ui, "Cargando conversación…");
                                }
                                for (index, m) in self
                                    .messages
                                    .iter()
                                    .enumerate()
                                    .filter(|(_, m)| m.role == "user" || m.role == "assistant")
                                {
                                    ui.push_id(index, |ui| {
                                        let is_user = m.role == "user";
                                        egui::Frame::new()
                                            .fill(if is_user {
                                                theme::SURFACE
                                            } else {
                                                egui::Color32::TRANSPARENT
                                            })
                                            .corner_radius(12)
                                            .inner_margin(16)
                                            .show(ui, |ui| {
                                                ui.horizontal(|ui| {
                                                    if !is_user {
                                                        ui.add(widgets::logo(19.0));
                                                    }
                                                    ui.label(
                                                        RichText::new(if is_user {
                                                            "Tú"
                                                        } else {
                                                            "Orbit"
                                                        })
                                                        .small()
                                                        .strong(),
                                                    );
                                                });
                                                ui.add_space(6.0);
                                                CommonMarkViewer::new().show(
                                                    ui,
                                                    &mut self.markdown,
                                                    &m.content,
                                                );
                                            });
                                        ui.add_space(20.0);
                                    });
                                }
                                if !self.tools.is_empty() {
                                    egui::CollapsingHeader::new(format!(
                                        "Actividad · {} herramientas",
                                        self.tools.len()
                                    ))
                                    .show(ui, |ui| {
                                        for (name, output) in self.tools.values() {
                                            ui.label(RichText::new(name).strong());
                                            ui.add(
                                                egui::Label::new(
                                                    RichText::new(output).monospace().size(11.0),
                                                )
                                                .wrap(),
                                            );
                                        }
                                    });
                                }
                                if self.busy || !self.stream.text().is_empty() {
                                    ui.horizontal(|ui| {
                                        ui.add(widgets::logo(20.0));
                                        ui.strong("Orbit");
                                        widgets::muted(ui, &self.activity);
                                    });
                                    CommonMarkViewer::new().show(
                                        ui,
                                        &mut self.markdown,
                                        &self.stream.text(),
                                    );
                                }
                            });
                    });
                });
            });
    }
    fn welcome(&mut self, ui: &mut egui::Ui, top: f32) {
        ui.add_space(top);
        ui.add(widgets::mascot(96.0));
        ui.add_space(22.0);
        widgets::muted(ui, "TU ESPACIO PARA CONSTRUIR");
        ui.add_space(8.0);
        ui.label(RichText::new("¿Qué hacemos hoy?").size(36.0));
        ui.add_space(6.0);
        ui.label(
            RichText::new("Tus agentes, tus proyectos y todo Nexode en un mismo lugar.")
                .color(theme::MUTED),
        );
        ui.add_space(28.0);
        ui.horizontal(|ui| {
            if self.cloud.account.is_none() && ui.button("Conectar mi cuenta Nexode").clicked() {
                self.cloud.open = true;
            }
            if ui.button("Conectar mis agentes").clicked() {
                self.show_settings = true;
            }
            if ui.button("Explorar servicios").clicked() {
                self.show_services = true;
            }
        });
        ui.add_space(12.0);
        ui.horizontal_wrapped(|ui|{
            for(icon,title,prompt)in[(Icon::Code,"Entender este proyecto","Explora la estructura del proyecto y explícame cómo funciona, sin modificar archivos."),(Icon::Changes,"Revisar los cambios","Revisa los cambios de Git y señala posibles errores. No modifiques archivos."),(Icon::Search,"Encontrar qué mejorar","Analiza el proyecto y propón tres mejoras concretas. No modifiques archivos.")]{
                if widgets::action(ui,icon,title).clicked(){self.draft=prompt.into();self.focus_composer=true;}
            }
        });
    }
}
impl eframe::App for OrbitApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.events();
        if let Some(shell) = &mut self.desktop_shell {
            shell.update(ctx);
        }
        if self.providers.poll() {
            self.usage.refresh();
            if let Some(session) = &self.session {
                self.backend
                    .send(Command::Models(session.session_id.clone()));
            }
        }
        if self.cloud.poll() {
            self.cloud_reconnect_pending = true;
        }
        if self.cloud_reconnect_pending
            && !self.busy
            && !self.connecting
            && !self.providers.is_busy()
        {
            self.cloud_reconnect_pending = false;
            self.connecting = true;
            self.backend.send(Command::Connect(self.settings.clone()));
        }
        if self.connected {
            self.usage.update(
                ctx,
                &crate::settings::engine_binary(&self.settings),
                &crate::settings::data_dir().join("engine"),
            );
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::N)) {
            self.new_chat();
        }
        self.sidebar(ctx);
        self.review_panel(ctx);
        self.header(ctx);
        self.composer(ctx);
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::CANVAS))
            .show(ctx, |ui| self.conversation(ui));
        self.settings_window(ctx);
        self.cloud.ui(
            ctx,
            self.busy || self.connecting || self.providers.is_busy(),
        );
        crate::ecosystem::panel(
            ctx,
            &mut self.show_services,
            &mut self.service_query,
            &mut self.draft,
        );
    }
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.save();
        self.backend.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stream_replacements_retract_and_preserve_message_order() {
        let mut s = StreamText::default();
        s.update(Some("b"), "first", false);
        s.update(Some("a"), "second", false);
        s.update(Some("b"), "", true);
        assert_eq!(s.text(), "second");
        s.update(Some("b"), "corrected", true);
        assert_eq!(s.text(), "corrected\n\nsecond");
    }
    #[test]
    fn legacy_stream_appends_without_ids() {
        let mut s = StreamText::default();
        s.update(None, "hello", false);
        s.update(None, " world", false);
        assert_eq!(s.text(), "hello world");
    }
}
