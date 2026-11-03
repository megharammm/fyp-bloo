//! Bloo AI — retrieval primitives.
//!
//! WP-03 fills this crate in: embeddings, hybrid retrieval, reranking, prompt assembly.
//! This module defines the embedding contract now so the storage dimension and the
//! provider can never drift apart later.

pub mod embed;

pub use embed::{build, Embedder, Embedding};
