//! Hybrid retrieval against Postgres.
//!
//! Dense (pgvector cosine) and lexical (`tsvector`) candidates are fused with Reciprocal Rank
//! Fusion in one SQL statement. RRF needs no score normalisation, which is why it survives
//! mixing cosine similarities with `ts_rank_cd` values.

use anyhow::{Context, Result};
use bloo_core::PgPool;
use serde::Serialize;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize)]
pub struct RetrievedChunk {
    pub id: Uuid,
    pub document_id: Uuid,
    pub text: String,
    pub section_path: String,
    pub page_from: Option<i32>,
    pub page_to: Option<i32>,
    pub title: String,
    pub kind: String,
    pub canonical_url: Option<String>,
    /// Fused rank score (higher is better).
    pub rrf: f32,
    /// Dense cosine similarity for the best dense hit (0 when only lexical matched).
    pub similarity: f32,
}

pub fn vector_literal(v: &[f32]) -> String {
    let mut s = String::with_capacity(v.len() * 10 + 2);
    s.push('[');
    for (i, x) in v.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        let x = if x.is_finite() { *x } else { 0.0 };
        s.push_str(&format!("{x}"));
    }
    s.push(']');
    s
}

const HYBRID_SQL: &str = r#"
with q as (
    select $1::vector(384) as dense, $2::text as lex
),
dense as (
    select c.id,
           1 - (c.embedding <=> q.dense) as sim,
           row_number() over (order by c.embedding <=> q.dense) as rank
      from searchable_chunks c, q
     order by c.embedding <=> q.dense
     limit 40
),
lex as (
    select c.id,
           row_number() over (
               order by ts_rank_cd(c.tsv, websearch_to_tsquery('simple', q.lex)) desc
           ) as rank
      from searchable_chunks c, q
     where c.tsv @@ websearch_to_tsquery('simple', q.lex)
     order by ts_rank_cd(c.tsv, websearch_to_tsquery('simple', q.lex)) desc
     limit 40
),
fused as (
    select id, sum(1.0 / (60 + rank)) as rrf
      from (select id, rank from dense union all select id, rank from lex) u
     group by id
)
select c.id, c.document_id, c.text, c.section_path, c.page_from, c.page_to,
       c.title, c.kind, c.canonical_url,
       f.rrf::float8 as rrf,
       coalesce(d.sim, 0)::float8 as sim
  from fused f
  join searchable_chunks c on c.id = f.id
  left join dense d on d.id = f.id
 order by f.rrf desc
 limit $3
"#;

/// Run hybrid retrieval. Only `published`, non-expired chunks are visible.
pub async fn hybrid_search(
    pool: &PgPool,
    query: &str,
    embedding: &[f32],
    limit: i64,
) -> Result<Vec<RetrievedChunk>> {
    let literal = vector_literal(embedding);
    let records = sqlx::query(HYBRID_SQL)
        .bind(literal)
        .bind(query)
        .bind(limit)
        .fetch_all(pool)
        .await
        .context("hybrid retrieval failed")?;

    let mut out = Vec::with_capacity(records.len());
    for row in records {
        use sqlx::Row;
        out.push(RetrievedChunk {
            id: row.get("id"),
            document_id: row.get("document_id"),
            text: row.get("text"),
            section_path: row.get("section_path"),
            page_from: row.get("page_from"),
            page_to: row.get("page_to"),
            title: row.get("title"),
            kind: row.get("kind"),
            canonical_url: row.get("canonical_url"),
            rrf: row.get::<f64, _>("rrf") as f32,
            similarity: row.get::<f64, _>("sim") as f32,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::vector_literal;

    #[test]
    fn vector_literal_is_pgvector_text() {
        assert_eq!(vector_literal(&[0.5, -1.0]), "[0.5,-1]");
    }
}
