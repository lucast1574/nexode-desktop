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
];
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
pub fn panel(ctx: &egui::Context, open: &mut bool, query: &mut String, draft: &mut String) {
    egui::Window::new("Servicios Nexode").open(open).default_width(620.0).show(ctx,|ui|{
        ui.heading("De tu idea a un servicio real");
        ui.label("Opciones del ecosistema, priorizadas según lo que necesitas. Cada servicio conserva sus permisos y condiciones.");
        ui.add(egui::TextEdit::singleline(query).hint_text("¿Qué quieres construir? Mapas, WhatsApp, publicar…").desired_width(f32::INFINITY));
        egui::ScrollArea::vertical().max_height(480.0).show(ui,|ui|{
            for s in ranked(query){
                ui.add_space(12.0);ui.strong(s.name);ui.label(s.purpose);
                ui.horizontal(|ui|{
                    ui.hyperlink_to("Conocer el servicio",s.url);
                    if ui.button("Preparar con Orbit").clicked(){*draft=format!("Quiero usar {} para mi proyecto. Revisa {} y la documentación pública vigente. Comprueba qué acceso necesito y prepara un plan con costes antes de desplegar o contratar nada.",s.name,s.url);}
                });ui.separator();
            }
        });
    });
}
#[cfg(test)]
mod tests {
    use super::*;
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
