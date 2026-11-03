//! Bloo AI worker: claims ingestion jobs and runs the pipeline.
//!
//! One process, one worker, Postgres queue. Start it beside the API:
//!
//! ```powershell
//! cargo run -p bloo_api --bin worker
//! ```

use anyhow::{Context, Result};
use bloo_core::Config;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,bloo_ingest=debug,bloo_rag=debug".into()),
        )
        .compact()
        .init();

    let cfg = Config::from_env().context("invalid configuration")?;
    let pool = bloo_core::connect(&cfg.database_url)
        .await
        .context("cannot reach Postgres - is blooai-db up on 27432?")?;
    bloo_core::migrate(&pool).await?;

    let embedder = bloo_rag::build(&cfg)?;
    tracing::info!(
        embedder = embedder.name(),
        dim = embedder.dim(),
        "worker ready"
    );

    // Single-worker assumption: anything still `running` was orphaned by a crash.
    let requeued = bloo_ingest::queue::requeue_orphans(&pool).await?;
    if requeued > 0 {
        tracing::warn!(count = requeued, "requeued orphaned jobs");
    }

    loop {
        match bloo_ingest::queue::claim(&pool).await? {
            Some(job) => {
                tracing::info!(job = %job.id, kind = %job.kind, attempt = job.attempts, "claimed");
                let started = std::time::Instant::now();

                match bloo_ingest::pipeline::process(&pool, &cfg, embedder.as_ref(), &job).await {
                    Ok(()) => {
                        bloo_ingest::queue::complete(&pool, job.id).await?;
                        tracing::info!(job = %job.id, ms = started.elapsed().as_millis(), "done");
                    }
                    Err(err) => {
                        let retry =
                            bloo_ingest::queue::fail(&pool, job.id, &err.to_string()).await?;
                        tracing::warn!(job = %job.id, error = %err, retry, "job failed");
                    }
                }
            }
            None => tokio::time::sleep(Duration::from_millis(500)).await,
        }
    }
}
