use axum::{
    Json,
    extract::{Multipart, Path as UrlPath, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use bloo_core::Job;
use serde::Deserialize;
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

fn db_error(err: impl std::fmt::Display) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "error": err.to_string() })),
    )
        .into_response()
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
            Err(err) => return db_error(format!("cannot store {filename}: {err}")),
        };

        // Content-addressed dedupe among live documents only; an archived version may share
        // the hash with a re-uploaded file.
        let existing: Option<Uuid> = match sqlx::query_scalar(
            "select id from documents where file_hash = $1 and status <> 'archived'",
        )
        .bind(&stored.hash)
        .fetch_optional(&state.pool)
        .await
        {
            Ok(v) => v,
            Err(err) => return db_error(err),
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
            Err(err) => return db_error(err),
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
            Err(err) => return db_error(err),
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

#[derive(Deserialize, Default)]
pub struct ListParams {
    pub q: Option<String>,
    pub status: Option<String>,
    pub kind: Option<String>,
    pub category: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

/// `GET /api/admin/documents` — filtered, paginated.
pub async fn list(State(state): State<AppState>, Query(params): Query<ListParams>) -> Response {
    let q = params.q.unwrap_or_default();
    let pattern = format!("%{}%", q.trim());
    let status = params.status.unwrap_or_default();
    let kind = params.kind.unwrap_or_default();
    let category = params.category.unwrap_or_default();
    let limit = params.limit.unwrap_or(50).clamp(1, 200);
    let offset = params.offset.unwrap_or(0).max(0);

    // Static SQL: only values are bound. Building the WHERE clause with format! would trip
    // sqlx 0.9's dynamic-SQL audit for no benefit.
    let total: i64 = match sqlx::query_scalar(
        "select count(*) from documents d
          where ($1 = '' or d.title ilike $2)
            and ($3 = '' or d.status = $3)
            and ($4 = '' or d.kind = $4)
            and ($5 = '' or d.category = $5)",
    )
    .bind(&q)
    .bind(&pattern)
    .bind(&status)
    .bind(&kind)
    .bind(&category)
    .fetch_one(&state.pool)
    .await
    {
        Ok(v) => v,
        Err(err) => return db_error(err),
    };

    let rows = match sqlx::query(
        "select d.id, d.title, d.status, d.kind, d.category, d.page_count,
                d.created_at, d.updated_at, d.error, d.source_path, d.version,
                (select count(*) from chunks c where c.document_id = d.id) as chunks
           from documents d
          where ($1 = '' or d.title ilike $2)
            and ($3 = '' or d.status = $3)
            and ($4 = '' or d.kind = $4)
            and ($5 = '' or d.category = $5)
          order by d.created_at desc
          limit $6 offset $7",
    )
    .bind(&q)
    .bind(&pattern)
    .bind(&status)
    .bind(&kind)
    .bind(&category)
    .bind(limit)
    .bind(offset)
    .fetch_all(&state.pool)
    .await
    {
        Ok(rows) => rows,
        Err(err) => return db_error(err),
    };

    let documents: Vec<Value> = rows
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
                "error": r.get::<Option<String>, _>("error"),
                "source_path": r.get::<Option<String>, _>("source_path"),
                "version": r.get::<i32, _>("version"),
                "created_at": r.get::<chrono::DateTime<chrono::Utc>, _>("created_at"),
                "updated_at": r.get::<chrono::DateTime<chrono::Utc>, _>("updated_at"),
            })
        })
        .collect();

    Json(json!({ "documents": documents, "total": total, "limit": limit, "offset": offset }))
        .into_response()
}

