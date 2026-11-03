# Bloo AI — handoff

Last updated: 2026-11-03 · `main` @ `c84ca55`

Read this first, then `PLAYBOOK.md` for the work packages. This file is the short version:
what exists, how to run it, what bit us, what is next.

---

## Where we are

| WP | State | Evidence |
|---|---|---|
| P0 scaffold | ✅ | compose, schema, CI, branch protection |
| P1 parsing | ✅ | 18 tests + real PDF via PDFium |
| P2 ingestion + queue | ✅ | smoke 14/14 |
| P3 retrieval + chat API | ✅ | smoke 22/22 |
| P4 chat UI | ✅ | streaming, citations, feedback, a11y pass |
| P5 admin console | ✅ | smoke 20/20 |
| P6 crawler | ⬜ | next backend task |
| P7 eval harness | ⬜ | needed for the report |

**Workspace tests:** 46 · **clippy:** clean · **CI:** green.

Current corpus: 5 published documents, 31 chunks (the five `content/seed/*.md` files).

---

## Running it

```powershell
cd D:\BLOO
START-BLOO.cmd            # db + API, builds if needed
```

| URL | |
|---|---|
| `http://127.0.0.1:27810` | student chat |
| `http://127.0.0.1:27810/admin` | knowledge console |
| `http://127.0.0.1:27810/thx01` | THX-01 model console |
| `http://127.0.0.1:27810/health` | health |

Ingestion runs in a second process:

```powershell
cargo run -p bloo_api --bin worker
```

THX-01 (routing) is a third process, optional:

```powershell
scripts\start-thx01.ps1    # 127.0.0.1:8095
```

Without it, routing degrades and answers still work.

---

## Measured numbers (for the report)

| Metric | Value |
|---|---|
| embed + retrieve | 191 ms |
| first token (`deepseek-v4.1-flash`) | ~3.1 s |
| first token (`glm-5.3-flash`) | 2.6 s — one-line swap |
| answer p95 (WP-03 smoke) | 3.8 s |
| THX-01 route (CPU) | 96 ms |
| THX-01 choice+number+cite+excerpt | 4.99 s |
| chunk size | avg 265 / max 448 tokens (cap 600) |
| RAM | ~250 MB steady, ~450 MB ingesting |

---

## Decisions worth remembering

- **Rust only.** Axum + Askama + vanilla JS. No Node, no npm.
- **One Postgres.** pgvector + `tsvector` + RRF. No Redis, no Qdrant, no object store.
- **THX-01 is an accelerator, not a dependency.** Self-hosted (`doofz/THX-01`, Apache-2.0),
  used for routing at query time; citation spans use a local sentence picker
  (`BLOO_THX_CITATIONS=1` turns the model-backed extractor back on, +2–3 s).
- **Embeddings stay local:** `bge-small-en-v1.5`, 384 d, ONNX, ~130 MB. A remote
  OpenAI-compatible backend exists but is dormant (`BLOO_EMBED_PROVIDER=openai`).
- **Ports are frozen:** Postgres `27432`, app `27810`, THX sidecar `27812`. Compose project
  `blooai`. Never touch `D:\Trivya-Data-Pipeline`.
- **Nothing is answerable until `published`.** Retrieval reads `searchable_chunks` only.

---

## Things that bit us (don't repeat)

1. **PowerShell 5.1 + native stderr.** `$ErrorActionPreference = 'Stop'` plus `2>&1` turns
   cargo/psql progress lines into terminating errors. Never redirect native output; check
   `$LASTEXITCODE`.
2. **Migrations must stay LF.** `.gitattributes` pins `*.sql text eol=lf`. CRLF checkouts change
   the sqlx checksum and block startup.
3. **Cargo does not watch `migrations/`.** `crates/bloo-core/build.rs` emits
   `rerun-if-changed=../../migrations`; without it a new migration is silently skipped.
4. **PDFium binds once per process.** Cache the `Pdfium` instance; rebinding fails with
   `PdfiumLibraryBindingsAlreadyInitialized`.
5. **OpenCode Go vs Console use different base paths.** Go key on the Console path returns
   `402`, which looks like "no credits". `scripts\check-llm.ps1` diagnoses it.
6. **Hash uniqueness must exclude archived rows.** Otherwise a changed or re-added file fails
   with a duplicate key (fixed in migration 0004).
7. **`cargo run` is ambiguous** with two binaries; `default-run = "bloo_api"` is set.

---

## Smoke tests

```powershell
scripts\smoke-wp02.ps1    # ingestion: 14 checks
scripts\smoke-wp03.ps1    # retrieval/answers: 22 checks
scripts\smoke-wp05.ps1    # admin console: 20 checks
```

Each resets the database and leaves a mutated corpus. Afterwards restore the demo state:

```powershell
# start API + worker, then
curl -X POST localhost:27810/api/admin/sync
# publish everything that is ready
docker exec blooai-db psql -U bloo -d blooai -c "update documents set status='published' where status='ready';"
```

If startup fails with a migration checksum error: `scripts\reset-db.ps1` (type `RESET`),
then sync again.

---

## Next session

1. **WP-06 — crawler**: `content/` sync exists; the crawler adds URL sources with change
   detection. Reuse `ingest::sync` semantics (path identity, version on change, archive on
   removal).
2. **WP-07 — eval harness**: golden set (~150 questions), retrieval metrics (hit@5, MRR),
   generation metrics via THX-01 judge, snapshots and deltas. This produces the report numbers.
3. **Frontend/UX pass** — the owner is taking this; keep the contract: SSE event names
   `token` / `citations` / `done`, citation shape `{title, section_path, page_label, quote, url}`,
   and no PDF links.

---

## Repo map

```
crates/bloo-core    config, db, models
crates/bloo-parse   pdfium/docx/html/md -> blocks
crates/bloo-ingest  chunk, queue, storage, pipeline, sync
crates/bloo-rag     embed, store (hybrid), thx, prompt, llm, retrieve
crates/bloo-api     Axum app + worker binary + templates
content/seed        demo corpus (markdown)
scripts/            dev, start, upgrade, sync, smoke tests, pdfium, reset-db
migrations/         LF only
templates/          base, chat, admin, thx01
static/             app.css, admin.js
```
