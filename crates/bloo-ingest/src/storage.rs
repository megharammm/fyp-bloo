//! Content-addressed local storage. One file per sha256, no object store.

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct StoredFile {
    pub hash: String,
    /// Path relative to `BLOO_DATA_DIR`, e.g. `raw/ab12….pdf`. This is what the DB stores.
    pub relative: String,
}

#[derive(Debug, Clone)]
pub struct Storage {
    root: PathBuf,
}

impl Storage {
    /// `root` is the directory that holds raw files (`<data_dir>/raw`).
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn sha256(bytes: &[u8]) -> String {
        hex::encode(Sha256::digest(bytes))
    }

    pub fn put(&self, bytes: &[u8], ext: &str) -> Result<StoredFile> {
        std::fs::create_dir_all(&self.root)
            .with_context(|| format!("cannot create {}", self.root.display()))?;

        let hash = Self::sha256(bytes);
        let ext = ext.trim_start_matches('.').to_ascii_lowercase();
        let name = if ext.is_empty() {
            hash.clone()
        } else {
            format!("{hash}.{ext}")
        };

        let path = self.root.join(&name);
        if !path.exists() {
            std::fs::write(&path, bytes)
                .with_context(|| format!("cannot write {}", path.display()))?;
        }

        Ok(StoredFile {
            hash,
            relative: format!("raw/{name}"),
        })
    }

    /// Remove every stored file whose name starts with this hash.
    pub fn delete(&self, hash: &str) -> Result<usize> {
        let mut removed = 0;
        if !self.root.exists() {
            return Ok(0);
        }
        for entry in std::fs::read_dir(&self.root)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with(hash) {
                std::fs::remove_file(entry.path())?;
                removed += 1;
            }
        }
        Ok(removed)
    }

    pub fn read(&self, relative: &str) -> Result<Vec<u8>> {
        let path = self.root.join(
            std::path::Path::new(relative)
                .file_name()
                .unwrap_or_default(),
        );
        std::fs::read(&path).with_context(|| format!("cannot read {}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_are_stable_and_content_addressed() {
        let a = Storage::sha256(b"hello");
        let b = Storage::sha256(b"hello");
        let c = Storage::sha256(b"hello!");
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(a.len(), 64);
    }
}
