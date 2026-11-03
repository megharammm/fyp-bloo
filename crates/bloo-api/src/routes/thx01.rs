use askama::Template;
use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::time::{Duration, Instant};

use crate::{AppState, Html};

#[derive(Template)]
#[template(path = "thx01.html")]
struct Thx01Template {
    version: &'static str,
    base_url: String,
}

pub async fn page(State(state): State<AppState>) -> impl IntoResponse {
    Html(Thx01Template {
        version: env!("CARGO_PKG_VERSION"),
        base_url: state.cfg.halx_base_url.clone(),
    })
}

fn upstream_base(cfg: &bloo_core::Config) -> String {
    cfg.halx_base_url.trim_end_matches('/').to_string()
}

fn with_auth(req: reqwest::RequestBuilder, cfg: &bloo_core::Config) -> reqwest::RequestBuilder {
    match cfg.halx_api_key.as_deref() {
        Some(k) if !k.trim().is_empty() => req.bearer_auth(k.trim()),
        _ => req,
    }
}

/// `GET /api/thx01/status` — is the decision model reachable right now?
pub async fn status(State(state): State<AppState>) -> impl IntoResponse {
    let base = upstream_base(&state.cfg);
    let url = format!("{base}/v1/models");

    let req = state.http.get(&url).timeout(Duration::from_secs(5));
    let req = with_auth(req, &state.cfg);

    match req.send().await {
        Ok(resp) => {
            let code = resp.status();
            let body: Value = resp.json().await.unwrap_or(Value::Null);
            (
                StatusCode::OK,
                Json(json!({
                    "reachable": code.is_success(),
                    "http_status": code.as_u16(),
                    "base_url": base,
                    "decider": state.cfg.decider(),
                    "models": body,
                    "hint": if code.is_success() { Value::Null } else { json!(
                        "The endpoint answered, but not with 200. See http_status."
                    ) },
                })),
            )
        }
        Err(err) => (
            StatusCode::OK,
            Json(json!({
                "reachable": false,
                "base_url": base,
                "decider": state.cfg.decider(),
                "error": err.to_string(),
                "hint": "THX-01 is not answering. Start it with scripts\\start-thx01.ps1 (host, recommended) \
                         or docker compose -p blooai --profile thx01 up -d thx01 (port 27812).",
            })),
        ),
    }
}

#[derive(Deserialize)]
pub struct DecideBody {
    pub state: Value,
    pub questions: Value,
    #[serde(default)]
    pub model: Option<String>,
}

/// `POST /api/thx01/decide` — proxy one decision request to the configured THX-01 server.
///
/// The browser never talks to the model directly: no CORS, and the key stays server-side.
pub async fn decide(State(state): State<AppState>, Json(body): Json<DecideBody>) -> Response {
    if !body.questions.is_object() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "ok": false, "error": "questions must be a JSON object" })),
        )
            .into_response();
    }

    let base = upstream_base(&state.cfg);
    let url = format!("{base}/v1/systemone");
    let payload = json!({
        "model": body.model.unwrap_or_else(|| "thx-01".to_string()),
        "state": body.state,
        "questions": body.questions,
    });

    let started = Instant::now();
    let req = state
        .http
        .post(&url)
        .timeout(Duration::from_secs(180))
        .json(&payload);
    let req = with_auth(req, &state.cfg);

    match req.send().await {
        Ok(resp) => {
            let code = resp.status();
            let elapsed = started.elapsed().as_millis() as u64;
            let text = resp.text().await.unwrap_or_default();

            match serde_json::from_str::<Value>(&text) {
                Ok(upstream) if code.is_success() => (
                    StatusCode::OK,
                    Json(json!({
                        "ok": true,
                        "elapsed_ms": elapsed,
                        "base_url": base,
                        "upstream": upstream,
                    })),
                )
                    .into_response(),
                Ok(upstream) => (
                    StatusCode::BAD_GATEWAY,
                    Json(json!({
                        "ok": false,
                        "elapsed_ms": elapsed,
                        "base_url": base,
                        "http_status": code.as_u16(),
                        "upstream": upstream,
                        "hint": "The model server returned an error. Check the raw response.",
                    })),
                )
                    .into_response(),
                Err(_) => (
                    StatusCode::BAD_GATEWAY,
                    Json(json!({
                        "ok": false,
                        "elapsed_ms": elapsed,
                        "base_url": base,
                        "http_status": code.as_u16(),
                        "error": "upstream did not return JSON",
                        "body": text.chars().take(800).collect::<String>(),
                    })),
                )
                    .into_response(),
            }
        }
        Err(err) => (
            StatusCode::BAD_GATEWAY,
            Json(json!({
                "ok": false,
                "elapsed_ms": started.elapsed().as_millis() as u64,
                "base_url": base,
                "error": err.to_string(),
                "hint": "THX-01 is not answering. Start it with scripts\\start-thx01.ps1 \
                         (or the docker --profile thx01 service on port 27812).",
            })),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn console_template_renders() {
        let html = Thx01Template {
            version: "0.1.0",
            base_url: "http://127.0.0.1:8095".to_string(),
        }
        .render()
        .expect("render");

        assert!(html.contains("THX-01 console"));
        assert!(html.contains("Decide"));
        assert!(html.contains("127.0.0.1:8095"));
    }
}
