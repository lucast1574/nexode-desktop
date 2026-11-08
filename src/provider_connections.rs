//! Native provider setup. Secrets travel through private IPC, never a chat turn.
use crate::{provider_usage::AccountUsage, theme, widgets};
use eframe::egui::{self, RichText};
use nexode_sdk::NexodeClient;
use serde::Deserialize;
use std::{
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use zeroize::{Zeroize, Zeroizing};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Provider {
    Claude,
    OpenAi,
    DeepSeek,
    Gemini,
    Antigravity,
}
impl Provider {
    const ALL: [Self; 5] = [
        Self::Claude,
        Self::OpenAi,
        Self::Gemini,
        Self::DeepSeek,
        Self::Antigravity,
    ];
    fn name(self) -> &'static str {
        match self {
            Self::Claude => "Claude",
            Self::OpenAi => "GPT / Codex",
            Self::DeepSeek => "DeepSeek",
            Self::Gemini => "Gemini",
            Self::Antigravity => "Antigravity",
        }
    }
    fn oauth_id(self) -> Option<&'static str> {
        match self {
            Self::Claude => Some("claude"),
            Self::OpenAi => Some("openai"),
            Self::DeepSeek => None,
            Self::Gemini => Some("gemini"),
            Self::Antigravity => Some("antigravity"),
        }
    }
    fn key_id(self) -> Option<&'static str> {
        match self {
            Self::Claude => Some("anthropic-api"),
            Self::OpenAi => Some("openai-api"),
            Self::DeepSeek => Some("deepseek"),
            Self::Gemini => Some("gemini-api"),
            Self::Antigravity => None,
        }
    }
    fn multi_account(self) -> bool {
        matches!(self, Self::Claude | Self::OpenAi)
    }
    fn description(self) -> &'static str {
        match self {
            Self::Claude => "Cuenta Claude o API de Anthropic",
            Self::OpenAi => "Cuenta de Codex o API de OpenAI",
            Self::Gemini => "Cuenta de Google o API de Gemini",
            Self::DeepSeek => "API key propia de DeepSeek",
            Self::Antigravity => "Autorización con tu cuenta de Google",
        }
    }
    fn matches_account(self, name: &str) -> bool {
        let name = name.to_ascii_lowercase();
        match self {
            Self::Claude => name.contains("claude") || name.contains("anthropic"),
            Self::OpenAi => {
                name.contains("openai") || name.contains("codex") || name.contains("gpt")
            }
            Self::DeepSeek => name.contains("deepseek"),
            Self::Gemini => {
                !name.contains("antigravity")
                    && (name.contains("gemini") || name.contains("google"))
            }
            Self::Antigravity => name.contains("antigravity"),
        }
    }
}

#[derive(Deserialize)]
struct AuthReply {
    status: String,
    #[serde(default)]
    auth_url: String,
    #[serde(default)]
    input_kind: String,
    #[serde(default)]
    expires_at_ms: i64,
}
struct Pending {
    provider: Provider,
    flow_id: String,
    url: String,
    input_kind: String,
    expires_at_ms: i64,
    binary: PathBuf,
    home: PathBuf,
}
enum Event {
    Pending(Pending),
    Saved,
    Cancelled,
    Failed(String),
}

