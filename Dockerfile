# Stage 1: Generate recipe for dependency caching
FROM rust:slim-bookworm AS chef
RUN cargo install cargo-chef
WORKDIR /app

# Stage 2: Prepare recipe (just dependency info)
FROM chef AS planner
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN cargo chef prepare --recipe-path recipe.json

# Stage 3: Build dependencies (cached if Cargo.toml/lock unchanged)
FROM chef AS builder

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    git \
    && rm -rf /var/lib/apt/lists/*

# Build dependencies first (cached layer)
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json -p temper-server --features "ethereum"

# Now copy source and build (only this runs on code changes)
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY web ./web
RUN cargo build --release -p temper-server --features "ethereum"

# Stage 4: Runtime
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    curl \
    && rm -rf /var/lib/apt/lists/*

RUN useradd -m -u 1000 temper

COPY --from=builder /app/target/release/temper-server /usr/local/bin/
COPY web /app/web

WORKDIR /app
RUN mkdir -p /data && chown temper:temper /data

USER temper

ENV RUST_LOG=info
ENV DATABASE_PATH=/data/temper.db
ENV PORT=8089

EXPOSE 8089

HEALTHCHECK --interval=30s --timeout=10s --start-period=5s --retries=3 \
    CMD curl -f http://localhost:8089/ || exit 1

CMD ["temper-server"]
