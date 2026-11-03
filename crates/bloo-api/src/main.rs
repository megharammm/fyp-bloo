use anyhow::Context;
use askama::Template;
use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::{Html as AxumHtml, IntoResponse, Response},
    routing::get,
};
use bloo_core::{Config, PgPool};
use serde_json::json;
use std::sync::Arc;
use tower_http::{services::ServeDir, trace::TraceLayer};

mod routes;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub cfg: Arc<Config>,
    /// Shared HTTP client for THX-01 (and the chat LLM).
    pub http: reqwest::Client,
    /// Local ONNX embedder, also used to embed incoming questions.
    pub embedder: Arc<dyn bloo_rag::Embedder>,
}

/// Renders an Askama template into an `IntoResponse`.
pub struct Html<T: Template>(pub T);

/// Renders an Askama template into a full `Response` (used with `.into_response()`).
pub struct RenderedHtml(pub String);

impl IntoResponse for RenderedHtml {
    fn into_response(self) -> Response {
        AxumHtml(self.0).into_response()
    }
}

impl<T: Template> IntoResponse for Html<T> {
    fn into_response(self) -> Response {
        match self.0.render() {
            Ok(body) => RenderedHtml(body).into_response(),
            Err(err) => {
                tracing::error!(error = %err, "template render failed");
                (StatusCode::INTERNAL_SERVER_ERROR, "template error").into_response()
            }
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,bloo_api=debug,bloo_core=debug".into()),
        )
        .compact()
        .init();

    let cfg = Config::from_env().context("invalid configuration")?;
    tracing::info!(bind = %cfg.bind, "starting Bloo AI");

    let pool = bloo_core::connect(&cfg.database_url).await.context(
        "cannot reach Postgres - is `docker compose -p blooai up -d db` running on port 27432?",
    )?;

    if cfg.auto_migrate {
        bloo_core::migrate(&pool)
            .await
            .context("migration failed")?;
        tracing::info!("migrations applied");
    }

    let embedder = bloo_rag::build(&cfg)
        .context("cannot build the embedding model (see BLOO_EMBED_PROVIDER)")?;
    tracing::info!(
        embedder = embedder.name(),
        dim = embedder.dim(),
        "embedder ready"
    );

    let state = AppState {
        pool,
        cfg: Arc::new(cfg),
        http: reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(180))
            .user_agent(concat!("BlooAI/", env!("CARGO_PKG_VERSION")))
            .build()
            .context("cannot build the HTTP client")?,
        embedder,
    };

    let app = Router::new()
        .route("/", get(routes::chat::index))
        .route("/admin", get(routes::admin::index))
        .route("/thx01", get(routes::thx01::page))
        .route("/health", get(health))
        .route("/api/version", get(version))
        .route("/api/thx01/status", get(routes::thx01::status))
        .route("/api/search", get(routes::ask::search))
        .route("/api/chat", axum::routing::post(routes::ask::chat))
        .route("/api/feedback", axum::routing::post(routes::ask::feedback))
        .route(
            "/api/thx01/decide",
            axum::routing::post(routes::thx01::decide),
        )
        .route(
            "/api/admin/documents",
            axum::routing::get(routes::documents::list)
                .post(routes::documents::upload)
                .layer(axum::extract::DefaultBodyLimit::max(50 * 1024 * 1024)),
        )
        .route(
            "/api/admin/documents/{id}",
            axum::routing::get(routes::documents::detail)
                .patch(routes::documents::patch)
                .delete(routes::documents::delete),
        )
        .route(
            "/api/admin/documents/{id}/chunks",
            axum::routing::get(routes::documents::chunks),
        )
        .route(
            "/api/admin/documents/{id}/publish",
            axum::routing::post(routes::documents::publish),
        )
        .route(
            "/api/admin/documents/{id}/unpublish",
            axum::routing::post(routes::documents::unpublish),
        )
        .route(
            "/api/admin/documents/{id}/archive",
            axum::routing::post(routes::documents::archive),
        )
        .route(
            "/api/admin/documents/{id}/reingest",
            axum::routing::post(routes::documents::reingest),
        )
        .route(
            "/api/admin/jobs",
            axum::routing::get(routes::documents::jobs),
        )
        .route(
            "/api/admin/stats",
            axum::routing::get(routes::documents::stats),
        )
        .route(
            "/api/admin/jobs/{id}",
            axum::routing::get(routes::documents::job),
        )
        .route("/api/admin/sync", axum::routing::post(routes::admin::sync))
        .nest_service("/static", ServeDir::new("static"))
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(
        &std::env::var("BLOO_BIND").unwrap_or_else(|_| "127.0.0.1:27810".into()),
    )
    .await
    .context("cannot bind Bloo port (27810) - is another process using it?")?;

    tracing::info!("listening on http://{}", listener.local_addr()?);
    axum::serve(listener, app).await?;
    Ok(())
}

async fn health(State(state): State<AppState>) -> impl IntoResponse {
    let db_ok: bool = sqlx::query_scalar("select true")
        .fetch_one(&state.pool)
        .await
        .unwrap_or(false);

    let halx_configured = state
        .cfg
        .halx_api_key
        .as_deref()
        .is_some_and(|k| !k.trim().is_empty());
    let llm_configured = state
        .cfg
        .llm_api_key
        .as_deref()
        .is_some_and(|k| !k.trim().is_empty());

    let code = if db_ok {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    (
        code,
        Json(json!({
            "service": "bloo-ai",
            "status": if db_ok { "ok" } else { "degraded" },
            "db": db_ok,
            "halx_configured": halx_configured,
            "llm_configured": llm_configured,
            // Without HALX the pipeline runs with the local decider (PLAYBOOK 8.1).
            "decider": state.cfg.decider(),
            "version": env!("CARGO_PKG_VERSION"),
        })),
    )
}

async fn version() -> impl IntoResponse {
    Json(json!({
        "service": "bloo-ai",
        "version": env!("CARGO_PKG_VERSION"),
        "commit": option_env!("BLOO_GIT_COMMIT").unwrap_or("dev"),
        "build": option_env!("BLOO_BUILD").unwrap_or("local"),
    }))
}
