# Bloo AI — Windows-first. Every target pins the compose project to `blooai`.
#
#   make db        start Postgres only (dev)
#   make dev       start Postgres + run the API on the host
#   make full      build + run everything in Docker
#   make upgrade   git pull -> rebuild -> migrate -> sync content
#   make sync      re-scan content/ and data/uploads, ingest what changed
#   make check     fmt + clippy + tests + cargo check

COMPOSE := docker compose -p blooai -f docker-compose.yml
PS := powershell -NoProfile -ExecutionPolicy Bypass -File

.PHONY: db dev full down logs upgrade sync check fmt clippy test seed

db:
	$(COMPOSE) up -d db

dev: db
	cargo run -p bloo_api

full:
	$(COMPOSE) --profile full up -d --build

down:
	$(COMPOSE) --profile full down

logs:
	$(COMPOSE) logs -f --tail=100

upgrade:
	$(PS) scripts/upgrade.ps1

sync:
	$(PS) scripts/sync.ps1

seed:
	$(PS) scripts/seed.ps1

check: fmt clippy
	cargo check --workspace
	cargo test --workspace

fmt:
	cargo fmt --all

clippy:
	cargo clippy --workspace --all-targets -- -D warnings

test:
	cargo test --workspace
