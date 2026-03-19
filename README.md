# Temper

**EVM transaction simulation API with a built-in trace viewer, debugger, and machine-to-machine payments via [MPP](https://mpp.dev).**

[![test](https://github.com/EnsoFinance/temper/actions/workflows/test.yaml/badge.svg)](https://github.com/EnsoFinance/temper/actions/workflows/test.yaml)

![cover](cover.jpg)

## Features

- **Simulate any EVM transaction** — fork any chain at any block, execute transactions, get traces and logs
- **Transaction hash replay** — submit a tx hash to replay and inspect historical transactions
- **Built-in trace viewer** — web UI at `/simulation/{id}` with decoded function calls, address labels, and balance changes
- **Step-by-step debugger** — source-level debugger at `/debug` with opcode stepping, stack/memory inspection, and Solidity source mapping
- **MPP payment gating** — charge for simulation access using [Machine Payments Protocol](https://mpp.dev) with Tempo TIP-20 stablecoins (first 10 requests free)
- **Tempo chain support** — TIP-20 precompiles, batch transactions (0x76), fee sponsorship

### Supported Chains

| Chain | ID | Feature |
|-------|-----|---------|
| Ethereum | 1 | `ethereum` |
| Arbitrum | 42161 | `ethereum` |
| Optimism | 10 | `ethereum` |
| Base | 8453 | `ethereum` |
| Polygon | 137 | `ethereum` |
| BSC | 56 | `ethereum` |
| Avalanche | 43114 | `ethereum` |
| **Tempo Mainnet** | **4217** | `tempo` |
| Tempo Testnet | 42429 | `tempo` |
| Tempo Moderato | 42431 | `tempo` |

## Quick Start

```bash
# 1. Clone and configure
cp .env.example .env
# Edit .env — add at least one RPC_URL_* (e.g. RPC_URL_TEMPO_MAINNET)

# 2. Build and run
cargo run --no-default-features --features tempo

# 3. Simulate a transaction
curl -X POST http://localhost:8080/api/v1/simulate \
  -H "Content-Type: application/json" \
  -d '{"chainId": 4217, "txHash": "0x97853001b7c7974d556953bc27af6a0b3d6931d313df6340a150e978754990f9"}'
```

Open `http://localhost:8080/simulation/{id}` to view the trace in the web UI.

## MPP Payment Gating

Temper supports the [Machine Payments Protocol](https://mpp.dev) — an open standard for machine-to-machine payments using HTTP 402. This lets AI agents autonomously pay for simulation access using stablecoins on Tempo.

### How it works

1. **First 10 requests per IP are free**
2. Request #11 returns `402 Payment Required` with an MPP challenge:
   ```
   WWW-Authenticate: Payment id="...", method="tempo", intent="charge",
     request="<base64url: {amount, currency, recipient, chainId}>"
   ```
3. Client pays on-chain (TIP-20 transfer on Tempo) and sends proof:
   ```
   Authorization: Payment <base64url credential with tx hash>
   ```
4. Server verifies the Transfer event on-chain and returns the simulation

### Using with the Tempo CLI

```bash
# Install tempo CLI
curl -fsSL https://tempo.xyz/install | bash

# Login and fund wallet
tempo wallet login
tempo wallet fund

# Make a paid request (tempo CLI auto-handles the 402 flow)
tempo request -t -X POST \
  --json '{"chainId":4217,"txHash":"0x978..."}' \
  http://localhost:8080/api/v1/simulate
```

### Configuration

| Variable | Default | Description |
|----------|---------|-------------|
| `MPP_ENABLED` | `false` | Enable MPP payment gating on `/simulate` and `/simulate-bundle` |
| `MPP_RECIPIENT` | — | Address that receives payments (required if enabled) |
| `MPP_AMOUNT` | `10000` | Price per simulation in smallest unit (10000 = 0.01 USDC.e) |
| `MPP_CURRENCY` | `0x20c0...b9537d11c60e8b50` | TIP-20 token address (default: USDC.e on Tempo) |
| `MPP_SECRET` | — | HMAC secret for challenge ID generation |

Only `POST /simulate` and `POST /simulate-bundle` are gated. GET endpoints, the web UI, and the debugger remain free.

## Web UI

### Simulation Viewer (`/simulation/{id}`)

Displays transaction overview with:
- Sender/recipient with resolved labels (contract names, token symbols)
- Call trace tree with decoded function signatures and parameters
- Balance changes (token in/out)
- Links to block explorers

### Debugger (`/debug?tx={hash}&chain={id}`)

Step-by-step EVM debugger with:
- Solidity source code mapping (from Sourcify/Etherscan)
- Current line highlighting
- Stack, memory, and storage inspection
- Opcode-level stepping

### Address Resolution

The trace viewer resolves addresses using multiple sources:

| Priority | Type | Source | Display |
|----------|------|--------|---------|
| 1 | System | Hardcoded | *Entrypoint*, *Fee Collector* |
| 2 | TIP-20 token | [tokenlist.tempo.xyz](https://tokenlist.tempo.xyz) | `pathUSD`, `USDC.e` |
| 3 | Verified contract | [contracts.tempo.xyz](https://contracts.tempo.xyz), Sourcify | `OPMET`, `UniswapV3Pool` |
| 4 | ABI (unverified) | [explore.tempo.xyz](https://explore.tempo.xyz) (whatsabi) | Function decoding |
| 5 | Unresolved | — | `0x1234…abcd` |

All addresses link to the appropriate block explorer.

## API Reference

### POST /api/v1/simulate

Simulate a single transaction or replay a historical one by tx hash.

```bash
# Direct simulation
curl -X POST http://localhost:8080/api/v1/simulate \
  -H "Content-Type: application/json" \
  -d '{
    "chainId": 1,
    "from": "0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045",
    "to": "0x66fc62c1748e45435b06cf8dd105b73e9855f93e",
    "data": "0xffa2ca3b...",
    "gasLimit": 500000,
    "value": "100000"
  }'

# Replay by tx hash
curl -X POST http://localhost:8080/api/v1/simulate \
  -H "Content-Type: application/json" \
  -d '{"chainId": 4217, "txHash": "0x978..."}'
```

### POST /api/v1/simulate-bundle

Simulate multiple transactions in sequence against the same EVM state.

### POST /api/v1/simulate-stateful

Create a persistent EVM session for multi-step simulations.

### GET /api/v1/simulation/{id}

Retrieve a stored simulation result.

### POST /api/v1/debug

Get step-by-step debug trace with source mapping.

### GET /api/v1/contract/{chainId}/{address}/source

Fetch verified contract source code.

## Configuration

### Required

At least one `RPC_URL_*` environment variable for the chain you want to simulate:

```bash
RPC_URL_ETHEREUM=https://eth.example.com
RPC_URL_TEMPO_MAINNET=https://rpc.tempo.xyz
```

### Optional

| Variable | Default | Description |
|----------|---------|-------------|
| `PORT` | `8080` | Server port |
| `API_KEY` | — | Require `X-API-KEY` header on all requests |
| `DATABASE_URL` | — | SQLite path for simulation storage (e.g. `sqlite:./temper.db`) |
| `STORAGE_ENABLED` | auto | Enable simulation persistence (auto if DATABASE_URL set) |
| `WEB_UI_ENABLED` | `true` | Serve web UI routes |
| `ETHERSCAN_KEY` | — | For contract source fetching |
| `BASE_URL` | `http://localhost:{PORT}` | Public URL for simulation links |

See `.env.example` for the full list.

## Building

```bash
# Ethereum (default)
cargo build

# Tempo
cargo build --no-default-features --features tempo
```

## Architecture

```
crates/
├── temper-core/        # EVM backends, simulation logic, TIP-20 precompiles
├── temper-db/          # SQLite storage layer
└── temper-server/      # HTTP server (warp)
    ├── handlers.rs     # Request handlers
    ├── routes.rs       # Route definitions
    ├── mpp_gate.rs     # MPP payment gating (HTTP 402 flow)
    ├── debugger.rs     # Step-by-step debug trace with source mapping
    ├── trace_fetcher.rs # RPC trace + receipt fetching
    ├── contract_cache.rs # Contract source caching
    └── storage.rs      # Simulation persistence

web/
├── script.js           # Simulation viewer (trace rendering, address resolution)
├── debugger.js         # EVM debugger UI
├── style.css           # Tempo explorer design system
└── index.html          # Landing page
```

## Thanks

- [Foundry](https://github.com/foundry-rs/foundry) — EVM implementation
- [tempo-foundry](https://github.com/tempoxyz/tempo-foundry) — Tempo chain support
- [MPP](https://mpp.dev) — Machine Payments Protocol by Tempo and Stripe
- [tokenlist.tempo.xyz](https://tokenlist.tempo.xyz) — TIP-20 token metadata
- [contracts.tempo.xyz](https://contracts.tempo.xyz) — Tempo contract verification (Sourcify)
