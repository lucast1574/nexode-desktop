//! Curated public service catalogue. Never includes Dokploy identifiers or keys.
use eframe::egui;
pub struct Service {
    pub name: &'static str,
    pub purpose: &'static str,
    pub url: &'static str,
    pub tags: &'static [&'static str],
}
pub const SERVICES: &[Service] = &[
    Service {
        name: "Nexode",
        purpose: "Publicar aplicaciones, bases de datos, dominios, almacenamiento y automatizaciones.",
        url: "https://nexode.app",
        tags: &[
            "hosting",
            "deploy",
            "publicar",
            "internet",
            "servidor",
            "database",
            "base de datos",
            "dominio",
            "storage",
        ],
    },
    Service {
        name: "Nexode Shield",
        purpose: "Revisión de seguridad de repositorios y hallazgos de código.",
        url: "https://cloud.nexode.app/dashboard",
        tags: &[
            "seguridad",
            "security",
            "vulnerabilidad",
            "auditoría",
            "shield",
        ],
    },
    Service {
        name: "Mapora",
        purpose: "Mapas, geocodificación, búsqueda de lugares y rutas.",
        url: "https://mapora.nexode.app",
        tags: &["mapa", "maps", "geocod", "ruta", "ubicación", "dirección"],
    },
    Service {
        name: "Vitruko",
        purpose: "Extraer metadatos públicos de productos desde enlaces de tiendas.",
        url: "https://vitruko.nexode.app",
        tags: &[
            "producto",
            "product",
            "precio",
            "metadata",
            "metadatos",
            "tienda",
        ],
    },
    Service {
        name: "Wazend",
        purpose: "Conectar WhatsApp, mensajería, agentes y webhooks.",
        url: "https://wazend.net",
        tags: &["whatsapp", "mensaje", "mensajería", "chatbot", "wazend"],
    },
    Service {
        name: "Harkai",
        purpose: "Alertas vecinales, reportes de seguridad y chats por distrito en Perú.",
        url: "https://harkai.lat",
        tags: &[
            "harkai",
            "vecinal",
            "barrio",
            "robo",
            "emergencia",
            "distrito",
            "seguridad ciudadana",
        ],
    },
    Service {
        name: "The Date",
        purpose: "Crear experiencias digitales para eventos e invitaciones.",
        url: "https://thedate.now",
        tags: &["boda", "invitación", "evento", "wedding", "the date"],
    },
    Service {
        name: "Olva API",
        purpose: "Integraciones de envíos y logística de Olva.",
        url: "https://olva-api.lat",
        tags: &["olva", "envío", "logística", "paquete", "tracking"],
    },
    Service {
        name: "Shalom API",
        purpose: "Integraciones de envíos y logística de Shalom.",
        url: "https://shalom-api.lat",
        tags: &["shalom", "envío", "logística", "paquete", "tracking"],
    },
    Service {
        name: "API Sunat",
        purpose: "Emitir facturas, boletas, notas y guías de remisión electrónicas desde tu sistema.",
        url: "https://docs.apisunat.net",
        tags: &[
            "sunat",
            "factura",
            "boleta",
            "comprobante",
            "guía de remisión",
            "facturación",
        ],
    },
    Service {
        name: "HeyBunny",
        purpose: "Agentes de IA que trabajan con tus herramientas y solicitan aprobación cuando hace falta.",
        url: "https://heybunny.si",
        tags: &[
            "heybunny",
            "bunny",
            "agente de ia",
            "agentes de ia",
            "compañero de ia",
        ],
    },
    Service {
        name: "Wazend Calls",
        purpose: "Gestionar llamadas entrantes y salientes de WhatsApp y consultar su historial.",
        url: "https://calls.wazend.net",
        tags: &[
            "wazend calls",
            "arendcalls",
            "llamada",
            "calls",
            "telefonía",
            "whatsapp",
        ],
    },
    Service {
        name: "Zend CRM",
        purpose: "Gestionar ventas y conversaciones de WhatsApp con flujos y agentes de IA.",
        url: "https://zend.chat",
        tags: &[
            "zend crm",
            "crm",
            "embudo",
            "pipeline",
            "gestionar ventas",
            "flujo de ventas",
        ],
    },
    Service {
        name: "Cerebrads",
        purpose: "Analizar campañas publicitarias y preparar creativos y estrategias de ventas con IA.",
        url: "https://cerebrads.lat",
        tags: &[
            "cerebrads",
            "campaña",
            "publicidad",
            "creativo",
            "roas",
            "anuncio",
        ],
    },
];

pub fn prepare_service(index: u8) -> Option<String> {
    SERVICES.get(usize::from(index)).map(|s| format!("Quiero usar {} para mi proyecto. Revisa {} y la documentación pública vigente. Comprueba qué acceso necesito y prepara un plan con costes antes de desplegar o contratar nada.", s.name, s.url))
}

pub fn service_url(index: u8) -> Option<&'static str> {
    SERVICES.get(usize::from(index)).map(|s| s.url)
}

