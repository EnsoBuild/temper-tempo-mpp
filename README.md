# Temper

**Temper is an Ethereum Transaction Simulator. It provides a super simple HTTP API which simulates a given transaction request against a local EVM.**

[![test](https://github.com/EnsoFinance/temper/actions/workflows/test.yaml/badge.svg)](https://github.com/EnsoFinance/temper/actions/workflows/test.yaml)

![cover](cover.jpg)

## Features

Temper supports two mutually exclusive backends via feature flags:

- **`ethereum`** (default): Standard EVM simulation for Ethereum and EVM-compatible chains (Arbitrum, Optimism, Base, Polygon, BSC, etc.)
- **`tempo`**: Tempo chain simulation with TIP-20 precompile support for native stablecoins

### Tempo Chain Features

When built with the `tempo` feature, Temper supports:

- **TIP-20 Precompiles**: Native stablecoin addresses (`0x20c0...`) are automatically handled with proper storage access
- **Batch Transactions**: Execute multiple calls in a single transaction (Tempo type 0x76)
- **Fee Sponsorship**: Allow a `feePayer` to cover gas costs for the sender

## Building

```bash
# Build with ethereum feature (default)
cargo build

# Build with tempo feature
cargo build --no-default-features --features tempo
```

## Testing

```bash
# Run ethereum tests
cargo test --no-default-features --features ethereum

# Run tempo tests
cargo test --no-default-features --features tempo
```

## API

### POST /api/v1/simulate

Simulates a single transaction against a local EVM.

[See the full request and response types below.](#types)

Example body:

```json
{
  "chainId": 1,
  "from": "0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045",
  "to": "0x66fc62c1748e45435b06cf8dd105b73e9855f93e",
  "data": "0xffa2ca3b...",
  "gasLimit": 500000,
  "value": "100000",
  "blockNumber": 16784600
}
```

Example response:

```json
{
  "gasUsed": 214622,
  "blockNumber": 16784600,
  "success": true,
  "trace": { ... },
  "logs": [ ... ],
  "exitReason": "Return"
}
```

Notes:

- `blockNumber` can be omitted and the latest block will be used, however providing a `blockNumber` is recommended where possible to use the cache.

### POST /api/v1/simulate (Tempo Batch Transactions)

When using the `tempo` feature, you can simulate batch transactions with optional fee sponsorship:

```json
{
  "chainId": 42429,
  "from": "0x1234567890123456789012345678901234567890",
  "gasLimit": 500000,
  "feePayer": "0xAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
  "calls": [
    {
      "to": "0x20c0000000000000000000000000000000000000",
      "data": "0x70a08231..."
    },
    {
      "to": "0x20c0000000000000000000000000000000000001",
      "data": "0xa9059cbb...",
      "value": "0"
    }
  ]
}
```

Response includes individual call results:

```json
{
  "gasUsed": 85000,
  "blockNumber": 12345,
  "success": true,
  "callResults": [
    { "success": true, "gasUsed": 25000, "returnData": "0x...", "exitReason": "Return" },
    { "success": true, "gasUsed": 60000, "returnData": "0x...", "exitReason": "Return" }
  ]
}
```

### POST /api/v1/simulate-bundle

Simulates a bundle of transactions in order against the same EVM.

[See the full request and response types below.](#types)

Example body:

```json
[
  {
    "chainId": 1,
    "from": "0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045",
    "to": "0x66fc62c1748e45435b06cf8dd105b73e9855f93e",
    "data": "0xffa2ca3b...",
    "gasLimit": 500000,
    "value": "100000",
    "blockNumber": 16784600
  }
]
```

Example response:

```json
[{
  "gasUsed": 214622,
  "blockNumber": 16784600,
  "success": true,
  "trace": { ... },
  "logs": [ ... ],
  "exitReason": "Return"
}]
```

Notes:

- `chainId` must be the same in all transactions.
- `blockNumber` can be included and incremented when a multi-block simulation is required, or omitted in all transactions to use latest.

### POST /api/v1/simulate-stateful

Starts a new stateful simulation, allowing you to persist the state of a single EVM across multiple subsequent simulation requests.

[See the full request and response types below.](#types)

Example body:

```json
[
  {
    "chainId": 1,
    "gasLimit": 500000,
    "blockNumber": 16784600
  }
]
```

Example response:

```json
[{
  "statefulSimulationId": "aeb708a5-81d7-4126-a0b5-0f2a78b3830e",
}]
```


### POST /api/v1/simulate-stateful/{statefulSimulationId}

Simulates a bundle of transactions in order against the EVM referred to by the UUID in the URL. After the result is obtained, the EVM state will be retained for subsequent requests.

[See the full request and response types below.](#types)

Example body:

```json
[
  {
    "chainId": 1,
    "from": "0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045",
    "to": "0x66fc62c1748e45435b06cf8dd105b73e9855f93e",
    "data": "0xffa2ca3b...",
    "gasLimit": 500000,
    "value": "100000",
    "blockNumber": 16784600
  }
]
```

Example response:

```json
[{
  "gasUsed": 214622,
  "blockNumber": 16784600,
  "success": true,
  "trace": { ... },
  "logs": [ ... ],
  "exitReason": "Return"
}]
```
Notes:

- `chainId` must be the same in all transactions.
- `blockNumber` can be included and incremented when a multi-block simulation is required, or omitted in all transactions to use latest.


### DELETE /api/v1/simulate-stateful/{statefulSimulationId}

Ends a current stateful simulation, freeing associated memory.

[See the full request and response types below.](#types)

Example response:

```json
{
  "success": true
}
```



### Authentication

If you set an `API_KEY` environment variable then all calls to the API must be accompanied by a `X-API-KEY` header which contains this API Key.

## Running

### Locally

Copy `.env.example` to `.env`, fill out required values and run:

```bash
# Run with ethereum feature (default)
cargo run

# Run with tempo feature
cargo run --no-default-features --features tempo
```

If you want the server to restart on any code changes run:

```bash
cargo watch -x run
```

### Manual Testing

`body.json` contains a simple request in the root of the project so once the API is running you can just run:

```bash
curl -H "Content-Type: application/json" --data @tests/body.json http://localhost:8080/api/v1/simulate
```

If you have `jq` installed, you can run this to see pretty traces:

```bash
curl -H "Content-Type: application/json" --data @tests/body.json http://localhost:8080/api/v1/simulate | jq -r ".formattedTrace"
```

## Supported Chains

### Ethereum Feature
- Ethereum Mainnet (1)
- Arbitrum (42161)
- Optimism (10)
- Base (8453)
- Polygon (137)
- BSC (56)
- Gnosis (100)
- Avalanche C-Chain (43114)
- Linea (59144)
- HyperEVM (999)

### Tempo Feature
- Tempo Testnet (42429)

## Environment Variables

Configure RPC endpoints in `.env`:

```bash
# Ethereum chains
RPC_URL_1=https://eth-mainnet.g.alchemy.com/v2/YOUR_KEY
RPC_URL_42161=https://arb-mainnet.g.alchemy.com/v2/YOUR_KEY
RPC_URL_10=https://opt-mainnet.g.alchemy.com/v2/YOUR_KEY
# ... etc

# Tempo chain
RPC_URL_42429=https://rpc.tempo.xyz
```

## Architecture

```
src/
├── evm/
│   ├── mod.rs          # Feature-gated module exports
│   ├── ethereum.rs     # Standard foundry EVM backend
│   └── tempo.rs        # Tempo-foundry EVM backend with TIP-20 support
├── simulation.rs       # Simulation logic and API handlers
├── tip20.rs           # TIP-20 precompile bytecode and utilities
├── config.rs          # Configuration and environment handling
├── errors.rs          # Error types and rejection handling
└── lib.rs             # Library exports and route setup
```

## Roadmap

- [x] Support any RPC endpoint, not just Alchemy
- [x] Tempo chain support with TIP-20 precompiles
- [x] Batch transaction simulation
- [x] Fee sponsorship simulation
- [ ] Connect to local node via IPC
- [ ] Connect to local [reth](https://github.com/paradigmxyz/reth/) DB

### Contributing

[See CONTRIBUTING.md](CONTRIBUTING.md).

## Types

```typescript
export type SimulationRequest = {
  chainId: number;
  from: string;
  to?: string;           // Optional for batch transactions
  data?: string;
  gasLimit: number;
  value?: string;
  accessList?: AccessListItem[];
  blockNumber?: number;
  blockTimestamp?: number;
  stateOverrides?: Record<string, StateOverride>;
  formatTrace?: boolean;
  // Tempo-specific fields (tempo feature only)
  calls?: TempoCall[];   // Batch calls
  feePayer?: string;     // Fee sponsor address
  feeToken?: string;     // ERC20 token for gas payment
};

export type TempoCall = {
  to: string;
  data?: string;
  value?: string;
};

export type AccessListItem = {
  address: string;
  storageKeys: string[];
};

export type StateOverride = {
  balance?: string;
  nonce?: number;
  code?: string;
  state?: Record<string, string>;
  stateDiff?: Record<string, string>;
};

export type SimulationResponse = {
  simulationId: string;
  gasUsed: number;
  blockNumber: number;
  success: boolean;
  trace: CallTrace[];
  logs?: Log[];
  exitReason?: InstructionResult;
  returnData: string;
  formattedTrace?: string;
  callResults?: CallResult[];  // For batch transactions
};

export type CallResult = {
  success: boolean;
  gasUsed: number;
  returnData: string;
  exitReason: InstructionResult;
  logs: Log[];
};

export type Log = {
  topics: string[];
  data: string;
  address: string;
};

export type CallTrace = {
  callType: CallType;
  from: string;
  to: string;
  value: string;
};

export enum CallType {
  CALL,
  STATICCALL,
  CALLCODE,
  DELEGATECALL,
  CREATE,
  CREATE2,
}

export enum InstructionResult {
  //success codes
  Continue,
  Stop,
  Return,
  SelfDestruct,

  // revert code
  Revert,
  CallTooDeep,
  OutOfFund,

  // error codes
  OutOfGas,
  OpcodeNotFound,
  CallNotAllowedInsideStatic,
  InvalidOpcode,
  InvalidJump,
  InvalidMemoryRange,
  NotActivated,
  StackUnderflow,
  StackOverflow,
  OutOfOffset,
  FatalExternalError,
  GasMaxFeeGreaterThanPriorityFee,
  PrevrandaoNotSet,
  GasPriceLessThenBasefee,
  CallerGasLimitMoreThenBlock,
  RejectCallerWithCode,
  LackOfFundForGasLimit,
  CreateCollision,
  OverflowPayment,
  PrecompileError,
  NonceOverflow,
  CreateContractLimit,
  CreateContractWithEF,
}
```

## Thanks

 - Leverages crates from [Foundry](https://github.com/foundry-rs/foundry)
 - Uses [tempo-foundry](https://github.com/tempoxyz/tempo-foundry) for Tempo chain support
 - Inspired by [gakonst's example pyrevm](https://github.com/gakonst/pyrevm)
