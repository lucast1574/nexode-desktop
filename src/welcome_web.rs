//! Local welcome artwork only. The engine and all privileged actions remain in Rust.
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};

use eframe::egui;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WelcomeAction {
    AddProject,
    ConnectAgents,
    Services,
    NewChat,
    FocusComposer,
    BackToChats,
    ServicePrepare { index: u8 },
    ServiceExplore { index: u8 },
    Suggestion { index: u8 },
}

fn parse_action(body: &str) -> Option<WelcomeAction> {
    if body.len() > 128 {
        return None;
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Message {
        action: String,
        index: Option<u8>,
    }
    let message: Message = serde_json::from_str(body).ok()?;
    let fields: serde_json::Map<String, serde_json::Value> = serde_json::from_str(body).ok()?;
    if !matches!(
        message.action.as_str(),
        "suggestion" | "service_prepare" | "service_explore"
    ) && fields.len() != 1
    {
        return None;
    }
    match (message.action.as_str(), message.index) {
        ("add_project", None) => Some(WelcomeAction::AddProject),
        ("connect_agents", None) => Some(WelcomeAction::ConnectAgents),
        ("services", None) => Some(WelcomeAction::Services),
        ("new_chat", None) => Some(WelcomeAction::NewChat),
        ("focus_composer", None) => Some(WelcomeAction::FocusComposer),
        ("back_to_chats", None) => Some(WelcomeAction::BackToChats),
        ("service_prepare", Some(index))
            if usize::from(index) < crate::ecosystem::SERVICES.len() =>
        {
            Some(WelcomeAction::ServicePrepare { index })
        }
        ("service_explore", Some(index))
            if usize::from(index) < crate::ecosystem::SERVICES.len() =>
        {
            Some(WelcomeAction::ServiceExplore { index })
        }
        ("suggestion", Some(index @ 0..=3)) => Some(WelcomeAction::Suggestion { index }),
        _ => None,
    }
}

#[derive(Default)]
pub struct WelcomeWeb {
    view: Option<wry::WebView>,
    actions: Option<mpsc::Receiver<WelcomeAction>>,
    bounds: Option<wry::Rect>,
    failed: bool,
    state: Option<String>,
    ready: Arc<AtomicBool>,
    loading_since: Option<std::time::Instant>,
    document_key: String,
}

impl WelcomeWeb {
    pub fn new() -> Self {
        Self::default()
    }

    /// `None` destroys the welcome view before a native modal or conversation is drawn.
    /// Returns false when the caller should render its native fallback.
    pub fn sync(
        &mut self,
        frame: &eframe::Frame,
        ctx: &egui::Context,
        rect: Option<egui::Rect>,
        html: &str,
    ) -> bool {
        self.sync_document(frame, ctx, rect, html, "welcome")
    }

    pub fn sync_document(
        &mut self,
        frame: &eframe::Frame,
        ctx: &egui::Context,
        rect: Option<egui::Rect>,
        html: &str,
        document_key: &str,
    ) -> bool {
        if self.document_key != document_key {
            self.view = None;
            self.actions = None;
            self.bounds = None;
            self.state = None;
            self.ready = Arc::new(AtomicBool::new(false));
            self.loading_since = None;
            self.document_key = document_key.to_owned();
        }
        let Some(rect) = rect.filter(|r| r.is_positive() && r.is_finite()) else {
            self.view = None;
            self.actions = None;
            self.bounds = None;
            self.state = None;
            self.ready = Arc::new(AtomicBool::new(false));
            self.loading_since = None;
            return false;
        };
        if self.failed {
            return false;
        }
        let scale = ctx.pixels_per_point();
        let bounds = wry::Rect {
            position: wry::dpi::PhysicalPosition::new(
                (rect.left() * scale).round() as i32,
                (rect.top() * scale).round() as i32,
            )
            .into(),
            size: wry::dpi::PhysicalSize::new(
                (rect.width() * scale).round().max(1.0) as u32,
                (rect.height() * scale).round().max(1.0) as u32,
            )
            .into(),
        };
        if self.view.is_none() {
            let Ok(handle) = frame.window_handle() else {
                return false;
            };
            if !matches!(
                handle.as_raw(),
                RawWindowHandle::Win32(_)
                    | RawWindowHandle::AppKit(_)
                    | RawWindowHandle::Xlib(_)
                    | RawWindowHandle::Xcb(_)
            ) {
                return false;
            }
            #[cfg(target_os = "linux")]
            if !gtk::is_initialized_main_thread() {
                return false;
            }
            #[cfg(target_os = "linux")]
            {
                use gtk::prelude::ObjectExt;
                if gtk::gdk::Display::default()
                    .is_none_or(|display| display.type_().name() != "GdkX11Display")
                {
                    return false;
                }
            }
            let (send, receive) = mpsc::sync_channel(16);
            let repaint = ctx.clone();
            let loaded_repaint = ctx.clone();
            let ready = Arc::clone(&self.ready);
            // This policy is inserted before all supplied markup; no remote resources,
            // frames, forms, plugins or network requests are available to welcome code.
            let policy = "<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data:; font-src data:; connect-src 'none'; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'\">";
            // Keep the doctype first so the embedded page stays in standards mode.
            let html = if html.contains("<head>") {
                html.replacen("<head>", &format!("<head>{policy}"), 1)
            } else {
                format!("<!doctype html><head>{policy}</head><body>{html}</body>")
            };
            let result = wry::WebViewBuilder::new()
                .with_html(html)
                .with_bounds(bounds)
                .with_background_color((15, 15, 15, 255))
                .with_incognito(true)
                .with_devtools(false)
                .with_clipboard(false)
                .with_autoplay(false)
                .with_focused(false)
                .with_navigation_handler(|url| url == "about:blank")
                .with_new_window_req_handler(|_, _| wry::NewWindowResponse::Deny)
                .with_download_started_handler(|_, _| false)
                .with_permission_handler(|_| wry::PermissionResponse::Deny)
                .with_on_page_load_handler(move |event, _| {
                    ready.store(
                        matches!(event, wry::PageLoadEvent::Finished),
                        Ordering::Release,
                    );
                    loaded_repaint.request_repaint();
                })
                .with_ipc_handler(move |request| {
                    if let Some(action) = parse_action(request.body()) {
                        let _ = send.try_send(action);
                        repaint.request_repaint();
                    }
                })
                .build_as_child(frame);
            match result {
                Ok(view) => {
                    self.view = Some(view);
                    self.actions = Some(receive);
                    self.bounds = Some(bounds);
                    self.loading_since = Some(std::time::Instant::now());
                }
                Err(_) => {
                    self.failed = true;
                    return false;
                }
            }
        } else if self.bounds != Some(bounds) {
            if self
                .view
                .as_ref()
                .is_some_and(|view| view.set_bounds(bounds).is_err())
            {
                self.view = None;
                self.failed = true;
                return false;
            }
            self.bounds = Some(bounds);
        }
        if self.ready.load(Ordering::Acquire) {
            self.loading_since = None;
        } else if self
            .loading_since
            .is_some_and(|started| started.elapsed() >= std::time::Duration::from_secs(10))
        {
            // A missing/crashed OS helper can leave construction successful but
            // never paint. Restore the native welcome without weakening its sandbox.
            self.view = None;
            self.actions = None;
            self.bounds = None;
            self.state = None;
            self.loading_since = None;
            self.failed = true;
            return false;
        } else {
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
        }
        #[cfg(target_os = "linux")]
        {
            crate::desktop_shell::pump_gtk();
            ctx.request_repaint_after(std::time::Duration::from_millis(33));
        }
        true
    }

    /// Public display data only; never pass account credentials or conversation text.
    pub fn set_state(&mut self, first_project: bool, project_name: &str, reduce_motion: bool) {
        if !self.ready.load(Ordering::Acquire) {
            return;
        }
        let state = serde_json::json!({
            "firstProject": first_project,
            "projectName": project_name,
            "reduceMotion": reduce_motion,
        })
        .to_string();
        if self.state.as_ref() == Some(&state) {
            return;
        }
        if let Some(view) = &self.view {
            // Keep the latest state if page parsing has not reached its local script yet.
            let script = format!(
                "window.__orbitState={state}; if(window.orbitSetState) window.orbitSetState(window.__orbitState);"
            );
            if view.evaluate_script(&script).is_ok() {
                self.state = Some(state);
            }
        }
    }

    pub fn drain_actions(&self) -> Vec<WelcomeAction> {
        self.actions
            .as_ref()
            .map(|rx| rx.try_iter().take(16).collect())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipc_rejects_unknown_commands_fields_and_out_of_range_suggestions() {
        assert_eq!(
            parse_action(r#"{"action":"back_to_chats"}"#),
            Some(WelcomeAction::BackToChats)
        );
        assert_eq!(
            parse_action(r#"{"action":"service_prepare","index":0}"#),
            Some(WelcomeAction::ServicePrepare { index: 0 })
        );
        assert_eq!(
            parse_action(r#"{"action":"service_explore","index":13}"#),
            Some(WelcomeAction::ServiceExplore { index: 13 })
        );
        for invalid in [
            r#"{"action":"service_explore","index":255}"#,
            r#"{"action":"service_prepare","index":14}"#,
            r#"{"action":"back_to_chats","index":null}"#,
            r#"{"action":"service_explore","url":"https://example.com","index":0}"#,
        ] {
            assert_eq!(parse_action(invalid), None);
        }
        assert_eq!(
            parse_action(r#"{"action":"add_project"}"#),
            Some(WelcomeAction::AddProject)
        );
        assert_eq!(
            parse_action(r#"{"action":"suggestion","index":3}"#),
            Some(WelcomeAction::Suggestion { index: 3 })
        );
        assert_eq!(
            parse_action(r#"{"action":"new_chat"}"#),
            Some(WelcomeAction::NewChat)
        );
        assert_eq!(
            parse_action(r#"{"action":"focus_composer"}"#),
            Some(WelcomeAction::FocusComposer)
        );
        for invalid in [
            r#"{"action":"shell","command":"whoami"}"#,
            r#"{"action":"add_project","path":"/etc"}"#,
            r#"{"action":"suggestion","index":4}"#,
            r#"{"action":"suggestion","index":-1}"#,
            r#"{"action":"suggestion"}"#,
            r#"{"action":"new_chat","index":0}"#,
            r#"{"action":"new_chat","index":null}"#,
            r#"{"action":"focus_composer","command":"run"}"#,
        ] {
            assert_eq!(parse_action(invalid), None);
        }
        assert!(parse_action(&" ".repeat(129)).is_none());
    }
}
