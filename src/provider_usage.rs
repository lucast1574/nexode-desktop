//! Read-only subscription meters from Orbit's native `usage --json` contract.
//! No inference calls, synthetic percentages, credential reads or persistent cache.
use eframe::egui::{self, Color32, RichText, Stroke};
use serde::Deserialize;
use std::{
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, channel},
    },
    time::{Duration, Instant},
};

const REFRESH_INTERVAL: Duration = Duration::from_secs(120);
const COMMAND_TIMEOUT: Duration = Duration::from_secs(45);
const MAX_REPORT_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize)]
pub struct Limit {
    pub name: String,
    pub usage_percent: Option<f32>,
    pub resets_at: Option<String>,
}

impl Limit {
    /// Upstream usage is consumed percentage, not remaining percentage.
    pub fn remaining(&self) -> Option<f32> {
        self.usage_percent
            .filter(|value| value.is_finite() && *value >= 0.0)
            .map(|value| (100.0 - value).clamp(0.0, 100.0))
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct AccountUsage {
    pub provider_name: String,
    #[serde(default)]
    pub limits: Vec<Limit>,
    #[serde(default)]
    pub extra_info: Vec<(String, String)>,
    pub error: Option<String>,
    #[serde(skip)]
    stale: bool,
    #[serde(skip)]
    measured: Option<Instant>,
}

impl AccountUsage {
    pub fn remaining(&self) -> Option<f32> {
        self.limits
            .iter()
            .filter_map(Limit::remaining)
            .reduce(f32::min)
    }

    fn title(&self) -> String {
        // The CLI decorates active accounts with a star; native UI uses no glyph icons.
        self.provider_name.replace('✦', "").trim().to_owned()
    }

    fn short_name(&self) -> &'static str {
        let name = self.provider_name.to_ascii_lowercase();
        if name.contains("anthropic") || name.contains("claude") {
            "Claude"
        } else if name.contains("openai") || name.contains("codex") {
            "GPT"
        } else if name.contains("antigravity") {
            "AntiG"
        } else if name.contains("gemini") || name.contains("google") {
            "Gemini"
        } else if name.contains("deepseek") {
            "DeepSeek"
        } else {
            "IA"
        }
    }

    fn tooltip(&self, ui: &mut egui::Ui) {
        ui.set_max_width(340.0);
        ui.strong(self.title());
        for (key, value) in &self.extra_info {
            // Whitelist public account metadata. Never render raw network errors.
            match key.as_str() {
                "Account label" => {
                    ui.label(format!("Cuenta: {value}"));
                }
                "Plan" => {
                    ui.label(format!("Plan: {value}"));
                }
                _ => {}
            }
        }
        if self.stale {
            ui.colored_label(
                Color32::from_rgb(218, 181, 112),
                "Última lectura; no se pudo actualizar.",
            );
        } else if self.error.is_some() && !self.limits.is_empty() {
            ui.colored_label(
                Color32::from_rgb(218, 181, 112),
                "El proveedor no pudo confirmar esta cuota.",
            );
        }
        if self.limits.is_empty() {
            ui.label(if self.error.is_some() {
                "No se pudo consultar la cuota de esta cuenta."
            } else {
                "El proveedor no publica una cuota porcentual de suscripción."
            });
        }
        for limit in &self.limits {
            match limit.remaining() {
                Some(remaining) => {
                    ui.label(format!("{}: {:.0}% disponible", limit.name, remaining));
                }
                None => {
                    ui.label(format!("{}: cuota desconocida", limit.name));
                }
            }
            if let Some(reset) = &limit.resets_at {
                ui.small(format!("Reinicio: {reset}"));
            }
        }
        if self.limits.len() > 1 {
            ui.small("El círculo muestra la ventana con menos cuota restante.");
        }
        if let Some(at) = self.measured {
            ui.small(format!("Consultado hace {} s", at.elapsed().as_secs()));
        }
        ui.small("Cuota de la cuenta; no es el contexto del chat ni una estimación de tokens.");
    }
}

#[derive(Deserialize)]
struct UsageReport {
    providers: Vec<AccountUsage>,
}

fn decode_report(bytes: &[u8]) -> Result<Vec<AccountUsage>, String> {
    let mut report: UsageReport = serde_json::from_slice(bytes)
        .map_err(|_| "El motor devolvió un informe de cuota incompatible.".to_string())?;
    let now = Instant::now();
    for account in &mut report.providers {
        account.measured = Some(now);
    }
    Ok(report.providers)
}

/// Owns at most one read-only process. All network/IPC work stays off the UI thread.
pub struct UsageMonitor {
    pub accounts: Vec<AccountUsage>,
    pub error: Option<String>,
    pending: Option<Receiver<Result<Vec<AccountUsage>, String>>>,
    cancelled: Arc<AtomicBool>,
    next_refresh: Instant,
    source: Option<(PathBuf, PathBuf)>,
}

impl Default for UsageMonitor {
    fn default() -> Self {
        Self {
            accounts: vec![],
            error: None,
            pending: None,
            cancelled: Arc::new(AtomicBool::new(false)),
            next_refresh: Instant::now(),
            source: None,
        }
    }
}

impl UsageMonitor {
    /// Call after connection and each frame. `home` must be the engine's private
    /// state directory so account selection and newly added logins agree with chat.
    pub fn update(&mut self, ctx: &egui::Context, binary: &Path, home: &Path) {
        let source = (binary.to_owned(), home.to_owned());
        if self.source.as_ref() != Some(&source) {
            self.cancelled.store(true, Ordering::Release);
            self.cancelled = Arc::new(AtomicBool::new(false));
            self.pending = None;
            self.accounts.clear();
            self.error = None;
            self.next_refresh = Instant::now();
            self.source = Some(source.clone());
        }
        if let Some(receiver) = &self.pending {
            match receiver.try_recv() {
                Ok(result) => {
                    self.pending = None;
                    self.next_refresh = Instant::now() + REFRESH_INTERVAL;
                    match result {
                        Ok(accounts) => {
                            self.accounts = merge_readings(&self.accounts, accounts);
                            self.error = None;
                        }
                        Err(error) => {
                            self.error = Some(error);
                            for account in &mut self.accounts {
                                account.stale = true;
                            }
                        }
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.pending = None;
                    self.error = Some("La consulta de cuota se interrumpió.".into());
                    for account in &mut self.accounts {
                        account.stale = true;
                    }
                    self.next_refresh = Instant::now() + REFRESH_INTERVAL;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
        if self.pending.is_none() && Instant::now() >= self.next_refresh {
            let (sender, receiver) = channel();
            self.pending = Some(receiver);
            let cancelled = Arc::clone(&self.cancelled);
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let result = query(&source.0, &source.1, &cancelled);
                if !cancelled.load(Ordering::Acquire) {
                    let _ = sender.send(result);
                    ctx.request_repaint();
                }
            });
        }
        if self.pending.is_none() {
            ctx.request_repaint_after(self.next_refresh.saturating_duration_since(Instant::now()));
        }
    }

    /// Request a fresh reading after a completed turn or provider login. No overlapping jobs.
    pub fn refresh(&mut self) {
        self.next_refresh = Instant::now();
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("CUOTA DE IA")
                    .small()
                    .color(crate::theme::MUTED),
            );
            if ui.small_button("Actualizar").clicked() {
                self.refresh();
            }
        });
        if self.accounts.is_empty() {
            ui.small(if self.pending.is_some() {
                "Consultando tus cuentas…"
            } else if self.error.is_some() {
                "Cuota no disponible"
            } else {
                "Conecta una cuenta de IA"
            });
        }
        ui.horizontal_wrapped(|ui| {
            for account in &self.accounts {
                ui.vertical(|ui| {
                    let (rect, response) =
                        ui.allocate_exact_size(egui::vec2(36.0, 36.0), egui::Sense::hover());
                    let center = rect.center();
                    let radius = 14.0;
                    let painter = ui.painter();
                    let remaining = account.remaining();
                    let color = if account.stale || account.error.is_some() {
                        crate::theme::MUTED
                    } else if remaining.is_some_and(|v| v <= 10.0) {
                        Color32::from_rgb(218, 153, 137)
                    } else {
                        crate::theme::ACCENT
                    };
                    painter.circle_stroke(center, radius, Stroke::new(2.5_f32, crate::theme::LINE));
                    if let Some(remaining) = remaining {
                        let sweep = std::f32::consts::TAU * remaining / 100.0;
                        if sweep > 0.0 {
                            let steps = ((64.0 * remaining / 100.0).ceil() as usize).max(2);
                            let points = (0..=steps)
                                .map(|step| {
                                    let angle = -std::f32::consts::FRAC_PI_2
                                        + sweep * step as f32 / steps as f32;
                                    center + egui::vec2(angle.cos(), angle.sin()) * radius
                                })
                                .collect();
                            painter.add(egui::Shape::line(points, Stroke::new(2.5_f32, color)));
                        }
                        // Native vector mark, not a Unicode/emoji icon.
                        let diamond = vec![
                            center + egui::vec2(0.0, -4.0),
                            center + egui::vec2(4.0, 0.0),
                            center + egui::vec2(0.0, 4.0),
                            center + egui::vec2(-4.0, 0.0),
                        ];
                        painter.add(egui::Shape::convex_polygon(diamond, color, Stroke::NONE));
                    } else {
                        painter.text(
                            center,
                            egui::Align2::CENTER_CENTER,
                            "?",
                            egui::FontId::proportional(14.0),
                            crate::theme::MUTED,
                        );
                    }
                    response.on_hover_ui(|ui| account.tooltip(ui));
                    ui.label(
                        RichText::new(account.short_name())
                            .size(10.0)
                            .color(crate::theme::MUTED),
                    );
                });
            }
        });
        if let Some(error) = &self.error {
            ui.small(error);
        }
    }
}

