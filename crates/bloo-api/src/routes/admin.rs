use askama::Template;
use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};
use serde_json::json;

use crate::{AppState, Html};

#[derive(Template)]
#[template(path = "admin.html")]
struct AdminTemplate {
    version: &'static str,
}

pub async fn index(State(_state): State<AppState>) -> impl IntoResponse {
    Html(AdminTemplate {
        version: env!("CARGO_PKG_VERSION"),
    })
}

/// Content sync: scan `content/` + `data/uploads`, hash each file, enqueue only
/// new or changed documents. Implemented in P1/P5 — see PLAYBOOK.md.
pub async fn sync(State(state): State<AppState>) -> impl IntoResponse {
    let _ = &state.content_dir_placeholder();
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(json!({
            "ok": false,
            "todo": "P1: implement content sync (scan + sha256 + enqueue changed)",
        })),
    )
}

impl AppState {
    fn content_dir_placeholder(&self) -> String {
        self.cfg.content_dir.display().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admin_template_renders() {
        let html = AdminTemplate { version: "0.1.0" }.render().expect("render");
        assert!(html.contains("Knowledge console"));
        assert!(html.contains("27432"));
    }
}
