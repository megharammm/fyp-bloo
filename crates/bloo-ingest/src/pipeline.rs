//! The ingestion job itself: parse → chunk → embed → store.

use crate::queue::set_progress;
use crate::{chunk_blocks, ChunkOptions};
use anyhow::{bail, Context, Result};
use bloo_core::{Config, Job, PgPool};
use bloo_parse::Format;
use bloo_rag::Embedder;
use uuid::Uuid;

pub async fn process(
    pool: &PgPool,
    cfg: &Config,
    embedder: &dyn Embedder,
    job: &Job,
) -> Result<()> {
    let doc_id = job.document_id.context("job has no document_id")?;

    let (relative, kind): (String, String) =
        sqlx::query_as("select coalesce(file_path, ''), kind from documents where id = $1")
            .bind(doc_id)
            .fetch_one(pool)
            .await?;

    if relative.is_empty() {
        bail!("document {doc_id} has no stored file");
    }

    set_progress(pool, job.id, 5, "reading").await?;
    let path = cfg.data_dir.join(&relative);
    let bytes = tokio::fs::read(&path)
        .await
        .with_context(|| format!("cannot read {}", path.display()))?;

    set_progress(pool, job.id, 15, "parsing").await?;
    let format =
        Format::from_hint(&kind).with_context(|| format!("unsupported document kind {kind:?}"))?;
    let parsed = bloo_parse::parse(&bytes, format)?;

    if parsed.blocks.is_empty() {
        bail!("no extractable text - the file is probably a scanned image (OCR not enabled)");
    }

    set_progress(pool, job.id, 40, "chunking").await?;
    let chunks = chunk_blocks(&parsed.blocks, &ChunkOptions::default());
    if chunks.is_empty() {
        bail!("parser produced blocks but the chunker produced nothing");
    }

    set_progress(pool, job.id, 55, "embedding").await?;
    let texts: Vec<String> = chunks.iter().map(|c| c.text.clone()).collect();
    let vectors = embedder.embed_documents(&texts).await?;
    if vectors.len() != chunks.len() {
        bail!(
            "embedder returned {} vectors for {} chunks",
            vectors.len(),
            chunks.len()
        );
    }
    for v in &vectors {
        if v.len() != cfg.embed_dim {
            bail!(
                "embedding has {} dimensions, schema expects {}",
                v.len(),
                cfg.embed_dim
            );
        }
    }

    set_progress(pool, job.id, 85, "writing").await?;
    write_chunks(
        pool,
        doc_id,
        &chunks,
        &vectors,
        parsed.page_count,
        parsed.title.as_deref(),
    )
    .await?;

    set_progress(pool, job.id, 100, "done").await?;
    Ok(())
}

async fn write_chunks(
    pool: &PgPool,
    doc_id: Uuid,
    chunks: &[crate::Chunk],
    vectors: &[Vec<f32>],
    page_count: u32,
    title: Option<&str>,
) -> Result<()> {
    let mut tx = pool.begin().await?;

    sqlx::query("delete from chunks where document_id = $1")
        .bind(doc_id)
        .execute(&mut *tx)
        .await?;

    for (ord, (chunk, vector)) in chunks.iter().zip(vectors.iter()).enumerate() {
        let literal = vector_literal(vector);
        sqlx::query(
            "insert into chunks
                (document_id, ord, text, section_path, page_from, page_to,
                 chunk_type, token_count, embedding)
             values ($1, $2, $3, $4, $5, $6, $7, $8, $9::vector)",
        )
        .bind(doc_id)
        .bind(ord as i32)
        .bind(&chunk.text)
        .bind(&chunk.section_path)
        .bind(chunk.page_from)
        .bind(chunk.page_to)
        .bind(&chunk.chunk_type)
        .bind(chunk.token_count)
        .bind(literal)
        .execute(&mut *tx)
        .await?;
    }

    sqlx::query(
        "update documents
            set status = 'ready',
                page_count = $2,
                title = coalesce($3, title),
                error = null,
                updated_at = now()
          where id = $1",
    )
    .bind(doc_id)
    .bind(page_count as i32)
    .bind(title)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}

/// pgvector accepts the text form `[0.1,0.2,...]`.
pub fn vector_literal(v: &[f32]) -> String {
    let mut s = String::with_capacity(v.len() * 10 + 2);
    s.push('[');
    for (i, x) in v.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        // A non-finite value would make the whole insert fail with a cryptic error.
        let x = if x.is_finite() { *x } else { 0.0 };
        s.push_str(&format!("{x}"));
    }
    s.push(']');
    s
}

#[cfg(test)]
mod tests {
    use super::vector_literal;

    #[test]
    fn vector_literal_format() {
        assert_eq!(vector_literal(&[0.5, -1.0, 2.0]), "[0.5,-1,2]");
    }

    #[test]
    fn non_finite_values_are_zeroed() {
        let s = vector_literal(&[f32::NAN, f32::INFINITY]);
        assert_eq!(s, "[0,0]");
    }
}
