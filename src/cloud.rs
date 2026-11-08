//! Nexode account API. Desktop passwords are transient; only the user's own
//! connector is kept in the OS credential vault. No administrator credentials.
use eframe::egui;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::{
    sync::mpsc::{self, Receiver},
    time::Duration,
};
use zeroize::{Zeroize, Zeroizing};

const GRAPHQL: &str = "https://backend.nexode.app/api-v1/graphql";
const SDK: &str = "https://backend.nexode.app/api-v1/sdk/v1";
const VAULT: &str = "app.nexode.orbit.desktop";

fn entry() -> Result<keyring::Entry, String> {
    keyring::Entry::new(VAULT, "nexode-connector")
        .map_err(|_| "No se pudo abrir el almacén seguro del sistema.".into())
}
pub fn credential() -> Result<Option<Zeroizing<String>>, String> {
    match entry()?.get_password() {
        Ok(key) => Ok(Some(Zeroizing::new(key))),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(_) => Err("Desbloquea el almacén seguro del sistema para conectar Nexode.".into()),
    }
}
fn client(cookies: bool) -> Result<Client, String> {
    Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .cookie_store(cookies)
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|_| "No se pudo iniciar la conexión segura.".into())
}
fn response(response: reqwest::blocking::Response) -> Result<Value, String> {
    if !response.status().is_success() {
        return Err(format!(
            "Nexode respondió HTTP {}. Revisa tu sesión, permisos o conexión.",
            response.status().as_u16()
        ));
    }
    response
        .json()
        .map_err(|_| "Nexode devolvió una respuesta no válida.".into())
}
fn gql(c: &Client, query: &str, variables: Value) -> Result<Value, String> {
    let data = response(
        c.post(GRAPHQL)
            .json(&json!({"query":query,"variables":variables}))
            .send()
            .map_err(|_| "No se pudo contactar con Nexode.".to_string())?,
    )?;
    if data.get("errors").is_some() {
        // Never display arbitrary server error bodies: they may echo secrets.
        return Err("La operación no pudo completarse. Revisa los datos y la verificación de tu correo; espera antes de volver a intentarlo.".into());
    }
    data.get("data")
        .cloned()
        .ok_or("Respuesta GraphQL incompleta.".into())
}
fn sdk(key: &str, route: &str, body: Option<Value>) -> Result<Value, String> {
    let c = client(false)?;
    let request = match body {
        Some(body) => c.post(format!("{SDK}/{route}")).json(&body),
        None => c.get(format!("{SDK}/{route}")),
    };
    response(
        request
            .bearer_auth(key)
            .send()
            .map_err(|_| "No se pudo contactar con Nexode.".to_string())?,
    )
}
pub fn connector_secret(payload: &str) -> Result<Zeroizing<String>, String> {
    let value: Value =
        serde_json::from_str(payload).map_err(|_| "Conector inválido.".to_string())?;
    let key = value
        .pointer("/mcpServers/nexode/env/NEXODE_API_KEY")
        .and_then(Value::as_str)
        .filter(|s| s.starts_with("nxd_live_") && s.len() > 20)
        .ok_or("La cuenta no devolvió un conector Nexode válido.")?;
    Ok(Zeroizing::new(key.into()))
}
#[derive(Default)]
pub struct Account {
    pub email: String,
    pub workspace: String,
    pub plans: Vec<Value>,
    pub usage: Value,
}
fn account(key: &str) -> Result<Account, String> {
    let me = sdk(key, "me", None)?;
    Ok(Account {
        email: me
            .pointer("/user/email")
            .and_then(Value::as_str)
            .unwrap_or("Cuenta Nexode")
            .into(),
        workspace: me
            .pointer("/workspace/id")
            .and_then(Value::as_str)
            .unwrap_or("Sin workspace activo")
            .into(),
        // A missing paid entitlement must not prevent use of the harness.
        plans: sdk(key, "subscriptions/plans", None)
            .ok()
            .and_then(|v| v.get("data").and_then(Value::as_array).cloned())
            .unwrap_or_default(),
        usage: sdk(key, "billing/usage", None).unwrap_or(Value::Null),
    })
}
enum Action {
    Restore,
    Login(String, Zeroizing<String>),
    Register(String, Zeroizing<String>),
    Checkout(String),
    Logout,
}
enum Outcome {
    Account(Account),
    Registered,
    Checkout(String),
    LoggedOut,
    NoAccount,
}
pub struct Cloud {
    pub open: bool,
    pub account: Option<Account>,
    pub email: String,
    password: String,
    pub message: String,
    receiver: Option<Receiver<Result<Outcome, String>>>,
    pub checkout: Option<String>,
    selected_plan: Option<(String, String)>,
}
impl Drop for Cloud {
    fn drop(&mut self) {
        self.password.zeroize();
    }
}
impl Cloud {
    pub fn new(ctx: &egui::Context) -> Self {
        let mut cloud = Self {
            open: false,
            account: None,
            email: String::new(),
            password: String::new(),
            message: String::new(),
            receiver: None,
            checkout: None,
            selected_plan: None,
        };
        if std::env::var_os("ORBIT_NATIVE_TEST").is_none() { cloud.start(Action::Restore, ctx); }
        cloud
    }
    fn start(&mut self, action: Action, ctx: &egui::Context) {
        if self.receiver.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.receiver = Some(rx);
        self.message.clear();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(execute(action));
            ctx.request_repaint();
        });
    }
    /// True when the private engine must be restarted to change connector access.
    pub fn poll(&mut self) -> bool {
        let Some(result) = self.receiver.as_ref().and_then(|r| r.try_recv().ok()) else {
            return false;
        };
        self.receiver = None;
        match result {
            Ok(Outcome::Account(a)) => {
                self.account = Some(a);
                self.message = "Cuenta conectada. MCP preparado para tu workspace.".into();
                true
            }
            Ok(Outcome::Registered) => {
                self.message="Revisa el enlace de verificación enviado a tu correo. Después inicia sesión aquí.".into();
                false
            }
            Ok(Outcome::Checkout(url)) => {
                self.checkout = Some(url);
                self.message="Checkout preparado. El pago se confirma en Stripe; no se ha cobrado nada desde Orbit.".into();
                false
            }
            Ok(Outcome::LoggedOut) => {
                self.account = None;
                self.checkout = None;
                self.message = "Conector eliminado de este dispositivo.".into();
                true
            }
            Ok(Outcome::NoAccount) => false,
            Err(e) => {
                self.message = e;
                false
            }
        }
    }
    pub fn ui(&mut self, ctx: &egui::Context, engine_busy: bool) {
        let mut open = self.open;
        egui::Window::new("Mi cuenta Nexode").open(&mut open).default_width(520.0).show(ctx, |ui| {
            ui.add(crate::widgets::mascot(72.0));
            ui.heading("Tu cuenta. Todos tus agentes.");
            ui.label("Orbit es gratuito: conecta tus proveedores sin contratar un plan Nexode. El consumo de IA se factura según tu proveedor.");
            ui.add_space(12.0);
            let busy=self.receiver.is_some();
            ui.add_enabled_ui(!busy && !engine_busy, |ui| {
                if let Some(a)=&self.account {
                    ui.strong(&a.email); ui.label(format!("Workspace: {}", a.workspace));
                    ui.add_space(10.0); ui.strong("Servicios y créditos Nexode");
                    if a.usage.is_null() { ui.label("Saldo no disponible. El acceso al harness sigue activo."); }
                    else { billing_summary(ui, &a.usage); }
                    let plans=a.plans.clone();
                    for plan in plans.iter().filter(|p| p["purchasable"]==true) {
                        let slug=plan["slug"].as_str().unwrap_or_default();
                        let label=format!("{} · {} USD / mes",plan["name"].as_str().unwrap_or(slug),plan["price_monthly"]);
                        if ui.button(&label).clicked(){self.selected_plan=Some((slug.into(),label));}
                    }
                    if let Some((slug,label))=self.selected_plan.clone() {
                        ui.separator(); ui.label(format!("Contratar {label}. Renovación mensual. Revisa impuestos y total en Stripe."));
                        if ui.button("Preparar checkout seguro").clicked(){self.start(Action::Checkout(slug),ctx); self.selected_plan=None;}
                    }
                    if let Some(url)=&self.checkout { ui.hyperlink_to("Abrir Stripe y revisar el pago",url); }
                    ui.hyperlink_to("Gestionar facturación en Nexode", "https://cloud.nexode.app/dashboard");
                    ui.separator();
                    if ui.button("Actualizar cuenta y saldo").clicked(){self.start(Action::Restore,ctx);}
                    if ui.button("Cerrar sesión en este dispositivo").clicked(){self.start(Action::Logout,ctx);}
                } else {
                    ui.label("Correo electrónico"); ui.text_edit_singleline(&mut self.email);
                    ui.label("Contraseña"); ui.add(egui::TextEdit::singleline(&mut self.password).password(true));
                    let valid=self.email.contains('@') && !self.password.is_empty();
                    ui.horizontal(|ui| {
                        if ui.add_enabled(valid,egui::Button::new("Iniciar sesión")).clicked(){ let password=Zeroizing::new(std::mem::take(&mut self.password)); self.start(Action::Login(self.email.trim().into(),password),ctx); }
                        if ui.add_enabled(valid && self.password.len()>=6,egui::Button::new("Crear cuenta Nexode")).clicked(){let password=Zeroizing::new(std::mem::take(&mut self.password));self.start(Action::Register(self.email.trim().into(),password),ctx);}
                    });
                    ui.hyperlink_to("Recuperar contraseña / acceso con Google", "https://cloud.nexode.app/auth/login");
                    ui.label("El acceso con Google se gestiona en Nexode. Esta versión conecta el escritorio mediante correo y contraseña.");
                }
            });
            if busy { ui.label("Conectando con Nexode…"); }
            if !self.message.is_empty(){ui.label(&self.message);}
            ui.label("Las credenciales se guardan en el almacén seguro del sistema, nunca en el chat.");
        });
        self.open = open;
    }
}
fn billing_summary(ui: &mut egui::Ui, usage: &Value) {
    // Render only known numeric billing fields; no raw server payloads.
    for (key, label) in [
        ("available_usd", "Saldo disponible USD"),
        ("remaining_usd", "Créditos del plan USD"),
        ("wallet_usd", "Saldo recargado USD"),
    ] {
        if let Some(value) = usage.get(key).filter(|v| v.is_number()) {
            ui.label(format!("{label}: {value}"));
        }
    }
    ui.hyperlink_to(
        "Ver consumo detallado",
        "https://cloud.nexode.app/dashboard",
    );
}
fn execute(action: Action) -> Result<Outcome, String> {
    match action {
        Action::Login(email, password) => {
            let c = client(true)?;
            let login = gql(
                &c,
                "mutation($input:LoginInput!){login(input:$input){success}}",
                json!({"input":{"email":email,"password":password.as_str()}}),
            )?;
            if login["login"]["success"] != true {
                return Err(
                    "No se pudo iniciar sesión. Verifica el correo y la contraseña.".into(),
                );
            }
            let payload = gql(&c, "mutation{prepareMcpConnection}", json!({}))?;
            let key = connector_secret(
                payload["prepareMcpConnection"]
                    .as_str()
                    .ok_or("Conector no disponible.")?,
            )?;
            let a = account(&key)?;
            entry()?.set_password(&key).map_err(|_| {
                "No se pudo guardar la conexión en el almacén seguro del sistema.".to_string()
            })?;
            Ok(Outcome::Account(a))
        }
        Action::Register(email, password) => {
            let data = gql(
                &client(false)?,
                "mutation($input:RegisterInput!){register(input:$input){success}}",
                json!({"input":{"email":email,"password":password.as_str()}}),
            )?;
            if data["register"]["success"] != true {
                return Err("No se pudo registrar la cuenta.".into());
            }
            Ok(Outcome::Registered)
        }
        Action::Restore => match credential()? {
            Some(key) => account(&key).map(Outcome::Account),
            None => Ok(Outcome::NoAccount),
        },
        Action::Checkout(slug) => {
            if !["nexode-hobby", "nexode-pro", "nexode-team"].contains(&slug.as_str()) {
                return Err("Plan no compatible.".into());
            }
            let key = credential()?.ok_or("Inicia sesión en Nexode.")?;
            let data = sdk(
                &key,
                "subscriptions/checkout",
                Some(json!({"plan_slug":slug,"billing_cycle":"monthly"})),
            )?;
            let url = data["checkout_url"]
                .as_str()
                .ok_or("Checkout no disponible.")?;
            if !valid_checkout(url) {
                return Err("El checkout no pertenece al proveedor de pago esperado.".into());
            }
            Ok(Outcome::Checkout(url.into()))
        }
        Action::Logout => {
            match entry()?.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => {}
                Err(_) => return Err("No se pudo eliminar la conexión del almacén seguro.".into()),
            }
            Ok(Outcome::LoggedOut)
        }
    }
}
fn valid_checkout(url: &str) -> bool {
    reqwest::Url::parse(url).is_ok_and(|u| {
        u.scheme() == "https"
            && u.host_str() == Some("checkout.stripe.com")
            && u.username().is_empty()
            && u.password().is_none()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn connector_accepts_only_the_user_key_not_executable_payload() {
        let payload=json!({"mcpServers":{"nexode":{"command":"evil","env":{"NEXODE_API_KEY":"nxd_live_12345678901234567890"}}}}).to_string();
        assert_eq!(
            connector_secret(&payload).unwrap().as_str(),
            "nxd_live_12345678901234567890"
        );
        assert!(connector_secret("{}").is_err());
    }
    #[test]
    fn checkout_cannot_redirect_to_lookalike_or_local_endpoint() {
        assert!(valid_checkout("https://checkout.stripe.com/c/pay/cs_test"));
        for u in [
            "http://checkout.stripe.com/x",
            "https://checkout.stripe.com.evil/x",
            "https://evil@checkout.stripe.com/x",
            "file:///tmp/pay",
        ] {
            assert!(!valid_checkout(u));
        }
    }
}
