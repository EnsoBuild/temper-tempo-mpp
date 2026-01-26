# Temper Deployment Guide

This guide covers deploying Temper API and UI to production.

## Quick Comparison

| Platform | Ease | Cost | Best For |
|----------|------|------|----------|
| **Render** | Easy | Free tier available | Quick start, auto-deploy |
| **Fly.io** | Easy | Pay-as-you-go | Global edge, low latency |
| **Railway** | Easy | Usage-based | GitHub integration |
| **Docker** | Medium | Self-managed | Full control, VPS |

**Recommendation**: Start with **Render** or **Railway** for simplicity. All platforms work well.

## Architecture Overview

```
┌─────────────────┐     ┌─────────────────┐     ┌─────────────────┐
│   Load Balancer │────▶│  Temper Server  │────▶│   RPC Nodes     │
│   (Cloudflare)  │     │    (Rust)       │     │ (Alchemy/etc)   │
└─────────────────┘     └────────┬────────┘     └─────────────────┘
                                 │
                        ┌────────▼────────┐
                        │    SQLite DB    │
                        │  (Simulations)  │
                        └─────────────────┘
```

## Option 1: Render (Easiest)

### 1. Create render.yaml

```yaml
services:
  - type: web
    name: temper-api
    env: docker
    dockerfilePath: ./Dockerfile
    healthCheckPath: /
    envVars:
      - key: RUST_LOG
        value: info
      - key: ETHEREUM_RPC_URL
        sync: false
      - key: POLYGON_RPC_URL
        sync: false
      - key: DATABASE_PATH
        value: /data/temper.db
    disk:
      name: temper-data
      mountPath: /data
      sizeGB: 1
```

### 2. Deploy

