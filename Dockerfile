# Bloo AI — single binary, single process (UI + JSON API).
# Devs run `cargo run` on the host; this image is for the demo/review build.

FROM rust:1.96-slim-bookworm AS builder
WORKDIR /app

# Cache the dependency graph separately from our code.
COPY Cargo.toml Cargo.lock ./
COPY crates/bloo-core/Cargo.toml crates/bloo-core/Cargo.toml
COPY crates/bloo-api/Cargo.toml crates/bloo-api/Cargo.toml
RUN mkdir -p crates/bloo-core/src crates/bloo-api/src \
 && echo "fn main(){}" > crates/bloo-api/src/main.rs \
 && echo "" > crates/bloo-core/src/lib.rs \
 && cargo build --release --workspace || true

COPY crates ./crates
COPY migrations ./migrations
COPY templates ./templates
RUN touch crates/bloo-core/src/lib.rs crates/bloo-api/src/main.rs \
 && cargo build --release --bin bloo_api

FROM debian:bookworm-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /app/target/release/bloo_api /usr/local/bin/bloo_api
COPY templates ./templates
COPY static ./static

ENV BLOO_BIND=0.0.0.0:27810 \
    RUST_LOG=info

EXPOSE 27810
HEALTHCHECK --interval=10s --timeout=3s --retries=5 \
  CMD curl -fsS http://127.0.0.1:27810/health || exit 1

CMD ["bloo_api"]
