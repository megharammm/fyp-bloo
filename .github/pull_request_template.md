# Pull request

## Work package

- WP: `WP-__`
- Branch: `wp-__-<slug>`
- Owner: @

## What changed

<!-- one paragraph, no bullet soup -->

## Acceptance checks

Copy the WP's acceptance list from `PLAYBOOK.md` §10 and tick what you verified.

- [ ] 
- [ ] 

Evidence (command output, test name, or screenshot path):

```
```

## Gate

- [ ] `cargo fmt --all --check` — no output
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` — Finished
- [ ] `cargo test --workspace` — 0 failed
- [ ] `scripts\start.ps1` still serves `/`, `/admin`, `/health`

## Constraints (reject the PR if any is false)

- [ ] Rust only — no Node, npm, Python, or JS framework added
- [ ] One Postgres — no Redis, Qdrant, MinIO, or second datastore
- [ ] Ports unchanged: 27432 / 27810
- [ ] No `.pdf` link or `file_path` exposed in `/api/chat` responses
- [ ] No new dependency without a one-line justification above
- [ ] No secrets, no `.env`, no `data/` committed
- [ ] `D:\Trivya-Data-Pipeline` untouched

## Screenshots

<!-- UI changes require before/after -->