/// `GET /api/admin/documents/{id}` — metadata + chunk count + job history.
pub async fn detail(State(state): State<AppState>, UrlPath(id): UrlPath<Uuid>) -> Response {
    let row = match sqlx::query(
        "select d.id, d.title, d.status, d.kind, d.category, d.page_count,
                d.created_at, d.updated_at, d.error, d.source_path, d.version,
                d.valid_until, d.last_verified_at, d.file_hash,
                (select count(*) from chunks c where c.document_id = d.id) as chunks
           from documents d where d.id = $1",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await
    {
        Ok(Some(row)) => row,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "document not found" })),
            )
                .into_response();
        }
        Err(err) => return db_error(err),
    };

    let jobs = match sqlx::query_as::<_, Job>(
        "select id, kind, document_id, state, progress, stage, attempts, error,
                created_at, started_at, finished_at
           from jobs where document_id = $1
          order by created_at desc limit 20",
    )
    .bind(id)
    .fetch_all(&state.pool)
    .await
    {
        Ok(jobs) => jobs,
        Err(err) => return db_error(err),
    };

    let document = json!({
        "id": row.get::<Uuid, _>("id"),
        "title": row.get::<String, _>("title"),
        "status": row.get::<String, _>("status"),
        "kind": row.get::<String, _>("kind"),
        "category": row.get::<String, _>("category"),
        "page_count": row.get::<Option<i32>, _>("page_count"),
        "chunks": row.get::<i64, _>("chunks"),
        "error": row.get::<Option<String>, _>("error"),
        "source_path": row.get::<Option<String>, _>("source_path"),
        "version": row.get::<i32, _>("version"),
        "file_hash": row.get::<String, _>("file_hash"),
        "valid_until": row.get::<Option<chrono::NaiveDate>, _>("valid_until").map(|d| d.to_string()),
        "last_verified_at": row.get::<Option<chrono::NaiveDate>, _>("last_verified_at").map(|d| d.to_string()),
        "created_at": row.get::<chrono::DateTime<chrono::Utc>, _>("created_at"),
        "updated_at": row.get::<chrono::DateTime<chrono::Utc>, _>("updated_at"),
    });

    Json(json!({ "document": document, "jobs": jobs })).into_response()
}

#[derive(Deserialize, Default)]
pub struct ChunkParams {
    pub offset: Option<i64>,
    pub limit: Option<i64>,
}

/// `GET /api/admin/documents/{id}/chunks` — paginated chunk inspector payload.
pub async fn chunks(
    State(state): State<AppState>,
    UrlPath(id): UrlPath<Uuid>,
    Query(params): Query<ChunkParams>,
) -> Response {
    let limit = params.limit.unwrap_or(20).clamp(1, 100);
    let offset = params.offset.unwrap_or(0).max(0);

    let total: i64 = match sqlx::query_scalar("select count(*) from chunks where document_id = $1")
        .bind(id)
        .fetch_one(&state.pool)
        .await
    {
        Ok(v) => v,
        Err(err) => return db_error(err),
    };

    let rows = match sqlx::query(
        "select id, ord, text, section_path, page_from, page_to, chunk_type, token_count
           from chunks where document_id = $1 order by ord limit $2 offset $3",
    )
    .bind(id)
    .bind(limit)
    .bind(offset)
    .fetch_all(&state.pool)
    .await
    {
        Ok(rows) => rows,
        Err(err) => return db_error(err),
    };

    let chunks: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "id": r.get::<Uuid, _>("id"),
                "ord": r.get::<i32, _>("ord"),
                "text": r.get::<String, _>("text"),
                "section_path": r.get::<String, _>("section_path"),
                "page_from": r.get::<Option<i32>, _>("page_from"),
                "page_to": r.get::<Option<i32>, _>("page_to"),
                "chunk_type": r.get::<String, _>("chunk_type"),
                "token_count": r.get::<Option<i32>, _>("token_count"),
            })
        })
        .collect();

    Json(json!({ "chunks": chunks, "total": total, "limit": limit, "offset": offset }))
        .into_response()
}

#[derive(Deserialize, Default)]
pub struct PatchBody {
    pub title: Option<String>,
    pub category: Option<String>,
    #[serde(default, deserialize_with = "double_option")]
    pub valid_until: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub last_verified_at: Option<Option<String>>,
}

/// Distinguish "field absent" (None) from "field set to null" (Some(None)).
fn double_option<'de, D>(de: D) -> Result<Option<Option<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<String>::deserialize(de).map(Some)
}

