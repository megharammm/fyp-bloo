# AGENTS.md — read this first

You are working on **Bloo AI**, an internal smart-campus RAG assistant.
Everything below is binding. Read `PLAYBOOK.md` before touching code.

## Hard rules

1. **Rust only.** No Node, no Python, no npm. UI is server-rendered Askama + htmx.
2. **One Postgres.** No Redis, no Qdrant, no S3, no vector service. Vectors live in
   Postgres via `pgvector`. Files live in `./data/` on disk.
3. **Ports are frozen:** Postgres `27432`, app `27810`. Never bind 3000/8000/5432/6379/6333.
4. **Docker isolation:** always `docker compose -p blooai`. Never run `docker system prune`,
   `docker volume prune`, or any command without `-p blooai`.
   `D:\Trivya-Data-Pipeline` is a different project. Do not read, touch, rebuild, or stop it.
5. **Never expose source PDFs to students.** Citations are quoted text + source label only.
6. **Nothing is answerable until `documents.status = 'published'`.** The `searchable_chunks`
   view is the only read path for retrieval.
7. **No secrets in git.** `.env` is ignored. Use `.env.example` for shape only.
8. **Never push to `main`.** Work on `wp-<n>-<slug>`, open a PR, owner reviews, squash merge.
   No force-push on a branch under review. Never merge your own PR.

## Before you write code

```powershell
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

All three must be green before and after your change.

## Where to look

| Question | File |
|---|---|
| What am I supposed to build? | `PLAYBOOK.md` → Work packages |
| How does the pipeline work? | `PLAYBOOK.md` → Architecture |
| How do I call THX-01? | `PLAYBOOK.md` → Appendix A |
| What is the schema? | `migrations/0001_init.sql` |
| How do I upgrade the stack? | `scripts/upgrade.ps1` |
| How do I branch, commit, get reviewed? | `docs/CONTRIBUTING.md` |
| How do I (re)apply branch protection? | `scripts/protect-main.ps1` |
| Autonomous dev protocol | `PLAYBOOK.md` → §17 Agent walkthrough skill |
