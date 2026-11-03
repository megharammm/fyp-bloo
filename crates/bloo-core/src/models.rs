use chrono::{DateTime, NaiveDate, Utc};
use serde::Serialize;
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct Document {
    pub id: Uuid,
    pub source_id: Option<Uuid>,
    pub title: String,
    pub canonical_url: Option<String>,
    pub file_path: Option<String>,
    pub file_hash: String,
    pub mime: String,
    pub kind: String,
    pub category: String,
    pub status: String,
    pub version: i32,
    pub page_count: Option<i32>,
    pub valid_until: Option<NaiveDate>,
    pub last_verified_at: Option<NaiveDate>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct Chunk {
    pub id: Uuid,
    pub document_id: Uuid,
    pub ord: i32,
    pub text: String,
    pub section_path: String,
    pub page_from: Option<i32>,
    pub page_to: Option<i32>,
    pub chunk_type: String,
    pub token_count: Option<i32>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct Job {
    pub id: Uuid,
    pub kind: String,
    pub document_id: Option<Uuid>,
    pub state: String,
    pub progress: i32,
    pub stage: Option<String>,
    pub attempts: i32,
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
}

/// A citation rendered to the user. NOTE: `kind = "pdf"` deliberately carries
/// no link — students see the quoted text, never the source file.
#[derive(Debug, Clone, Serialize)]
pub struct Citation {
    pub title: String,
    pub section_path: String,
    pub kind: String,
    pub url: Option<String>,
    pub page_label: Option<String>,
    pub quote: String,
    pub score: f32,
}