impl Drop for UsageMonitor {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
    }
}

fn merge_readings(previous: &[AccountUsage], fresh: Vec<AccountUsage>) -> Vec<AccountUsage> {
    // A successful inventory drops disconnected accounts. Only a failed account
    // retains its own previous reading, explicitly marked stale.
    fresh
        .into_iter()
        .map(|account| {
            if account.error.is_some()
                && let Some(old) = previous
                    .iter()
                    .find(|old| old.provider_name == account.provider_name)
            {
                let mut old = old.clone();
                old.stale = true;
                old.error = account.error;
                return old;
            }
            account
        })
        .collect()
}

fn query(binary: &Path, home: &Path, cancelled: &AtomicBool) -> Result<Vec<AccountUsage>, String> {
    let mut accounts = decode_report(&run_report(binary, home, cancelled, &["usage", "--json"])?)?;
    // Some connected providers offer no usage endpoint at all. Keep them visible
    // as unknown; a lack of a quota report must not look like a disconnected account.
    if let Ok(bytes) = run_report(binary, home, cancelled, &["auth", "status", "--json"]) {
        add_unreported_providers(&mut accounts, &bytes);
    }
    Ok(accounts)
}

fn add_unreported_providers(accounts: &mut Vec<AccountUsage>, bytes: &[u8]) {
    #[derive(Deserialize)]
    struct AuthProvider {
        id: String,
        display_name: String,
        status: String,
    }
    #[derive(Deserialize)]
    struct AuthReport {
        providers: Vec<AuthProvider>,
    }
    let Ok(report) = serde_json::from_slice::<AuthReport>(bytes) else {
        return;
    };
    for provider in report
        .providers
        .into_iter()
        .filter(|p| p.status == "available")
    {
        let family = match provider.id.as_str() {
            "openai" | "openai-api" => "openai",
            "claude" | "anthropic" => "anthropic",
            "google" | "gemini" => "gemini",
            other => other,
        };
        if accounts.iter().any(|a| {
            a.provider_name.to_ascii_lowercase().contains(family)
                || a.provider_name.eq_ignore_ascii_case(&provider.display_name)
        }) {
            continue;
        }
        accounts.push(AccountUsage {
            provider_name: provider.display_name,
            limits: vec![],
            extra_info: vec![],
            error: None,
            stale: false,
            measured: Some(Instant::now()),
        });
    }
}

