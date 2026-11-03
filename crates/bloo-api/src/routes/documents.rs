use axum::{
    Json,
    extract::{Multipart, Path as UrlPath, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use bloo_core::Job;
use serde_json::{Value, json};
use sqlx::Row;
use uuid::Uuid;

use crate::AppState;

fn mime_for(kind: &str) -> &'static str {
    match kind {
        "pdf" => "application/pdf",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "html" => "text/html",
        "md" => "text/markdown",
        _ => "text/plain",
    }
}

fn kind_for(ext: &str) -> Option<&'static str> {
    match ext {
        "pdf" => Some("pdf"),
        "docx" => Some("docx"),
        "html" | "htm" => Some("html"),
        "md" | "markdown" => Some("md"),
        "txt" | "text" => Some("txt"),
        _ => None,
    }
}

/// `POST /api/admin/documents` — multipart upload. Returns 202 with one entry per file.
pub async fn upload(State(state): State<AppState>, mut multipart: Multipart) -> Response {
    let storage = bloo_ingest::Storage::new(state.cfg.data_dir.join("raw"));
    let mut results: Vec<Value> = Vec::new();
    let mut seen_any = false;

    loop {
        let field = match multipart.next_field().await {
            Ok(Some(field)) => field,
            Ok(None) => break,
            Err(err) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({ "error": format!("malformed multipart body: {err}") })),
                )
                    .into_response();
            }
        };

        seen_any = true;
        let filename = field.file_name().unwrap_or("upload").trim().to_string();
        let ext = std::path::Path::new(&filename)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();

        let Some(kind) = kind_for(&ext) else {
            return (
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                Json(json!({
                    "error": format!("unsupported file type '.{ext}' for {filename}"),
                    "supported": ["pdf", "docx", "html", "md", "txt"],
                })),
            )
                .into_response();
        };

        let bytes = match field.bytes().await {
            Ok(b) => b,
            Err(err) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({ "error": format!("cannot read {filename}: {err}") })),
                )
                    .into_response();
            }
        };

        if bytes.is_empty() {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": format!("{filename} is empty") })),
            )
                .into_response();
        }

        let stored = match storage.put(&bytes, &ext) {
            Ok(s) => s,
            Err(err) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "error": format!("cannot store {filename}: {err}") })),
                )
                    .into_response();
            }
        };

        // Content-addressed dedupe: the same bytes are only ever ingested once.
        let existing: Option<Uuid> =
            match sqlx::query_scalar("select id from documents where file_hash = $1")
                .bind(&stored.hash)
                .fetch_optional(&state.pool)
                .await
            {
                Ok(v) => v,
                Err(err) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({ "error": format!("database error: {err}") })),
                    )
                        .into_response();
                }
            };

        if let Some(id) = existing {
            results.push(json!({
                "document_id": id,
                "file": filename,
                "hash": stored.hash,
                "duplicate": true,
            }));
            continue;
        }

        let doc_id: Uuid = match sqlx::query_scalar(
            "insert into documents (title, file_path, file_hash, mime, kind, category, status)
             values ($1, $2, $3, $4, $5, 'general', 'processing')
             returning id",
        )
        .bind(&filename)
        .bind(&stored.relative)
        .bind(&stored.hash)
        .bind(mime_for(kind))
        .bind(kind)
        .fetch_one(&state.pool)
        .await
        {
            Ok(id) => id,
            Err(err) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "error": format!("cannot insert document: {err}") })),
                )
                    .into_response();
            }
        };

        let job_id = match bloo_ingest::queue::enqueue(
            &state.pool,
            "ingest",
            Some(doc_id),
            json!({ "file": filename }),
        )
        .await
        {
            Ok(id) => id,
            Err(err) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "error": format!("cannot enqueue job: {err}") })),
                )
                    .into_response();
            }
        };

        results.push(json!({
            "document_id": doc_id,
            "job_id": job_id,
            "file": filename,
            "hash": stored.hash,
            "duplicate": false,
        }));
    }

    if !seen_any {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "no file field in the request" })),
        )
            .into_response();
    }

    (StatusCode::ACCEPTED, Json(json!({ "uploaded": results }))).into_response()
}

