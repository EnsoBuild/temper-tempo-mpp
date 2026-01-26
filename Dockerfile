# Build stage
FROM rust:1.75-slim-bookworm as builder

WORKDIR /app

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Copy source
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates

# Build release binary
RUN cargo build --release -p temper-server --features "ethereum"

# Runtime stage
FROM debian:bookworm-slim

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    curl \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user
RUN useradd -m -u 1000 temper

# Copy binary and web assets
COPY --from=builder /app/target/release/temper-server /usr/local/bin/
COPY web /app/web

WORKDIR /app

# Create data directory
RUN mkdir -p /data && chown temper:temper /data

USER temper

# Environment
ENV RUST_LOG=info
ENV DATABASE_PATH=/data/temper.db
ENV PORT=8089

EXPOSE 8089

# Health check
HEALTHCHECK --interval=30s --timeout=10s --start-period=5s --retries=3 \
    CMD curl -f http://localhost:8089/ || exit 1

CMD ["temper-server"]
