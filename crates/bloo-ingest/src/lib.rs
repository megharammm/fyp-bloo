//! Bloo AI — ingestion pipeline.
//!
//! Upload → hash → parse → chunk → embed → store. The queue lives in Postgres, so there is no
//! Redis and no second source of truth.

pub mod chunk;
pub mod pipeline;
pub mod queue;
pub mod storage;
pub mod sync;

pub use chunk::{chunk_blocks, Chunk, ChunkOptions};
pub use storage::Storage;
pub use sync::SyncReport;
