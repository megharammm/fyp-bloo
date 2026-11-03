use sqlx::postgres::{PgPool, PgPoolOptions};
use std::time::Duration;

pub async fn connect(database_url: &str) -> anyhow::Result<PgPool> {
    let pool = PgPoolOptions::new()
        // Small pool on purpose: Bloo is a single-laptop deployment.
        .max_connections(8)
        .min_connections(1)
        .acquire_timeout(Duration::from_secs(5))
        .connect(database_url)
        .await?;
    Ok(pool)
}

pub async fn migrate(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::migrate!("../../migrations").run(pool).await?;
    Ok(())
}