pub struct ProviderConnections {
    provider: Provider,
    expanded: bool,
    account: String,
    api_key: String,
    callback: String,
    pending: Option<Pending>,
    busy: bool,
    message: Option<String>,
    sender: Sender<Event>,
    receiver: Receiver<Event>,
}
impl Default for ProviderConnections {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            provider: Provider::Claude,
            expanded: false,
            account: String::new(),
            api_key: String::new(),
            callback: String::new(),
            pending: None,
            busy: false,
            message: None,
            sender,
            receiver,
        }
    }
}
impl Drop for ProviderConnections {
    fn drop(&mut self) {
        self.api_key.zeroize();
        self.callback.zeroize();
    }
}
impl ProviderConnections {
    /// Call every app update, including while the account panel is closed.
    /// True means credentials changed: refresh models and subscription usage.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok(event) = self.receiver.try_recv() {
            self.busy = false;
            match event {
                Event::Pending(pending) => {
                    self.pending = Some(pending);
                    self.message = None;
                }
                Event::Saved => {
                    self.pending = None;
                    self.message = Some(
                        "Credencial guardada en Orbit. Actualiza el catálogo para ver sus modelos."
                            .into(),
                    );
                    changed = true;
                }
                Event::Cancelled => {
                    self.pending = None;
                    self.message =
                        Some("Conexión cancelada. Las cuentas existentes se conservan.".into());
                }
                Event::Failed(message) => self.message = Some(message),
            }
        }
        changed
    }

    pub fn is_busy(&self) -> bool {
        self.busy
    }

    /// Renders controls; providers can coexist in the engine's account store.
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        binary: &Path,
        home: &Path,
        client: Option<NexodeClient>,
        accounts: &[AccountUsage],
    ) -> bool {
        let changed = self.poll();
        self.provider_cards(ui, accounts);
        ui.add_space(12.0);
        if !self.expanded && self.pending.is_none() {
            return changed;
        }
        ui.horizontal(|ui| {
            ui.label(RichText::new(self.provider.name()).size(22.0).strong());
            if !self.busy
                && self.pending.is_none()
                && widgets::icon_button(ui, widgets::Icon::Close, "Cerrar opciones del proveedor")
                    .clicked()
            {
                self.expanded = false;
                self.api_key.zeroize();
            }
        });
        for account in accounts
            .iter()
            .filter(|account| self.provider.matches_account(&account.provider_name))
        {
            ui.horizontal_wrapped(|ui| {
                ui.label(account.provider_name.replace('✦', "").trim());
                widgets::muted(
                    ui,
                    if account.error.is_some() {
                        "Uso no disponible; revisa la cuenta si no responde."
                    } else {
                        "Detectada en Orbit"
                    },
                );
            });
        }
        ui.add_enabled_ui(!self.busy && self.pending.is_none(), |ui| {
            widgets::muted(ui, "Cada conexión se guarda de forma independiente. Conectar un proveedor no elimina los demás.");
        });
        if let Some(message) = &self.message {
            ui.label(message);
        }
        if self.busy {
            ui.label("Conectando de forma segura…");
        }
        if let Some(pending) = &self.pending {
            ui.add_space(10.0);
            ui.strong(format!("Autorizar {}", pending.provider.name()));
            if now_ms() >= pending.expires_at_ms {
                ui.label("Esta autorización venció. Cancélala y comienza de nuevo.");
            } else {
                ui.hyperlink_to("Abrir autorización en tu navegador", &pending.url);
                ui.label(match pending.input_kind.as_str() {
                    "auth_code" => "Después de autorizar, pega aquí el código mostrado por el proveedor.",
                    "callback_url" => "Después de autorizar, copia aquí la URL completa de retorno, aunque el navegador muestre que localhost no responde.",
                    _ => "Después de autorizar, pega el código o la URL completa de retorno.",
                });
                ui.add(
                    egui::TextEdit::singleline(&mut self.callback)
                        .password(true)
                        .hint_text("Código o URL de retorno")
                        .desired_width(f32::INFINITY),
                );
                if ui
                    .add_enabled(
                        !self.busy && !self.callback.trim().is_empty(),
                        egui::Button::new("Completar conexión"),
                    )
                    .clicked()
                {
                    self.complete(client.clone(), ui.ctx().clone());
                }
            }
            if ui
                .add_enabled(!self.busy, egui::Button::new("Cancelar autorización"))
                .clicked()
            {
                self.cancel(ui.ctx().clone());
            }
            return changed;
        }
        ui.add_enabled_ui(!self.busy, |ui| {
            if self.provider.oauth_id().is_some() {
                ui.add_space(10.0);
                ui.label(RichText::new("Conectar una cuenta").strong());
                if self.provider.multi_account() {
                    ui.add(egui::TextEdit::singleline(&mut self.account).hint_text("Etiqueta de cuenta (opcional)").desired_width(290.0));
                    ui.small("Deja la etiqueta vacía para añadir otra cuenta. Una etiqueta existente actualiza esa cuenta.");
                } else {
                    ui.small("El motor mantiene una cuenta OAuth activa de este proveedor; una nueva autorización actualiza esa conexión.");
                }
                if ui.button("Conectar con mi cuenta").clicked() { self.begin(binary, home, ui.ctx().clone()); }
            }
            if let Some(id) = self.provider.key_id() {
                ui.add_space(16.0);
                ui.label(RichText::new("Usar una API key propia").strong());
                if self.provider == Provider::DeepSeek {
                    ui.label("DeepSeek se conecta mediante su API key. El uso de la API se factura con DeepSeek; una suscripción de chat no concede automáticamente acceso a la API.");
                } else {
                    ui.small("La API tiene sus propios costes y límites, separados de una suscripción de chat.");
                }
                ui.small("Se guarda en el almacén privado del motor. No se envía a la conversación ni a Nexode.");
                ui.add(egui::TextEdit::singleline(&mut self.api_key).password(true).hint_text("API key").desired_width(f32::INFINITY));
                if ui.add_enabled(client.is_some() && !self.api_key.trim().is_empty(), egui::Button::new("Guardar clave en Orbit")).clicked()
                    && let Some(client) = client.clone() {
                        let key = Zeroizing::new(std::mem::take(&mut self.api_key));
                        let sender = self.sender.clone(); let ctx = ui.ctx().clone(); self.busy = true;
                        std::thread::spawn(move || {
                            let event = if client.set_api_key(id, key.trim()).is_ok() { Event::Saved } else { Event::Failed("Orbit no pudo guardar la clave. Comprueba que el motor esté conectado y sea compatible; la clave no se ha mostrado ni registrado.".into()) };
                            let _ = sender.send(event); ctx.request_repaint();
                        });
                }
                if client.is_none() { ui.small("Conecta el motor para guardar una API key."); }
            }
        });
        changed
    }

    fn provider_cards(&mut self, ui: &mut egui::Ui, accounts: &[AccountUsage]) {
        let columns = if ui.available_width() >= 600.0 { 2 } else { 1 };
        let width = (ui.available_width() - 12.0 * (columns - 1) as f32) / columns as f32;
        for row in Provider::ALL.chunks(columns) {
            ui.horizontal(|ui| {
                for &provider in row {
                    ui.push_id(provider.name(), |ui| {
                        egui::Frame::new()
                            .fill(theme::SURFACE)
                            .corner_radius(12)
                            .inner_margin(18.0)
                            .show(ui, |ui| {
                                ui.vertical(|ui| {
                                    let content_width = (width - 36.0).max(0.0);
                                    ui.set_min_width(content_width);
                                    ui.set_max_width(content_width);
                                    ui.set_min_height(104.0);
                                    ui.label(RichText::new(provider.name()).size(18.0).strong());
                                    widgets::muted(ui, provider.description());
                                    let count = accounts
                                        .iter()
                                        .filter(|account| {
                                            provider.matches_account(&account.provider_name)
                                        })
                                        .count();
                                    ui.add_space(4.0);
                                    ui.horizontal_wrapped(|ui| {
                                        let selected = self.expanded && self.provider == provider;
                                        let label = if selected {
                                            "Opciones abiertas"
                                        } else if count > 0 {
                                            "Gestionar conexión"
                                        } else {
                                            "Conectar"
                                        };
                                        if ui
                                            .add_enabled(
                                                !self.busy && self.pending.is_none(),
                                                egui::Button::new(label)
                                                    .selected(selected)
                                                    .min_size(egui::vec2(140.0, 32.0)),
                                            )
                                            .clicked()
                                        {
                                            if self.provider != provider {
                                                self.api_key.zeroize();
                                                self.account.clear();
                                                self.callback.zeroize();
                                                self.message = None;
                                            }
                                            self.provider = provider;
                                            self.expanded = true;
                                        }
                                        if count > 0 {
                                            ui.label(
                                                RichText::new(format!(
                                                    "{count} detectada{}",
                                                    if count == 1 { "" } else { "s" }
                                                ))
                                                .size(11.0)
                                                .color(theme::ACCENT),
                                            );
                                        }
                                    });
                                });
                            });
                    });
                }
            });
            ui.add_space(2.0);
        }
        widgets::muted(
            ui,
            "Las cuentas detectadas se actualizan junto con el uso de IA. Los permisos y cuotas se comprueban con cada proveedor.",
        );
    }

    fn begin(&mut self, binary: &Path, home: &Path, ctx: egui::Context) {
        let Some(id) = self.provider.oauth_id() else {
            return;
        };
        if self.account.len() > 64 || self.account.chars().any(char::is_control) {
            self.message =
                Some("Usa una etiqueta de hasta 64 caracteres, sin caracteres de control.".into());
            return;
        }
        static FLOW: AtomicU64 = AtomicU64::new(1);
        let flow_id = format!(
            "desktop-{}-{}-{}",
            std::process::id(),
            now_ms(),
            FLOW.fetch_add(1, Ordering::Relaxed)
        );
        let mut args = vec![
            "login".into(),
            id.into(),
            "--print-auth-url".into(),
            "--json".into(),
            "--no-validate".into(),
            "--flow-id".into(),
            flow_id.clone(),
        ];
        if self.provider.multi_account() && !self.account.trim().is_empty() {
            args.extend(["--account".into(), self.account.trim().into()]);
        }
        let binary = binary.to_owned();
        let home = home.to_owned();
        let provider = self.provider;
        let sender = self.sender.clone();
        self.busy = true;
        self.message = None;
        std::thread::spawn(move || {
            let result = run_login(&binary, &home, &args).and_then(|reply| {
                if reply.status != "pending" || !allowed_auth_url(&reply.auth_url) || !matches!(reply.input_kind.as_str(), "auth_code" | "callback_url" | "auth_code_or_callback_url") || reply.expires_at_ms <= now_ms() { return Err("El motor devolvió una autorización no válida. Actualiza Orbit y vuelve a intentar.".into()); }
                Ok(Pending { provider, flow_id, url: reply.auth_url, input_kind: reply.input_kind, expires_at_ms: reply.expires_at_ms, binary, home })
            });
            let _ = sender.send(match result {
                Ok(pending) => Event::Pending(pending),
                Err(error) => Event::Failed(error),
            });
            ctx.request_repaint();
        });
    }

    fn complete(&mut self, client: Option<NexodeClient>, ctx: egui::Context) {
        let Some(pending) = &self.pending else {
            return;
        };
        let input = Zeroizing::new(std::mem::take(&mut self.callback));
        let flag = if pending.input_kind == "callback_url"
            || input.trim().starts_with("http://")
            || input.trim().starts_with("https://")
        {
            "--callback-url"
        } else {
            "--auth-code"
        };
        let provider = pending.provider.oauth_id().unwrap();
        let args = Zeroizing::new(vec![
            "login".into(),
            provider.into(),
            "--json".into(),
            "--no-validate".into(),
            "--flow-id".into(),
            pending.flow_id.clone(),
            flag.into(),
            input.trim().into(),
        ]);
        let binary = pending.binary.clone();
        let home = pending.home.clone();
        let sender = self.sender.clone();
        self.busy = true;
        std::thread::spawn(move || {
            let result = run_login(&binary, &home, &args).and_then(|reply| {
                if reply.status != "authenticated" { return Err("La autorización sigue pendiente. Completa el paso del proveedor.".into()); }
                if let Some(client) = client { client.notify_auth_changed(provider).map_err(|_| "Cuenta guardada, pero el motor no pudo recargarla. Reconecta el motor desde Configuración.".to_string())?; }
                Ok(())
            });
            let _ = sender.send(match result {
                Ok(()) => Event::Saved,
                Err(error) => Event::Failed(error),
            });
            ctx.request_repaint();
        });
    }

    fn cancel(&mut self, ctx: egui::Context) {
        let Some(pending) = &self.pending else {
            return;
        };
        self.callback.zeroize();
        let args = vec![
            "login".into(),
            pending.provider.oauth_id().unwrap().into(),
            "--json".into(),
            "--flow-id".into(),
            pending.flow_id.clone(),
            "--cancel".into(),
        ];
        let binary = pending.binary.clone();
        let home = pending.home.clone();
        let sender = self.sender.clone();
        self.busy = true;
        std::thread::spawn(move || {
            let result = run_login(&binary, &home, &args);
            let _ = sender.send(match result {
                Ok(_) => Event::Cancelled,
                Err(error) => Event::Failed(error),
            });
            ctx.request_repaint();
        });
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}
fn allowed_auth_url(value: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(value) else {
        return false;
    };
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port_or_known_default() == Some(443)
        && matches!(
            url.host_str(),
            Some(
                "claude.ai"
                    | "auth.claude.ai"
                    | "console.anthropic.com"
                    | "auth.openai.com"
                    | "accounts.google.com"
            )
        )
}
fn parse_reply(bytes: &[u8]) -> Result<AuthReply, String> {
    serde_json::from_slice(bytes).map_err(|_| {
        "El motor devolvió una respuesta de autenticación incompatible. Actualiza Orbit.".into()
    })
}
fn run_login(binary: &Path, home: &Path, args: &[String]) -> Result<AuthReply, String> {
    let mut command = Command::new(binary);
    command
        .args(args)
        .env("NEXODE_HOME", home)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command
        .spawn()
        .map_err(|_| "No se pudo iniciar Orbit para conectar la cuenta.".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or("No se pudo leer la respuesta de Orbit.")?;
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stdout.take(256 * 1024).read_to_end(&mut bytes);
        bytes
    });
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let bytes = Zeroizing::new(reader.join().unwrap_or_default());
                if !status.success() {
                    return Err("El proveedor no pudo completar la conexión. Verifica el código, la URL y su vigencia; vuelve a autorizar si es necesario.".into());
                }
                return parse_reply(&bytes);
            }
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                return Err("La conexión tardó demasiado. Vuelve a intentarlo.".into());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn auth_urls_reject_credential_exfiltration_targets() {
        assert!(allowed_auth_url(
            "https://accounts.google.com/o/oauth2/auth?state=abc"
        ));
        for url in [
            "http://auth.openai.com/",
            "https://auth.openai.com.evil.test/",
            "https://auth.openai.com@evil.test/",
            "https://evil.test/",
            "javascript:alert(1)",
            "https://auth.openai.com:8443/",
        ] {
            assert!(!allowed_auth_url(url), "{url}");
        }
    }
    #[test]
    fn parses_only_contract_fields_and_never_echoes_raw_errors() {
        let pending = parse_reply(br#"{"status":"pending","provider":"claude","auth_url":"https://claude.ai/oauth/authorize","input_kind":"auth_code_or_callback_url","expires_at_ms":9999999999999,"pending_path":"private"}"#).unwrap();
        assert_eq!(pending.status, "pending");
        assert_eq!(pending.input_kind, "auth_code_or_callback_url");
        assert!(
            !parse_reply(b"error with sk-secret-token")
                .err()
                .unwrap()
                .contains("sk-secret-token")
        );
    }
    #[test]
    fn method_availability_matches_provider_contracts() {
        assert_eq!(Provider::DeepSeek.oauth_id(), None);
        assert_eq!(Provider::Antigravity.key_id(), None);
        assert_eq!(Provider::Gemini.key_id(), Some("gemini-api"));
        assert!(Provider::Claude.multi_account() && Provider::OpenAi.multi_account());
        assert!(!Provider::Gemini.multi_account());
    }
}