/// `PATCH /api/admin/documents/{id}` — title, category and dates.
pub async fn patch(
    State(state): State<AppState>,
    UrlPath(id): UrlPath<Uuid>,
    Json(body): Json<PatchBody>,
) -> Response {
    let set_valid = body.valid_until.is_some();
    let valid = body
        .valid_until
        .as_ref()
        .and_then(|v| v.as_deref())
        .filter(|s| !s.trim().is_empty());

    let set_verified = body.last_verified_at.is_some();
    let verified = body
        .last_verified_at
        .as_ref()
        .and_then(|v| v.as_deref())
        .filter(|s| !s.trim().is_empty());

    let updated = sqlx::query(
        "update documents set
             title = coalesce($2, title),
             category = coalesce($3, category),
             valid_until = case when $4 then $5::date else valid_until end,
             last_verified_at = case when $6 then $7::date else last_verified_at end,
             updated_at = now()
           where id = $1
           returning id",
    )
    .bind(id)
    .bind(body.title.as_deref().filter(|s| !s.trim().is_empty()))
    .bind(body.category.as_deref().filter(|s| !s.trim().is_empty()))
    .bind(set_valid)
    .bind(valid)
    .bind(set_verified)
    .bind(verified)
    .fetch_optional(&state.pool)
    .await;

    match updated {
        Ok(Some(_)) => Json(json!({ "updated": true })).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "document not found" })),
        )
            .into_response(),
        Err(err) => db_error(err),
    }
}

async fn current_status(state: &AppState, id: Uuid) -> Option<String> {
    sqlx::query_scalar("select status from documents where id = $1")
        .bind(id)
        .fetch_optional(&state.pool)
        .await
        .ok()
        .flatten()
}

fn conflict(current: &str, wanted: &str) -> Response {
    (
        StatusCode::CONFLICT,
        Json(json!({
            "error": format!("document is '{current}', expected {wanted}"),
            "status": current,
        })),
    )
        .into_response()
}

/// `POST /api/admin/documents/{id}/publish`
pub async fn publish(State(state): State<AppState>, UrlPath(id): UrlPath<Uuid>) -> Response {
    match sqlx::query(
        "update documents set status = 'published', updated_at = now()
          where id = $1 and status in ('ready', 'published') returning id",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await
    {
        Ok(Some(_)) => Json(json!({ "published": id })).into_response(),
        Ok(None) => match current_status(&state, id).await {
            None => (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "document not found" })),
            )
                .into_response(),
            Some(status) => conflict(&status, "'ready'"),
        },
        Err(err) => db_error(err),
    }
}

/// `POST /api/admin/documents/{id}/unpublish` — published → ready.
pub async fn unpublish(State(state): State<AppState>, UrlPath(id): UrlPath<Uuid>) -> Response {
    match sqlx::query(
        "update documents set status = 'ready', updated_at = now()
          where id = $1 and status = 'published' returning id",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await
    {
        Ok(Some(_)) => Json(json!({ "unpublished": id })).into_response(),
        Ok(None) => match current_status(&state, id).await {
            None => (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "document not found" })),
            )
                .into_response(),
            Some(status) => conflict(&status, "'published'"),
        },
        Err(err) => db_error(err),
    }
}

/// `POST /api/admin/documents/{id}/archive`
pub async fn archive(State(state): State<AppState>, UrlPath(id): UrlPath<Uuid>) -> Response {
    match sqlx::query(
        "update documents set status = 'archived', updated_at = now()
          where id = $1 and status in ('ready', 'published', 'failed') returning id",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await
    {
        Ok(Some(_)) => Json(json!({ "archived": id })).into_response(),
        Ok(None) => match current_status(&state, id).await {
            None => (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "document not found" })),
            )
                .into_response(),
            Some(status) => conflict(&status, "'ready' or 'published' or 'failed'"),
        },
        Err(err) => db_error(err),
    }
}

/// `POST /api/admin/documents/{id}/reingest` — re-parse, re-chunk, re-embed.
pub async fn reingest(State(state): State<AppState>, UrlPath(id): UrlPath<Uuid>) -> Response {
    let active: i64 = match sqlx::query_scalar(
        "select count(*) from jobs where document_id = $1 and state in ('queued', 'running')",
    )
    .bind(id)
    .fetch_one(&state.pool)
    .await
    {
        Ok(v) => v,
        Err(err) => return db_error(err),
    };

    if active > 0 {
        return (
            StatusCode::CONFLICT,
            Json(json!({ "error": "a job is already running for this document" })),
        )
            .into_response();
    }

    match sqlx::query(
        "update documents set status = 'processing', error = null, updated_at = now()
          where id = $1 and status <> 'archived' returning id",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await
    {
        Ok(Some(_)) => {}
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "document not found or archived" })),
            )
                .into_response();
        }
        Err(err) => return db_error(err),
    }

    match bloo_ingest::queue::enqueue(
        &state.pool,
        "ingest",
        Some(id),
        json!({ "reason": "reingest" }),
    )
    .await
    {
        Ok(job_id) => (StatusCode::ACCEPTED, Json(json!({ "job_id": job_id }))).into_response(),
        Err(err) => db_error(err),
    }
}

