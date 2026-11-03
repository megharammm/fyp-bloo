//! Postgres job queue. `SELECT … FOR UPDATE SKIP LOCKED`, no Redis.
//!
//! Single-worker assumption: on startup the worker requeues every `running` job, because any
//! such row was orphaned by a crash. With two concurrent workers that would steal in-flight
//! work, which this project deliberately does not do.

use anyhow::Result;
use bloo_core::{Job, PgPool};
use serde_json::Value;
use uuid::Uuid;

pub const MAX_ATTEMPTS: i32 = 3;

pub async fn enqueue(
    pool: &PgPool,
    kind: &str,
    document_id: Option<Uuid>,
    payload: Value,
) -> Result<Uuid> {
    let id: Uuid = sqlx::query_scalar(
        "insert into jobs (kind, document_id, payload) values ($1, $2, $3) returning id",
    )
    .bind(kind)
    .bind(document_id)
    .bind(payload)
    .fetch_one(pool)
    .await?;
    Ok(id)
}

pub async fn claim(pool: &PgPool) -> Result<Option<Job>> {
    let job = sqlx::query_as::<_, Job>(
        r#"
        update jobs
           set state = 'running',
               started_at = now(),
               attempts = attempts + 1,
               stage = 'claimed'
         where id = (
               select id from jobs
                where state = 'queued'
                order by created_at
                limit 1
                for update skip locked
         )
        returning id, kind, document_id, state, progress, stage, attempts, error,
                  created_at, started_at, finished_at
        "#,
    )
    .fetch_optional(pool)
    .await?;
    Ok(job)
}

pub async fn set_progress(pool: &PgPool, id: Uuid, pct: i32, stage: &str) -> Result<()> {
    sqlx::query("update jobs set progress = $2, stage = $3 where id = $1")
        .bind(id)
        .bind(pct.clamp(0, 100))
        .bind(stage)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn complete(pool: &PgPool, id: Uuid) -> Result<()> {
    sqlx::query(
        "update jobs set state = 'done', progress = 100, stage = 'done', finished_at = now() where id = $1",
    )
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Mark a job failed. Returns `true` when it will be retried.
pub async fn fail(pool: &PgPool, id: Uuid, error: &str) -> Result<bool> {
    let attempts: i32 = sqlx::query_scalar("select attempts from jobs where id = $1")
        .bind(id)
        .fetch_one(pool)
        .await?;

    if attempts < MAX_ATTEMPTS {
        sqlx::query(
            "update jobs set state = 'queued', stage = 'retry', error = $2, started_at = null where id = $1",
        )
        .bind(id)
        .bind(error)
        .execute(pool)
        .await?;
        return Ok(true);
    }

    let mut tx = pool.begin().await?;
    sqlx::query(
        "update jobs set state = 'failed', stage = 'failed', error = $2, finished_at = now() where id = $1",
    )
    .bind(id)
    .bind(error)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "update documents set status = 'failed', error = $2, updated_at = now()
          where id = (select document_id from jobs where id = $1)",
    )
    .bind(id)
    .bind(error)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(false)
}

/// Requeue jobs left `running` by a crashed worker. Called once at worker startup.
pub async fn requeue_orphans(pool: &PgPool) -> Result<u64> {
    let result = sqlx::query(
        "update jobs set state = 'queued', stage = 'requeued', started_at = null where state = 'running'",
    )
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}
