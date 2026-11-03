use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
};
use bloo_rag::retrieve::{citation_for, pick_quote};
use bloo_rag::{LlmClient, ThxClient};
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::json;
use std::convert::Infallible;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use crate::AppState;

#[derive(Deserialize)]
pub struct SearchParams {
    pub q: String,
}

/// `GET /api/search?q=` — retrieval only, for debugging and evaluation.
pub async fn search(State(state): State<AppState>, Query(params): Query<SearchParams>) -> Response {
    if params.q.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "q is required" })),
        )
            .into_response();
    }

    let embedding = match state.embedder.embed_query(&params.q).await {
        Ok(e) => e,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": format!("embedding failed: {err}") })),
            )
                .into_response();
        }
    };

    match bloo_rag::store::hybrid_search(&state.pool, &params.q, &embedding, 10).await {
        Ok(hits) => Json(json!({
            "query": params.q,
            "hits": hits.iter().map(|h| json!({
                "id": h.id,
                "document_id": h.document_id,
                "title": h.title,
                "section_path": h.section_path,
                "page": h.page_from,
                "rrf": h.rrf,
                "similarity": h.similarity,
                "text": h.text.chars().take(240).collect::<String>(),
            })).collect::<Vec<_>>()
        }))
        .into_response(),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": err.to_string() })),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct AskRequest {
    pub question: String,
    #[serde(default)]
    pub stream: bool,
}

#[derive(Deserialize)]
pub struct FeedbackRequest {
    pub message_id: uuid::Uuid,
    /// 1 = thumbs up, -1 = thumbs down.
    pub verdict: i16,
    pub note: Option<String>,
}

/// `POST /api/feedback` — store a thumbs verdict against an assistant message.
pub async fn feedback(State(state): State<AppState>, Json(req): Json<FeedbackRequest>) -> Response {
    if req.verdict != 1 && req.verdict != -1 {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "verdict must be 1 or -1" })),
        )
            .into_response();
    }

    match sqlx::query("insert into feedback (message_id, verdict, note) values ($1, $2, $3)")
        .bind(req.message_id)
        .bind(req.verdict)
        .bind(req.note.as_deref())
        .execute(&state.pool)
        .await
    {
        Ok(_) => Json(json!({ "ok": true })).into_response(),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": err.to_string() })),
        )
            .into_response(),
    }
}

/// Persist the exchange and its citations. Returns the assistant message id.
async fn persist_exchange(
    state: &AppState,
    question: &str,
    answer: &str,
    citations: &[bloo_core::Citation],
    route: Option<&str>,
    latency_ms: u64,
    model: &str,
) -> Option<uuid::Uuid> {
    let title: String = question.chars().take(80).collect();

    let conversation: uuid::Uuid =
        sqlx::query_scalar("insert into conversations (title) values ($1) returning id")
            .bind(title)
            .fetch_one(&state.pool)
            .await
            .ok()?;

    sqlx::query("insert into messages (conversation_id, role, content) values ($1, 'user', $2)")
        .bind(conversation)
        .bind(question)
        .execute(&state.pool)
        .await
        .ok()?;

    let message: uuid::Uuid = sqlx::query_scalar(
        "insert into messages (conversation_id, role, content, route, latency_ms, model)
         values ($1, 'assistant', $2, $3, $4, $5) returning id",
    )
    .bind(conversation)
    .bind(answer)
    .bind(route)
    .bind(latency_ms as i32)
    .bind(model)
    .fetch_one(&state.pool)
    .await
    .ok()?;

    for (ord, citation) in citations.iter().enumerate() {
        let _ = sqlx::query(
            "insert into citations
                (message_id, title, section_path, kind, url, page_label, quote, score, ord)
             values ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        )
        .bind(message)
        .bind(&citation.title)
        .bind(&citation.section_path)
        .bind(&citation.kind)
        .bind(&citation.url)
        .bind(&citation.page_label)
        .bind(&citation.quote)
        .bind(citation.score)
        .bind(ord as i32)
        .execute(&state.pool)
        .await;
    }

    Some(message)
}

/// `POST /api/chat` — JSON by default, SSE when `"stream": true`.
pub async fn chat(State(state): State<AppState>, Json(req): Json<AskRequest>) -> Response {
    if req.question.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "question is required" })),
        )
            .into_response();
    }

    if req.stream {
        stream_chat(state, req).await
    } else {
        json_chat(state, req).await
    }
}

