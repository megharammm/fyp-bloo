---
name: bloo-dev
description: Build Bloo AI, the Rust smart-campus RAG assistant in D:\BLOO. Use when implementing a work package (WP-00 … WP-08), wiring the ingestion pipeline, retrieval, THX-01 calls, chat/admin UI, or running the eval harness. Enforces the project's hard constraints (Rust only, one Postgres, frozen ports, Docker isolation from Trivya-Data-Pipeline) and the acceptance gates.
---

# Bloo AI — development skill

You are the implementation agent for **Bloo AI**, a Rust smart-campus RAG assistant at
`D:\BLOO`. Read `AGENTS.md` and `PLAYBOOK.md` before your first edit. This skill is your
execution protocol; `PLAYBOOK.md` §10 is the specification of each work package.

---

## 1. Contract

- **Rust only.** Axum + Askama + htmx. No Node, npm, Python, or JS framework.
- **One Postgres with pgvector.** No Redis, Qdrant, MinIO, Elasticsearch, or second datastore.
- **Ports are frozen:** Postgres `27432`, app `27810`.
- **Docker isolation:** always `docker compose -p blooai`. Never `docker system prune`,
  `docker volume prune`, or remove containers you did not create.
  `D:\Trivya-Data-Pipeline` is off limits — do not read, rebuild, or stop it.
- **Students never see source files.** Citations are quoted text + section label; `kind=pdf`
  has no URL.
- **Nothing is retrievable until published** (`documents.status='published'`, view
  `searchable_chunks`).

Violating any of these is a failed task, even if the tests pass.

---

## 2. Boot sequence — every session, no exceptions

```powershell
cd D:\BLOO
git status --short
git log --oneline -5
Get-Content AGENTS.md
Get-Content PLAYBOOK.md -TotalCount 140

cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

docker ps --filter "name=blooai" --format "{{.Names}}  {{.Status}}  {{.Ports}}"
```

If the baseline is red on a clean tree, fix that first in its own commit and report it.
Never build on a red baseline.

---

## 3. Choose the next work package

Take the **lowest-numbered WP** in `PLAYBOOK.md` §10 whose:

1. dependencies are met, and
2. checklist still has unchecked items.

If several qualify, pick the one that unblocks the most teammates. Announce before coding:

```
WP-02 · ingestion + chunking · files: crates/bloo-ingest/src/*, crates/bloo-api/src/bin/worker.rs
```

Read that WP's **Acceptance** block now. Those are your tests. Write them before or alongside
the implementation — never after.

---

## 4. Implementation loop

1. **Read before write.** Open every file you will touch; match its error handling, naming and
   module layout. Do not introduce a second style.
2. **Smallest vertical slice.** One callable path end to end (route → handler → query → row)
   before broadening.
3. **Test in the same commit.** No "tests later".
4. **Narrow loop:**
   ```powershell
   cargo check -p <crate>
   cargo test  -p <crate>
   cargo clippy -p <crate> --all-targets -- -D warnings
   ```
5. **Wide gate before commit:**
   ```powershell
   cargo fmt --all
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test --workspace
   ```
6. **Commit:** `WP-02: <what changed>` + body listing each acceptance check and how it was
   verified.

---

## 5. Dependency policy

Before `cargo add X`, answer:

- Does it break "Rust only, one Postgres, no extra daemon"? If yes — reject.
- Is it maintained or the de-facto choice? (`axum`, `sqlx`, `askama`, `pdfium-render`,
  `fastembed`, `scraper`, `pulldown-cmark` qualify.)
- Can it be feature-gated so `cargo test` works offline?

Resolve versions with `cargo add`; never hand-write a version from memory.

---

## 6. Hard stops

Do not:

- add authentication, ACLs, or multi-tenancy (out of scope by design)
- add a JS framework, npm, or a Node build step
- add Redis / Qdrant / MinIO / any second database
- bind any port other than 27810 / 27432
- run docker commands without `-p blooai`
- expose `file_path` or any `.pdf` link in `/api/chat` responses
- edit `migrations/0001_init.sql` after merge — add `0002_*.sql`
- use sqlx `query!` macros (they require a live DB at build time) — use `sqlx::query()`
- commit `.env`, `data/`, or model weights

---

## 7. When blocked

1. Re-read the WP acceptance block.
2. Check the error against `PLAYBOOK.md` Appendix D.
3. Grep `crates/` before assuming something is missing.
4. If Docker is down, continue with pure, unit-testable work (parsers, chunker, SQL strings,
   templates) and say that integration verification is pending.
5. Ask a human only then — with: what you tried, the exact error, two options, your pick.

Never ask a question the repo already answers.

---

## 8. Definition of done

```powershell
cargo fmt --all --check                                            # no output
cargo clippy --workspace --all-targets -- -D warnings              # Finished
cargo test --workspace                                             # 0 failed
docker ps --filter "name=blooai" --format "{{.Names}} {{.Status}}" # blooai-db (healthy)
```

Plus every box in the WP's Acceptance list demonstrated by a test, a command output, or a
screenshot path. "It compiles" is not done.

---

## 9. Report format

```
WP: <id>
Changed: <files>
Acceptance: <n>/<n> — <how each was verified>
Gate: fmt ✅ clippy ✅ test ✅
Next: <next WP> because <reason>
Risks: <anything a human must decide>
```

---

## 10. First-run expectations (fresh clone)

```powershell
cd D:\BLOO
Copy-Item .env.example .env     # add HALX_API_KEY + LLM_API_KEY
cargo check --workspace
scripts\dev.ps1                 # db on 27432, app on 27810
```

| URL | Expected today |
|---|---|
| `/` | chat shell renders |
| `/admin` | knowledge console renders |
| `/health` | `{"service":"bloo-ai","status":"ok","db":true}` |

Progression after each WP is listed in `PLAYBOOK.md` §17.9.

---

## 11. Session budget

One WP per session. One concern per commit. If you finish early, write `#[ignore]` acceptance
stubs for the next WP — do not start implementing it. Scope creep is how this project fails.
