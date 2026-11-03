# Bloo AI

Internal smart-campus assistant. Ask a question in plain English, get an answer drawn only
from published campus documents — with the exact sentence it came from.

Built entirely in Rust. One Postgres. No Node, no Python, no vector service.

```
question ──► route ──► retrieve (dense + lexical) ──► rerank ──► generate ──► verify
             THX-01        pgvector + tsvector         THX-01      LLM        THX-01
```

---

## Quickstart

**First time on a laptop** (Rust ≥ 1.85, Docker Desktop, git, git-lfs):

```powershell
git clone https://github.com/megharammm/fyp-bloo.git D:\BLOO
cd D:\BLOO
git lfs install
powershell -ExecutionPolicy Bypass -File scripts\setup.ps1
```

It checks the toolchain, starts Docker, writes `.env`, asks for the two API keys, creates the
database, builds, and opens the browser.

**Every day after that:** double-click `START-BLOO.cmd`. Stop with `STOP-BLOO.cmd`.

| URL | |
|---|---|
| <http://127.0.0.1:27810> | chat |
| <http://127.0.0.1:27810/admin> | knowledge console |
| <http://127.0.0.1:27810/thx01> | THX-01 console — test the decision model directly |
| <http://127.0.0.1:27810/health> | health |

Full container build (demo day):

```powershell
scripts\upgrade.ps1
```

Team workflow — branches, reviews, agent rules: **[docs/CONTRIBUTING.md](docs/CONTRIBUTING.md)**.

---

## Commands

| Command | What it does |
|---|---|
| `scripts\setup.ps1` | First run on a new laptop: toolchain check, Docker, `.env`, keys, DB, build, launch |
| `scripts\start.ps1` | Daily start (`-Pull` to fast-forward first, `-Fast` to skip the build) |
| `scripts\stop.ps1` | Stop the app and the database, keep the volume |
| `START-BLOO.cmd` / `STOP-BLOO.cmd` | Double-click wrappers for the two above |
| `scripts\dev.ps1` | Same as start, plus the full port check |
| `scripts\upgrade.ps1` | `git pull` → rebuild image → migrate → restart → sync content |
| `scripts\sync.ps1` | Re-scan `content/` + `data/uploads`, ingest only what changed |
| `scripts\seed.ps1` | Load the demo corpus |
| `scripts\check-ports.ps1` | Refuse to start if 27432/27810 belong to another project |
| `scripts\protect-main.ps1` | Apply branch protection to `main` (needs `gh auth login`, once) |
| `scripts\check-halx.ps1` | Verify the HAL-X key against `/v1/models`; explains 401 vs 403 |
| `scripts\check-llm.ps1` | Verify the chat LLM base URL + key with a one-token request |
| `scripts\fetch-pdfium.ps1` | Install the native PDFium library into `data/pdfium/` |
| `cargo run -p bloo_parse --example dump -- <file>` | Print the parsed block stream for any document |
| `cargo run -p bloo_api --bin worker` | Run the ingestion worker (claims jobs from Postgres) |
| `scripts\make-test-pdfs.ps1` | Generate synthetic campus PDFs for testing |
| `scripts\smoke-wp02.ps1` | End-to-end ingestion test: 30 uploads, dedupe, delete, crash recovery |
| `scripts\smoke-wp03.ps1` | Retrieval + answering: hit@5, verbatim citations, refusals, SSE, p95 |
| `scripts\test-thx01.ps1` | Send a real Bloo-shaped request to the self-hosted THX-01 sidecar |
| `scripts\start-thx01.ps1` | Run THX-01 as a host process (recommended; avoids Docker/WSL memory) |
| `make check` | fmt + clippy + test + check |

---

## Team workflow

`main` is always demo-able. One work package per branch, PR, review by `@megharammm`, squash
merge. CI runs `fmt + clippy + test + release build` on every PR.

```
main
 └── wp-03-retrieval   ← your branch
      └── PR → review → squash
```

Agents may branch, commit and open PRs. They may **not** push to `main`, force-push a branch
under review, merge their own PR, or touch `D:\Trivya-Data-Pipeline`.

Full rules: **[docs/CONTRIBUTING.md](docs/CONTRIBUTING.md)**.

---

## Ports — do not change

| Service | Port |
|---|---|
| Postgres (`blooai-db`) | **27432** |
| App (`blooai-app`) | **27810** |
| THX-01 sidecar (`blooai-thx01`), optional | **27812** |

Compose project is pinned to `blooai`. Never run `docker compose` without `-p blooai`,
and never run `docker system prune` — `D:\Trivya-Data-Pipeline` shares this machine.

---

## Stack

| Layer | Choice |
|---|---|
| HTTP + UI | Axum 0.8 + Askama + htmx (no JS framework) |
| DB + vectors | Postgres 16 + pgvector (HNSW) |
| Lexical search | `tsvector` + `websearch_to_tsquery` + RRF |
| Embeddings | `fastembed` · `bge-small-en-v1.5` · 384 d · ONNX · CPU |
| Routing / rerank / citations | HAL-X **THX-01** (typed decisions, ~10 ms) |
| Generation | OpenAI-compatible chat API, streamed |
| Queue | Postgres `SKIP LOCKED` |
| Eval | Rust CLI · retrieval metrics + THX-01 faithfulness |

Steady-state RAM ≈ 250 MB, peak while ingesting ≈ 450 MB.

---

## Status

| Phase | State |
|---|---|
| P0 scaffold — workspace, schema, compose, UI shells, scripts | ✅ done |
| P1 parsing (`bloo-parse`) | ✅ done |
| P2 ingestion + queue | ✅ done |
| P3 retrieval + THX-01 + chat | ✅ done |
| P4 chat UI | ⬜ |
| P5 admin console | ⬜ |
| P6 crawler | ⬜ |
| P7 eval | ⬜ |
| P8 hardening + demo | ⬜ |

Work package detail, acceptance criteria and the agent protocol: **[PLAYBOOK.md](PLAYBOOK.md)**.

---

## Repository

```
crates/bloo-core   config · db · models            ✅
crates/bloo-api    Axum app + templates            ✅
crates/bloo-parse  parsers (pdfium/docx/html)      ✅ P1
crates/bloo-ingest chunker · queue · storage · pipeline ✅ P2
crates/bloo-rag    embed · retrieve · thx · llm    ⬜ P3
crates/bloo-eval   retrieval + generation metrics  ⬜ P7
migrations/        schema
templates/         Askama pages
content/seed/      demo corpus (git LFS)
data/              runtime files (gitignored)
```

---

## Known limitations (v1, by design)

Single admin, no auth. English only. No OCR (scanned PDFs are not indexed). No per-document
ACLs. Table extraction is heuristic, not Docling-grade. All of these are documented and
argued in the report rather than hidden.
