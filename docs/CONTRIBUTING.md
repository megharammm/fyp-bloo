# Contributing to Bloo AI

Four people, one Rust workspace, one laptop each. This file is the workflow contract.
`AGENTS.md` is the technical contract. `PLAYBOOK.md` is the specification.

---

## 1. First run (once per laptop)

Prerequisites: **Rust ≥ 1.85**, **Docker Desktop**, **git**, **git-lfs**.

```powershell
git clone https://github.com/megharammm/fyp-bloo.git D:\BLOO
cd D:\BLOO
git lfs install
powershell -ExecutionPolicy Bypass -File scripts\setup.ps1
```

`setup.ps1` checks the toolchain, starts Docker, writes `.env`, prompts for the two API keys,
creates the database, builds, launches, and opens <http://127.0.0.1:27810>.

Daily start: **double-click `START-BLOO.cmd`** (or `scripts\start.ps1`).
Stop: `STOP-BLOO.cmd` or `Ctrl+C`.

Keys are never committed. Each teammate asks the owner for `HALX_API_KEY` and `LLM_API_KEY`
and pastes them once during setup.

---

## 2. Branch model

`main` is always demo-able. Nobody — human or agent — pushes to it directly.

```
main
 └── wp-03-retrieval      ← one work package per branch
      └── PR → review → squash merge
```

Branch names:

```
wp-01-parse
wp-02-ingest
wp-03-retrieval
wp-04-chat-ui
wp-05-admin
wp-06-crawler
wp-07-eval
wp-08-demo
fix/<short-slug>
chore/<short-slug>
```

Start from a fresh `main`:

```powershell
git checkout main
git pull --ff-only
git checkout -b wp-03-retrieval
```

---

## 3. Commit rules

- One work package per branch. **One concern per commit.**
- Format: `WP-03: add hybrid retrieval with RRF fusion`
- Body: which acceptance checks you ran and the command output that proves it.
- Never commit `.env`, `data/`, `target/`, or model weights.
- Never rewrite history on a shared branch (`rebase -i` only on your own unpushed commits).

---

## 4. The gate (non-negotiable)

```powershell
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
scripts\start.ps1        # /, /admin, /health still work
```

Then push and open a PR. CI runs the same gate on Ubuntu — if it is red, the PR is not ready.

---

## 5. Review

- Owner (`@megharammm`) reviews. `CODEOWNERS` auto-requests it.
- Review looks at three things only:
  1. Does it meet the WP's acceptance list?
  2. Does it violate a hard constraint? (see the PR template checkboxes)
  3. Would a stranger understand this in six months?
- **Squash merge** to `main`. Delete the branch.
- If review asks for changes, push more commits to the same branch — do not force-push after
  review has started.

---

## 6. AI agents: read this twice

Agents are welcome and expected. They are held to a stricter rule set than humans.

**Allowed**

- Read the repo, create branches, write code and tests.
- Run `cargo`, `git`, and Docker commands scoped to project `blooai`.
- Commit to their own `wp-*` branch and open a PR.

**Forbidden**

- Pushing to `main`. No exceptions, no "it's just a README fix".
- `git push --force` on any branch that has an open PR.
- Merging their own PR.
- Running `docker compose` without `-p blooai`, or any `docker system/volume prune`.
- Reading, stopping, or modifying anything under `D:\Trivya-Data-Pipeline`.
- Adding a dependency that breaks "Rust only / one Postgres" (see `AGENTS.md` §1).
- Committing secrets, `.env`, or `data/`.
- Editing `migrations/0001_init.sql` after it is merged.

**Agent prompt that works**

> Read `D:\BLOO\AGENTS.md` and `D:\BLOO\PLAYBOOK.md`. Run the §17.1 boot sequence and report
> the results. Then implement WP-`__` exactly as specified, on branch `wp-__-<slug>`. Stop when
> the acceptance list is green and open a PR. Do not merge.

**Review is not optional because the agent is confident.** A PR with green CI and a filled
acceptance list is the only thing that gets merged.

---

## 7. Definition of ready for review

- [ ] Branch is rebased on current `main`
- [ ] CI green
- [ ] Every WP acceptance box ticked with evidence
- [ ] PR template fully filled, including the constraint checkboxes
- [ ] Screenshots attached for any UI change
- [ ] No unrelated files touched

---

## 8. When two people touch the same file

`crates/bloo-api/src/routes/mod.rs` and `Cargo.toml` are the usual collisions.

- **`Cargo.toml`**: add your dependency in the WP that needs it; announce it in the group chat
  before pushing.
- **`routes/mod.rs`**: add one `pub mod` line; if there is a conflict, keep both, alphabetically.
- **`migrations/`**: never edit an existing file. Add `0002_*.sql`, `0003_*.sql`.
- If a conflict is not trivial, **stop and ask** rather than "resolving" blindly. Bad merges
  in a RAG pipeline are invisible until the demo.
