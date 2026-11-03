//! Bloo AI — shared core: config, database, domain models.

pub mod config;
pub mod db;
pub mod models;

pub use config::Config;
pub use db::{connect, migrate};
pub use sqlx::PgPool;