/// `GET /api/admin/jobs/{id}`
pub async fn job(State(state): State<AppState>, UrlPath(id): UrlPath<Uuid>) -> Response {
    match sqlx::query_as::<_, Job>(
        "select id, kind, document_id, state, progress, stage, attempts, error,
                created_at, started_at, finished_at
           from jobs where id = $1",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await
    {
        Ok(Some(job)) => Json(job).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "job not found" })),
        )
            .into_response(),
        Err(err) => db_error(err),
    }
}

#[derive(Deserialize, Default)]
pub struct JobParams {
    pub state: Option<String>,
    pub limit: Option<i64>,
}

/// `GET /api/admin/jobs` — recent jobs, optionally filtered by state.
pub async fn jobs(State(state): State<AppState>, Query(params): Query<JobParams>) -> Response {
    let filter = params.state.unwrap_or_default();
    let limit = params.limit.unwrap_or(20).clamp(1, 100);

    match sqlx::query_as::<_, Job>(
        "select id, kind, document_id, state, progress, stage, attempts, error,
                created_at, started_at, finished_at
           from jobs
          where ($1 = '' or state = $1)
          order by created_at desc limit $2",
    )
    .bind(&filter)
    .bind(limit)
    .fetch_all(&state.pool)
    .await
    {
        Ok(jobs) => Json(json!({ "jobs": jobs })).into_response(),
        Err(err) => db_error(err),
    }
}

/// `GET /api/admin/stats`
pub async fn stats(State(state): State<AppState>) -> Response {
    let row = sqlx::query(
        "select
            (select count(*) from documents) as documents,
            (select count(*) from documents where status = 'published') as published,
            (select count(*) from documents where status = 'ready') as ready,
            (select count(*) from documents where status = 'failed') as failed,
            (select count(*) from documents where status = 'processing') as processing,
            (select count(*) from chunks) as chunks,
            (select count(*) from messages
              where role = 'assistant' and created_at::date = current_date) as questions_today,
            (select count(*) from feedback where verdict = -1) as thumbs_down",
    )
    .fetch_one(&state.pool)
    .await;

    match row {
        Ok(row) => Json(json!({
            "documents": row.get::<i64, _>("documents"),
            "published": row.get::<i64, _>("published"),
            "ready": row.get::<i64, _>("ready"),
            "failed": row.get::<i64, _>("failed"),
            "processing": row.get::<i64, _>("processing"),
            "chunks": row.get::<i64, _>("chunks"),
            "questions_today": row.get::<i64, _>("questions_today"),
            "thumbs_down": row.get::<i64, _>("thumbs_down"),
        }))
        .into_response(),
        Err(err) => db_error(err),
    }
}

/// `DELETE /api/admin/documents/{id}` — refuses while a job is in flight.
pub async fn delete(State(state): State<AppState>, UrlPath(id): UrlPath<Uuid>) -> Response {
    let active: i64 = match sqlx::query_scalar(
        "select count(*) from jobs where document_id = $1 and state in ('queued', 'running')",
    )
    .bind(id)
    .fetch_one(&state.pool)
    .await
    {
        Ok(v) => v,
        Err(err) => return db_error(err),
    };

    if active > 0 {
        return (
            StatusCode::CONFLICT,
            Json(
                json!({ "error": "a job is running for this document; wait or archive it first" }),
            ),
        )
            .into_response();
    }

    let row = match sqlx::query("select file_hash from documents where id = $1")
        .bind(id)
        .fetch_optional(&state.pool)
        .await
    {
        Ok(Some(row)) => row,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "document not found" })),
            )
                .into_response();
        }
        Err(err) => return db_error(err),
    };
    let hash = row.get::<String, _>("file_hash");

    if let Err(err) = sqlx::query("delete from documents where id = $1")
        .bind(id)
        .execute(&state.pool)
        .await
    {
        return db_error(err);
    }

    let storage = bloo_ingest::Storage::new(state.cfg.data_dir.join("raw"));
    let files_removed = storage.delete(&hash).unwrap_or(0);

    Json(json!({ "deleted": id, "files_removed": files_removed })).into_response()
}