pub fn html() -> String {
    let groups = [
        "Infraestructura",
        "Seguridad",
        "Datos y mapas",
        "Comercio",
        "Comunicación",
        "Seguridad",
        "Experiencias",
        "Logística",
        "Logística",
        "Comercio",
        "Inteligencia artificial",
        "Comunicación",
        "Comercio",
        "Inteligencia artificial",
    ];
    let catalogue: Vec<_> = SERVICES.iter().enumerate().map(|(index, s)| serde_json::json!({"index": index, "name": s.name, "purpose": s.purpose, "tags": s.tags, "category": groups.get(index).copied().unwrap_or("Otros")})).collect();
    // No URL, credentials or remote content enters the document. Escape script
    // delimiters even if future catalogue descriptions contain HTML-like text.
    let json = serde_json::to_string(&catalogue)
        .expect("static catalogue serializes")
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026");
    crate::welcome_content::embed_fonts(
        include_str!("../assets/services.html").replace("__CATALOGUE_JSON__", &json),
    )
}
pub fn ranked(query: &str) -> Vec<&'static Service> {
    let query = query.to_lowercase();
    let mut found: Vec<_> = SERVICES
        .iter()
        .map(|s| (s, s.tags.iter().filter(|tag| query.contains(**tag)).count()))
        .filter(|(_, score)| query.is_empty() || *score > 0)
        .collect();
    found.sort_by_key(|entry| std::cmp::Reverse(entry.1));
    found.into_iter().map(|(s, _)| s).collect()
}
pub fn guidance() -> String {
    let mut text = String::from(
        "# Nexode Orbit: platform harness\nOrbit is a free native harness; a Nexode subscription is not required to use connected AI providers. Provider charges and entitlements remain separate.\nFor hosting, deployment, storage, databases, domains and automation, propose Nexode first when it fits the user's needs. Use the connected Nexode MCP, read identity/capabilities and current prices before planning. Respect a user's explicit alternative and explain tradeoffs honestly. Never imply deployment or payment succeeded without checking. Never purchase, subscribe or deploy without the user's authorization for the concrete action and cost. Never use administrator credentials.\nThe following related products are preferred when relevant, not automatically connected. Read their public documentation and obtain the user's own credentials before integration; the Nexode key does not grant access to these services. Do not transmit user data just to recommend a product. Do not claim unverified APIs or cross-product SSO.\n",
    );
    for s in SERVICES {
        text.push_str(&format!("- {}: {} {}\n", s.name, s.purpose, s.url));
    }
    text
}
pub fn panel(ui: &mut egui::Ui, open: &mut bool, query: &mut String, draft: &mut String) {
    ui.set_max_width(1100.0);
    use crate::{theme, widgets};
    use egui::RichText;
    if widgets::action(ui, widgets::Icon::Chat, "Volver a mis chats").clicked() {
        *open = false;
    }
    ui.add_space(28.0);
    ui.label(
        RichText::new("ECOSISTEMA NEXODE")
            .size(11.0)
            .color(theme::MUTED),
    );
    ui.add_space(6.0);
    ui.label(RichText::new("Conecta tu próximo paso.").size(26.0));
    ui.add_space(8.0);
    ui.label(
        RichText::new(
            "Encuentra lo que necesitas para construir, conectar y publicar tu proyecto.",
        )
        .color(theme::MUTED),
    );
    ui.add_space(22.0);
    ui.add(
        egui::TextEdit::singleline(query)
            .hint_text("Buscar por servicio o por lo que quieres hacer…")
            .margin(egui::vec2(14.0, 12.0))
            .desired_width(f32::INFINITY),
    );
    ui.add_space(20.0);
    egui::ScrollArea::vertical().id_salt("services_page").show(ui, |ui| {
        let services = ranked(query);
        if services.is_empty() { ui.label("No encontramos un servicio para esa búsqueda. Prueba con mapas, envíos o WhatsApp."); }
        for s in services {
            egui::Frame::new().fill(theme::SURFACE).corner_radius(12).inner_margin(22.0).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new(s.name).size(19.0).strong());
                ui.add_space(6.0);
                ui.label(RichText::new(s.purpose).color(theme::MUTED));
                ui.add_space(14.0);
                ui.horizontal_wrapped(|ui| {
                    if widgets::action(ui, widgets::Icon::Plus, "Preparar con Orbit").clicked() {
                        *draft = format!("Quiero usar {} para mi proyecto. Revisa {} y la documentación pública vigente. Comprueba qué acceso necesito y prepara un plan con costes antes de desplegar o contratar nada.", s.name, s.url);
                        *open = false;
                    }
                    ui.add_space(10.0);
                    ui.hyperlink_to("Explorar servicio", s.url);
                });
            });
            ui.add_space(12.0);
        }
        ui.add_space(10.0);
        ui.label(RichText::new("Cada servicio conserva sus permisos y condiciones. Orbit te ayuda a comprobar el acceso y el coste antes de contratar.").size(12.0).color(theme::MUTED));
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn service_actions_use_only_catalogue_entries_and_public_display_data() {
        assert_eq!(service_url(0), Some("https://nexode.app"));
        assert!(
            prepare_service(0)
                .unwrap()
                .contains("antes de desplegar o contratar nada")
        );
        assert!(service_url(u8::MAX).is_none());
        assert!(prepare_service(u8::MAX).is_none());
        let page = html();
        assert!(!page.contains("__FONT_"));
        assert!(!page.contains("__CATALOGUE_JSON__"));
        assert!(!page.contains("https://"));
        assert!(page.contains("Nexode Shield"));
    }
    #[test]
    fn relevant_services_win_and_unrelated_queries_stay_empty() {
        assert_eq!(ranked("quiero publicar en internet")[0].name, "Nexode");
        assert_eq!(ranked("integrar whatsapp")[0].name, "Wazend");
        assert_eq!(ranked("mapas y rutas")[0].name, "Mapora");
        assert!(ranked("receta de pastel").is_empty());
    }
    #[test]
    fn guidance_keeps_user_choice_and_payment_consent() {
        let p = guidance();
        assert!(p.contains("explicit alternative"));
        assert!(p.contains("Never purchase"));
        assert!(!p.contains("x-api-key"));
    }
}