fn run_report(
    binary: &Path,
    home: &Path,
    cancelled: &AtomicBool,
    args: &[&str],
) -> Result<Vec<u8>, String> {
    let mut command = Command::new(binary);
    command
        .args(args)
        .env("NEXODE_HOME", home)
        .env("NEXODE_NO_AUTO_UPDATE", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = command
        .spawn()
        .map_err(|_| "No se pudo consultar la cuota con el motor de Orbit.".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or("No se pudo leer la cuota del motor.")?;
    // Drain concurrently to avoid pipe backpressure. Truncation is an error, never partial JSON.
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout
            .take(MAX_REPORT_BYTES + 1)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });
    let started = Instant::now();
    loop {
        if cancelled.load(Ordering::Acquire) || started.elapsed() >= COMMAND_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            return Err("La consulta de cuota no terminó a tiempo.".into());
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return Err("No se pudo actualizar la cuota del proveedor.".into());
                }
                break;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("No se pudo consultar el estado de cuota.".into());
            }
        }
    }
    let bytes = reader
        .join()
        .map_err(|_| "No se pudo leer el informe de cuota.")?
        .map_err(|_| "No se pudo leer el informe de cuota.")?;
    if bytes.len() as u64 > MAX_REPORT_BYTES {
        return Err("El informe de cuota excede el tamaño permitido.".into());
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    const REPORT: &[u8] = br#"{"providers":[{"provider_name":"OpenAI - work","limits":[{"name":"5 hours","usage_percent":30,"resets_at":"2026-11-08T01:00:00Z"},{"name":"Weekly","usage_percent":80,"resets_at":null}],"extra_info":[["Account label","work"]],"error":null},{"provider_name":"OpenAI - personal","limits":[{"name":"5 hours","usage_percent":0,"resets_at":null}],"error":null},{"provider_name":"DeepSeek","limits":[],"error":null}]}"#;

    #[test]
    fn keeps_each_account_and_uses_most_restrictive_remaining_window() {
        let accounts = decode_report(REPORT).unwrap();
        assert_eq!(accounts.len(), 3);
        assert_eq!(accounts[0].remaining(), Some(20.0));
        assert_eq!(accounts[1].remaining(), Some(100.0));
        assert_eq!(accounts[2].remaining(), None);
    }

    #[test]
    fn does_not_invent_quota_from_invalid_or_missing_values() {
        let mut limit = Limit {
            name: "Weekly".into(),
            usage_percent: None,
            resets_at: None,
        };
        assert_eq!(limit.remaining(), None);
        limit.usage_percent = Some(-1.0);
        assert_eq!(limit.remaining(), None);
        limit.usage_percent = Some(f32::NAN);
        assert_eq!(limit.remaining(), None);
        limit.usage_percent = Some(105.0);
        assert_eq!(limit.remaining(), Some(0.0));
    }

    #[test]
    fn failed_account_retains_stale_reading_but_removed_accounts_disappear() {
        let previous = decode_report(REPORT).unwrap();
        let mut failed = previous[0].clone();
        failed.limits.clear();
        failed.error = Some("offline".into());
        let merged = merge_readings(&previous, vec![failed]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].remaining(), Some(20.0));
        assert!(merged[0].stale);
        assert_eq!(merged[0].measured, previous[0].measured);
    }

    #[test]
    fn shows_connected_provider_without_quota_but_does_not_duplicate_account_reports() {
        let mut accounts = decode_report(REPORT).unwrap();
        add_unreported_providers(&mut accounts, br#"{"providers":[{"id":"openai","display_name":"OpenAI","status":"available"},{"id":"google","display_name":"Gemini OAuth","status":"available"},{"id":"cursor","display_name":"Cursor","status":"not_configured"}]}"#);
        assert_eq!(accounts.len(), 4);
        assert_eq!(accounts[3].provider_name, "Gemini OAuth");
        assert_eq!(accounts[3].remaining(), None);
    }
}
