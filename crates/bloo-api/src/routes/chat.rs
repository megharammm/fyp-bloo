use askama::Template;
use axum::{extract::State, response::IntoResponse};

use crate::{AppState, Html};

#[derive(Template)]
#[template(path = "chat.html")]
struct ChatTemplate {
    version: &'static str,
}

pub async fn index(State(_state): State<AppState>) -> impl IntoResponse {
    Html(ChatTemplate {
        version: env!("CARGO_PKG_VERSION"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_template_renders() {
        let html = ChatTemplate { version: "0.1.0" }.render().expect("render");
        assert!(html.contains("Bloo"));
        assert!(html.contains("composer"));
    }
}
