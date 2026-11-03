//! Content sync: walk `content/`, diff against the database, ingest only what changed.
//!
//! Identity is the file path, not the hash. A changed file archives the previous row and
//! inserts a new version, so the table never shows two live rows with the same title.

use crate::queue;
use crate::storage::Storage;
use anyhow::{Context, Result};
use bloo_core::{Config, PgPool};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::Path;

#[derive(Debug, Default, Clone, Serialize)]
pub struct SyncReport {
    pub new: usize,
    pub changed: usize,
    pub unchanged: usize,
    pub removed: usize,
    pub queued: usize,
    pub skipped: usize,
    pub errors: Vec<String>,
}

fn supported(ext: &str) -> Option<&'static str> {
    match ext.to_ascii_lowercase().as_str() {
        "pdf" => Some("pdf"),
        "docx" => Some("docx"),
        "html" | "htm" => Some("html"),
        "md" | "markdown" => Some("md"),
        "txt" => Some("txt"),
        _ => None,
    }
}

fn mime_for(kind: &str) -> &'static str {
    match kind {
        "pdf" => "application/pdf",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "html" => "text/html",
        "md" => "text/markdown",
        _ => "text/plain",
    }
}

fn should_skip(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower == "readme.md" || lower == "manifest.json" || name.starts_with('.')
}

fn collect(root: &Path) -> Result<Vec<(String, std::path::PathBuf)>> {
    let mut out = Vec::new();
    if !root.exists() {
        return Ok(out);
    }
    walk(root, root, &mut out)?;
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, std::path::PathBuf)>) -> Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            walk(root, &path, out)?;
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if should_skip(&name) {
            continue;
        }
        let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
            continue;
        };
        if supported(ext).is_none() {
            continue;
        }
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        out.push((rel, path));
    }
    Ok(())
}

/// Sync the content directory with the database.
pub async fn sync_content(pool: &PgPool, cfg: &Config) -> Result<SyncReport> {
    let storage = Storage::new(cfg.data_dir.join("raw"));
    let files = collect(&cfg.content_dir)
        .with_context(|| format!("cannot scan {}", cfg.content_dir.display()))?;

    let mut report = SyncReport::default();
    let mut seen: HashSet<String> = HashSet::new();

    // Existing live rows keyed by path.
    let rows = sqlx::query_as::<_, (String, String, i32, String)>(
        "select source_path, file_hash, version, status
           from documents
          where source_path is not null and status <> 'archived'",
    )
    .fetch_all(pool)
    .await?;
    let live: HashMap<String, (String, i32)> = rows
        .into_iter()
        .map(|(path, hash, version, _status)| (path, (hash, version)))
        .collect();

    for (rel, path) in files {
        seen.insert(rel.clone());

        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| rel.clone());
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default()
            .to_string();
        let Some(kind) = supported(&ext) else {
            report.skipped += 1;
            continue;
        };

        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(err) => {
                report.errors.push(format!("{rel}: {err}"));
                continue;
            }
        };
        let stored = match storage.put(&bytes, &ext) {
            Ok(s) => s,
            Err(err) => {
                report.errors.push(format!("{rel}: {err}"));
                continue;
            }
        };

        match live.get(&rel) {
            Some((hash, _version)) if *hash == stored.hash => {
                report.unchanged += 1;
            }
            Some((_hash, version)) => {
                // Changed: retire the old row, insert the next version.
                sqlx::query(
                    "update documents set status = 'archived', updated_at = now()
                      where source_path = $1 and status <> 'archived'",
                )
                .bind(&rel)
                .execute(pool)
                .await?;

                let id: uuid::Uuid = sqlx::query_scalar(
                    "insert into documents
                        (title, file_path, file_hash, mime, kind, category, status,
                         source_path, version)
                     values ($1, $2, $3, $4, $5, 'general', 'processing', $6, $7)
                     returning id",
                )
                .bind(&name)
                .bind(&stored.relative)
                .bind(&stored.hash)
                .bind(mime_for(kind))
                .bind(kind)
                .bind(&rel)
                .bind(version + 1)
                .fetch_one(pool)
                .await?;

                queue::enqueue(
                    pool,
                    "ingest",
                    Some(id),
                    serde_json::json!({ "file": name, "sync": "changed" }),
                )
                .await?;

                report.changed += 1;
                report.queued += 1;
            }
            None => {
                // A path may have archived history; continue its version sequence.
                let previous: i32 = sqlx::query_scalar(
                    "select coalesce(max(version), 0) from documents where source_path = $1",
                )
                .bind(&rel)
                .fetch_one(pool)
                .await?;

                let id: uuid::Uuid = sqlx::query_scalar(
                    "insert into documents
                        (title, file_path, file_hash, mime, kind, category, status,
                         source_path, version)
                     values ($1, $2, $3, $4, $5, 'general', 'processing', $6, $7)
                     returning id",
                )
                .bind(&name)
                .bind(&stored.relative)
                .bind(&stored.hash)
                .bind(mime_for(kind))
                .bind(kind)
                .bind(&rel)
                .bind(previous + 1)
                .fetch_one(pool)
                .await?;

                queue::enqueue(
                    pool,
                    "ingest",
                    Some(id),
                    serde_json::json!({ "file": name, "sync": "new" }),
                )
                .await?;

                report.new += 1;
                report.queued += 1;
            }
        }
    }

    // Anything that disappeared from disk is archived, never hard-deleted.
    for path in live.keys() {
        if !seen.contains(path) {
            sqlx::query(
                "update documents set status = 'archived', updated_at = now()
                  where source_path = $1 and status <> 'archived'",
            )
            .bind(path)
            .execute(pool)
            .await?;
            report.removed += 1;
        }
    }

    sqlx::query(
        "insert into app_settings (key, value, updated_at)
         values ('last_sync_at', to_jsonb(now()), now())
         on conflict (key) do update set value = excluded.value, updated_at = now()",
    )
    .execute(pool)
    .await?;

    Ok(report)
}
