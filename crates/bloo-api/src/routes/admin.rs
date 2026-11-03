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

/// `POST /api/admin/sync` — diff `content/` against the database and ingest what changed.
pub async fn sync(State(state): State<AppState>) -> impl IntoResponse {
    match bloo_ingest::sync::sync_content(&state.pool, &state.cfg).await {
        Ok(report) => (StatusCode::OK, Json(report)).into_response(),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": err.to_string() })),
        )
            .into_response(),
    }
}
