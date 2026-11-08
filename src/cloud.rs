//! Nexode account API. Desktop passwords are transient; only the user's own
//! connector is kept in the OS credential vault. No administrator credentials.
use eframe::egui;
#[path = "cloud_browser.rs"]
mod browser;
#[path = "cloud_ui.rs"]
mod ui;
pub use browser::BrowserLogin;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::{
    sync::mpsc::{self, Receiver},
    time::{Duration, Instant},
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
    BrowserStart,
    BrowserPoll(Zeroizing<String>, Zeroizing<String>),
    BrowserSave(Account, Zeroizing<String>),
    Restore,
    Login(String, Zeroizing<String>),
    Register(String, Zeroizing<String>),
    Checkout(String),
    Logout,
}
enum Outcome {
    BrowserStarted(BrowserLogin),
    BrowserPending(Duration),
    BrowserApproved(Account, Zeroizing<String>),
    Account(Account),
    Registered,
    Checkout(String),
    LoggedOut,
    NoAccount,
}
pub struct Cloud {
    ctx: egui::Context,
    pub browser_login: Option<BrowserLogin>,
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
            ctx: ctx.clone(),
            browser_login: None,
            open: false,
            account: None,
            email: String::new(),
            password: String::new(),
            message: String::new(),
            receiver: None,
            checkout: None,
            selected_plan: None,
        };
        #[cfg(feature = "screenshot")]
        if std::env::var_os("ORBIT_NATIVE_TEST").is_some()
            && std::env::var_os("ORBIT_TEST_ACCOUNT").is_some()
        {
            cloud.account = Some(Account {
                email: "orbit@example.invalid".into(),
                workspace: "test".into(),
                plans: vec![],
                usage: Value::Null,
            });
            return cloud;
        }
        if std::env::var_os("ORBIT_NATIVE_TEST").is_none() {
            cloud.start(Action::Restore, ctx);
        }
        cloud
    }
    pub fn start_browser_login(&mut self, ctx: &egui::Context) {
        if self.browser_login.is_none() {
            self.start(Action::BrowserStart, ctx);
        }
    }
    /// Called each update. Only the active five-minute authorization requests
    /// timed repaints; the connector is never part of the browser URL.
    pub fn tick_browser(&mut self, ctx: &egui::Context) {
        let Some(login) = &mut self.browser_login else {
            return;
        };
        if Instant::now() >= login.expires_at {
            self.browser_login = None;
            self.message = "La autorización venció. Vuelve a conectar con Google.".into();
            return;
        }
        if !login.opened {
            ctx.open_url(egui::OpenUrl::new_tab(login.verification_url.clone()));
            login.opened = true;
        }
        if !login.committing && self.receiver.is_none() && Instant::now() >= login.next_poll {
            login.next_poll = Instant::now() + Duration::from_secs(5);
            let action = Action::BrowserPoll(login.device_code.clone(), login.verifier.clone());
            self.start(action, ctx);
        }
        ctx.request_repaint_after(Duration::from_secs(1));
    }
    /// Poll workers never persist credentials. Dropping their receiver safely
    /// discards late results; only the short local vault commit disables cancel.
    pub fn cancel_browser_login(&mut self) {
        if self
            .browser_login
            .as_ref()
            .is_some_and(|login| !login.committing)
        {
            self.receiver = None;
            self.browser_login = None;
            self.message.clear();
        }
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
            Ok(Outcome::BrowserStarted(login)) => {
                self.browser_login = Some(login);
                false
            }
            Ok(Outcome::BrowserPending(delay)) => {
                if let Some(login) = &mut self.browser_login {
                    login.next_poll = Instant::now() + delay;
                }
                false
            }
            Ok(Outcome::BrowserApproved(account, key)) => {
                if let Some(login) = &mut self.browser_login {
                    login.committing = true;
                    self.start(Action::BrowserSave(account, key), &self.ctx.clone());
                }
                false
            }
            Ok(Outcome::Account(a)) => {
                self.browser_login = None;
                if self.account.is_none() {
                    self.open = false;
                }
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
                self.browser_login = None;
                self.account = None;
                self.checkout = None;
                self.message = "Conector eliminado de este dispositivo.".into();
                true
            }
            Ok(Outcome::NoAccount) => {
                self.open = true;
                false
            }
            Err(e) => {
                self.browser_login = None;
                self.message = e;
                false
            }
        }
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
        Action::BrowserStart => browser::begin().map(Outcome::BrowserStarted),
        Action::BrowserPoll(device_code, verifier) => browser::exchange(&device_code, &verifier),
        Action::BrowserSave(account, key) => {
            entry()?.set_password(&key).map_err(|_| {
                "No se pudo guardar la conexión en el almacén seguro del sistema.".to_string()
            })?;
            Ok(Outcome::Account(account))
        }
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
