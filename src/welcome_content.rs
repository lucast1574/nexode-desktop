//! Self-contained welcome document. Assets are embedded; no server or network.
use base64::{Engine, engine::general_purpose::STANDARD};

pub fn html() -> String {
    include_str!("../assets/welcome.html")
        .replace(
            "__NEXO_DATA_URI__",
            &format!(
                "data:image/webp;base64,{}",
                STANDARD.encode(include_bytes!("../assets/nexo.webp"))
            ),
        )
        .replace(
            "__FONT_REGULAR__",
            &format!(
                "data:font/ttf;base64,{}",
                STANDARD.encode(include_bytes!("../assets/fonts/Geist-Regular.ttf"))
            ),
        )
        .replace(
            "__FONT_MEDIUM__",
            &format!(
                "data:font/ttf;base64,{}",
                STANDARD.encode(include_bytes!("../assets/fonts/Geist-Medium.ttf"))
            ),
        )
}