1. Go to [render.com](https://render.com)
2. New → Web Service
3. Connect your GitHub repo
4. Render auto-detects `render.yaml` or Dockerfile
5. Add environment variables (RPC URLs)
6. Deploy

### 3. Custom Domain

In Render dashboard: Settings → Custom Domain → Add your domain

**Pros**: Free tier, auto-deploy on git push, managed SSL, simple UI
**Cons**: Cold starts on free tier, US regions only (free)

---

## Option 2: Docker Deployment

### 1. Create Dockerfile

```dockerfile
# Dockerfile
FROM rust:1.75-slim as builder

WORKDIR /app
COPY . .

RUN apt-get update && apt-get install -y pkg-config libssl-dev
RUN cargo build --release --features "ethereum"

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/temper-server /usr/local/bin/
COPY --from=builder /app/web /app/web

WORKDIR /app
ENV RUST_LOG=info

EXPOSE 8089

CMD ["temper-server"]
```

### 2. Create docker-compose.yml

```yaml
version: '3.8'

services:
  temper:
    build: .
    ports:
      - "8089:8089"
    environment:
      - RUST_LOG=info
      - ETHEREUM_RPC_URL=${ETHEREUM_RPC_URL}
      - POLYGON_RPC_URL=${POLYGON_RPC_URL}
      - ARBITRUM_RPC_URL=${ARBITRUM_RPC_URL}
      - BASE_RPC_URL=${BASE_RPC_URL}
      - OPTIMISM_RPC_URL=${OPTIMISM_RPC_URL}
      - DATABASE_PATH=/data/temper.db
      - BASE_URL=https://api.temper.dev
    volumes:
      - temper-data:/data
    restart: unless-stopped
    healthcheck:
      test: ["CMD", "curl", "-f", "http://localhost:8089/health"]
      interval: 30s
      timeout: 10s
      retries: 3

volumes:
  temper-data:
```

### 3. Deploy

```bash
# Build and run
docker-compose up -d

# View logs
docker-compose logs -f temper

# Scale (if using swarm)
docker service scale temper=3
```

## Option 3: Fly.io

### 1. Create fly.toml

```toml
app = "temper-api"
primary_region = "iad"

[build]
dockerfile = "Dockerfile"

[env]
RUST_LOG = "info"
PORT = "8089"

[http_service]
internal_port = 8089
force_https = true
auto_stop_machines = false
auto_start_machines = true
min_machines_running = 1

[[http_service.checks]]
grace_period = "10s"
interval = "30s"
method = "GET"
path = "/health"
timeout = "5s"

[mounts]
source = "temper_data"
destination = "/data"

[[vm]]
cpu_kind = "shared"
cpus = 2
memory_mb = 1024
```

### 2. Deploy

```bash
# Install flyctl
curl -L https://fly.io/install.sh | sh

# Login
fly auth login

# Create app
fly launch

# Set secrets
fly secrets set ETHEREUM_RPC_URL="https://eth-mainnet.g.alchemy.com/v2/..."
fly secrets set POLYGON_RPC_URL="https://polygon-mainnet.g.alchemy.com/v2/..."

# Deploy
fly deploy

# Scale
fly scale count 2
```

## Option 4: Railway

### 1. Connect GitHub repo

1. Go to railway.app
2. New Project → Deploy from GitHub
3. Select temper repo

### 2. Configure

```bash
# Build command
cargo build --release --features "ethereum"

# Start command
./target/release/temper-server

# Variables
RUST_LOG=info
ETHEREUM_RPC_URL=...
DATABASE_PATH=/data/temper.db
```

## Option 5: VPS/Bare Metal

### 1. Install dependencies

```bash
# Ubuntu/Debian
apt-get update
apt-get install -y build-essential pkg-config libssl-dev

# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

### 2. Build

```bash
git clone https://github.com/enso/temper.git
cd temper
cargo build --release --features "ethereum"
```

### 3. Create systemd service

```ini
# /etc/systemd/system/temper.service
[Unit]
Description=Temper Transaction Simulation API
After=network.target

[Service]
Type=simple
User=temper
WorkingDirectory=/opt/temper
ExecStart=/opt/temper/target/release/temper-server
Environment=RUST_LOG=info
Environment=ETHEREUM_RPC_URL=...
Environment=DATABASE_PATH=/opt/temper/data/temper.db
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
```

### 4. Start

```bash
sudo systemctl enable temper
sudo systemctl start temper
sudo systemctl status temper
```

## Reverse Proxy (Nginx)

```nginx
server {
    listen 443 ssl http2;
    server_name api.temper.dev;

    ssl_certificate /etc/letsencrypt/live/api.temper.dev/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/api.temper.dev/privkey.pem;

    location / {
        proxy_pass http://127.0.0.1:8089;
        proxy_http_version 1.1;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;

        # WebSocket support (if needed)
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";

        # Timeouts for large simulations
        proxy_read_timeout 300s;
        proxy_connect_timeout 75s;
    }
}
```

## Environment Variables

| Variable | Description | Required |
|----------|-------------|----------|
| `ETHEREUM_RPC_URL` | Ethereum mainnet RPC | Yes |
| `POLYGON_RPC_URL` | Polygon mainnet RPC | No |
| `ARBITRUM_RPC_URL` | Arbitrum One RPC | No |
| `BASE_RPC_URL` | Base mainnet RPC | No |
| `OPTIMISM_RPC_URL` | Optimism mainnet RPC | No |
| `DATABASE_PATH` | Path to SQLite database | No (default: ./temper.db) |
| `BASE_URL` | Public URL for simulation links | No |
| `PORT` | Server port | No (default: 8089) |
| `RUST_LOG` | Log level (info, debug, trace) | No |

## RPC Provider Recommendations

For production, use archive nodes with debug trace support:

| Provider | Debug Trace | Archive | Recommended Plan |
|----------|-------------|---------|------------------|
| Alchemy | Yes (Growth+) | Yes | Growth |
| QuickNode | Yes (Build+) | Yes | Build |
| Infura | No | Yes | - |
| Ankr | No | Yes | - |

## Monitoring

### Health Check

```bash
curl https://api.temper.dev/health
# Returns: {"status": "ok"}
```

### Metrics (Prometheus)

Add to your Prometheus config:

```yaml
scrape_configs:
  - job_name: 'temper'
    static_configs:
      - targets: ['localhost:8089']
    metrics_path: '/metrics'
```

### Logging

```bash
# View logs (systemd)
journalctl -u temper -f

# View logs (Docker)
docker logs -f temper

# View logs (Fly.io)
fly logs
```

## Backup

### Database Backup

```bash
# Backup
sqlite3 /data/temper.db ".backup '/backup/temper-$(date +%Y%m%d).db'"

# Scheduled backup (cron)
0 */6 * * * sqlite3 /data/temper.db ".backup '/backup/temper-$(date +\%Y\%m\%d-\%H\%M).db'"
```

## Security Checklist

- [ ] Use HTTPS (TLS 1.3)
- [ ] Set up rate limiting
- [ ] Configure CORS properly
- [ ] Keep RPC URLs in secrets/env, not code
- [ ] Regular security updates
- [ ] Monitor for unusual activity
- [ ] Backup database regularly

## Scaling

### Horizontal Scaling

Temper is stateless except for the SQLite database. For high traffic:

1. Use PostgreSQL instead of SQLite
2. Run multiple instances behind load balancer
3. Use Redis for session/cache sharing

### Performance Tuning

```bash
# Increase file descriptors
ulimit -n 65535

# Rust optimizations (in Cargo.toml)
[profile.release]
lto = true
codegen-units = 1
```

## Docs Deployment (Mintlify)

```bash
# Install Mintlify CLI
npm i -g mintlify

# Preview locally
cd docs
mintlify dev

# Deploy
mintlify deploy
```

Or connect to Mintlify dashboard for automatic deploys on git push.