async fn json_chat(state: AppState, req: AskRequest) -> Response {
    let llm = match LlmClient::from_config(&state.cfg) {
        Ok(c) => c,
        Err(err) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "error": err.to_string() })),
            )
                .into_response();
        }
    };
    let thx = ThxClient::new(&state.cfg);

    match bloo_rag::answer(
        &state.pool,
        &state.cfg,
        state.embedder.as_ref(),
        &thx,
        &llm,
        &req.question,
    )
    .await
    {
        Ok(out) => {
            let message_id = persist_exchange(
                &state,
                &req.question,
                &out.answer,
                &out.citations,
                out.route.as_deref(),
                out.latency_ms,
                &out.model,
            )
            .await;

            Json(json!({
                "answer": out.answer,
                "refused": out.refused,
                "citations": out.citations,
                "message_id": message_id,
                "meta": {
                    "route": out.route,
                    "topic": out.topic,
                    "retrieved": out.retrieved,
                    "best_similarity": out.best_similarity,
                    "latency_ms": out.latency_ms,
                    "model": out.model,
                }
            }))
            .into_response()
        }
        Err(err) => (
            StatusCode::BAD_GATEWAY,
            Json(json!({ "error": err.to_string() })),
        )
            .into_response(),
    }
}

async fn stream_chat(state: AppState, req: AskRequest) -> Response {
    let (tx, rx) = mpsc::channel::<Result<Event, Infallible>>(64);

    tokio::spawn(async move {
        let started = std::time::Instant::now();
        let thx = ThxClient::new(&state.cfg);

        let prepared = match bloo_rag::retrieve::prepare(
            &state.pool,
            &state.cfg,
            state.embedder.as_ref(),
            &thx,
            &req.question,
        )
        .await
        {
            Ok(p) => p,
            Err(err) => {
                let _ = tx
                    .send(Ok(Event::default()
                        .event("error")
                        .data(json!({ "error": err.to_string() }).to_string())))
                    .await;
                return;
            }
        };

        if let Some(refusal) = prepared.refusal {
            let message_id = persist_exchange(
                &state,
                &req.question,
                &refusal,
                &[],
                None,
                started.elapsed().as_millis() as u64,
                "refusal",
            )
            .await;

            let _ = tx
                .send(Ok(Event::default()
                    .event("token")
                    .data(json!({ "t": refusal }).to_string())))
                .await;
            let _ = tx
                .send(Ok(Event::default()
                    .event("citations")
                    .data(json!({ "citations": [] }).to_string())))
                .await;
            let _ = tx
                .send(Ok(Event::default().event("done").data(
                    json!({
                        "latency_ms": started.elapsed().as_millis() as u64,
                        "refused": true,
                        "message_id": message_id,
                    })
                    .to_string(),
                )))
                .await;
            return;
        }

        let llm = match LlmClient::from_config(&state.cfg) {
            Ok(c) => c,
            Err(err) => {
                let _ = tx
                    .send(Ok(Event::default()
                        .event("error")
                        .data(json!({ "error": err.to_string() }).to_string())))
                    .await;
                return;
            }
        };

        let user = bloo_rag::prompt::user_prompt(&req.question, &prepared.chunks);
        let mut stream = llm.stream(bloo_rag::prompt::system_prompt().to_string(), user);

        let mut full = String::new();
        let mut first_token_ms: Option<u64> = None;
        while let Some(item) = stream.next().await {
            match item {
                Ok(token) => {
                    if first_token_ms.is_none() {
                        first_token_ms = Some(started.elapsed().as_millis() as u64);
                    }
                    full.push_str(&token);
                    if tx
                        .send(Ok(Event::default()
                            .event("token")
                            .data(json!({ "t": token }).to_string())))
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
                Err(err) => {
                    let _ = tx
                        .send(Ok(Event::default()
                            .event("error")
                            .data(json!({ "error": err.to_string() }).to_string())))
                        .await;
                    return;
                }
            }
        }

        // Citations are computed after the stream so the first token is not delayed.
        let mut indices = bloo_rag::retrieve::used_indices(&full);
        if indices.is_empty() {
            indices = (0..prepared.chunks.len().min(2)).collect();
        }
        indices.retain(|i| *i < prepared.chunks.len());
        indices.truncate(4);

        let citations: Vec<_> = indices
            .iter()
            .map(|i| {
                let c = &prepared.chunks[*i];
                citation_for(c, pick_quote(&c.text, &req.question))
            })
            .collect();

        let _ = tx
            .send(Ok(Event::default()
                .event("citations")
                .data(json!({ "citations": citations }).to_string())))
            .await;

        let message_id = persist_exchange(
            &state,
            &req.question,
            &full,
            &citations,
            prepared.route.as_deref(),
            started.elapsed().as_millis() as u64,
            llm.model(),
        )
        .await;

        let _ = tx
            .send(Ok(Event::default().event("done").data(
                json!({
                    "latency_ms": started.elapsed().as_millis() as u64,
                    "first_token_ms": first_token_ms,
                    "refused": false,
                    "message_id": message_id,
                })
                .to_string(),
            )))
            .await;
    });

    Sse::new(ReceiverStream::new(rx))
        .keep_alive(KeepAlive::default())
        .into_response()
}