/// `GET /api/admin/documents`
pub async fn list(State(state): State<AppState>) -> Response {
    let rows = sqlx::query(
        "select d.id, d.title, d.status, d.kind, d.category, d.page_count,
                d.created_at, d.valid_until,
                (select count(*) from chunks c where c.document_id = d.id) as chunks
           from documents d
          order by d.created_at desc
          limit 200",
    )
    .fetch_all(&state.pool)
    .await;

    match rows {
        Ok(rows) => {
            let docs: Vec<Value> = rows
                .iter()
                .map(|r| {
                    json!({
                        "id": r.get::<Uuid, _>("id"),
                        "title": r.get::<String, _>("title"),
                        "status": r.get::<String, _>("status"),
                        "kind": r.get::<String, _>("kind"),
                        "category": r.get::<String, _>("category"),
                        "page_count": r.get::<Option<i32>, _>("page_count"),
                        "chunks": r.get::<i64, _>("chunks"),
                        "created_at": r.get::<chrono::DateTime<chrono::Utc>, _>("created_at"),
                    })
                })
                .collect();
            Json(json!({ "documents": docs })).into_response()
        }
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": err.to_string() })),
        )
            .into_response(),
    }
}

/// `GET /api/admin/jobs/:id`
pub async fn job(State(state): State<AppState>, UrlPath(id): UrlPath<Uuid>) -> Response {
    let job = sqlx::query_as::<_, Job>(
        "select id, kind, document_id, state, progress, stage, attempts, error,
                created_at, started_at, finished_at
           from jobs where id = $1",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await;

    match job {
        Ok(Some(job)) => Json(job).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "job not found" })),
        )
            .into_response(),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": err.to_string() })),
        )
            .into_response(),
    }
}

/// `DELETE /api/admin/documents/:id` — removes the row, its chunks and the stored file.
pub async fn delete(State(state): State<AppState>, UrlPath(id): UrlPath<Uuid>) -> Response {
    let row = sqlx::query("select file_hash from documents where id = $1")
        .bind(id)
        .fetch_optional(&state.pool)
        .await;

    let hash = match row {
        Ok(Some(r)) => r.get::<String, _>("file_hash"),
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "document not found" })),
            )
                .into_response();
        }
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": err.to_string() })),
            )
                .into_response();
        }
    };

    // chunks cascade with the document
    if let Err(err) = sqlx::query("delete from documents where id = $1")
        .bind(id)
        .execute(&state.pool)
        .await
    {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": err.to_string() })),
        )
            .into_response();
    }

    let storage = bloo_ingest::Storage::new(state.cfg.data_dir.join("raw"));
    let files_removed = storage.delete(&hash).unwrap_or(0);

    Json(json!({ "deleted": id, "files_removed": files_removed })).into_response()
}

/// `POST /api/admin/documents/:id/publish`
pub async fn publish(State(state): State<AppState>, UrlPath(id): UrlPath<Uuid>) -> Response {
    let updated = sqlx::query(
        "update documents set status = 'published', updated_at = now()
          where id = $1 and status in ('ready', 'published')
          returning id",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await;

    match updated {
        Ok(Some(_)) => Json(json!({ "published": id })).into_response(),
        Ok(None) => {
            let status: Option<String> =
                sqlx::query_scalar("select status from documents where id = $1")
                    .bind(id)
                    .fetch_optional(&state.pool)
                    .await
                    .unwrap_or(None);

            match status {
                None => (
                    StatusCode::NOT_FOUND,
                    Json(json!({ "error": "document not found" })),
                )
                    .into_response(),
                Some(status) => (
                    StatusCode::CONFLICT,
                    Json(json!({
                        "error": format!("document is '{status}', not 'ready'"),
                        "status": status,
                    })),
                )
                    .into_response(),
            }
        }
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": err.to_string() })),
        )
            .into_response(),
    }
}
