# jojo-alpha

An AI-native trading agent that consumes [The Graph](https://thegraph.com/) subgraph data as a first-class primitive. Unlike traditional bots with fixed queries, jojo-alpha uses a **bidirectional inference loop** where LLM reasoning directs what data to fetch, and query results inform trading decisions.

## Key Innovation

```
┌─────────────────┐     ┌─────────────────┐     ┌─────────────────┐
│  LLM Inference  │────▶│   Query Plan    │────▶│  Graph Subgraph │
│  "What pools    │     │  • Networks     │     │  Uniswap V3     │
│   should I      │     │  • Protocols    │     │  Aave, Curve... │
│   analyze?"     │     │  • Filters      │     │                 │
└─────────────────┘     └─────────────────┘     └─────────────────┘
        ▲                                               │
        │                                               │
        └───────────────────────────────────────────────┘
                    Results inform next query
```

The agent doesn't know what data it needs until it starts reasoning. This creates adaptive, multi-round query patterns that traditional rule-based bots can't match.

## Features

- **Inference-Guided Queries** - LLM generates structured query plans, fetching only relevant data
- **Multi-Protocol Support** - Uniswap V3 across Ethereum, Arbitrum, Base, Optimism, Polygon (extensible to Aave, Curve)
- **Paper Trading Mode** - Develop strategies without capital risk
- **Formal Safety Invariants** - Private key isolation, spend limits, slippage guards, audit logging
- **Sandbox Execution** - TypeScript agent runs in QuickJS with no direct network/filesystem access

## Architecture

```
TypeScript Agent (QuickJS sandbox)
         ↓
    BAML Functions (LLM inference)
         ↓
    Interceptor Pipeline (risk controls)
         ↓
    Rust Tools (TheGraphTool, OdosTool)
         ↓
    SecureWallet (transaction signing)
```

## Dependencies

This project depends on the upstream [baml-ts-sandbox](https://github.com/ryan-s-roberts/baml-ts-sandbox) workspace, which provides the `baml-rt` facade crate plus A2A, observability, provenance, and the agent builder/runner toolchain.

## Quick Start

```bash
# Build
cargo build --release

# Run in paper trading mode (no real funds)
cargo run -- run --agent ./agent --dry-run

# Query The Graph directly
cargo run -- query --protocol uniswap_v3 --network ethereum --query-type top_pools

# Get a swap quote
cargo run -- quote --input <token> --output <token> --amount <wei>
```

## Environment Variables

| Variable | Required | Purpose |
|----------|----------|---------|
| `GRAPH_API_KEY` | For queries | [The Graph API key](https://thegraph.com/studio/) |
| `OPENROUTER_API_KEY` | For trading | LLM inference via OpenRouter |
| `PRIVATE_KEY` | Optional | Wallet key (hex, with or without 0x) |
| `BAML_QJS_MEMORY_LIMIT_BYTES` | Optional | Cap QuickJS memory usage (bytes) |
| `BAML_QJS_MAX_STACK_BYTES` | Optional | Cap QuickJS stack size (bytes) |
| `BAML_QJS_GC_THRESHOLD` | Optional | GC threshold for QuickJS allocations |
| `BAML_QJS_GC_INTERVAL_SECS` | Optional | Periodic full GC interval |

### BAML Runtime Showcase (Upstream Features)

This repo is meant to be a demo/template for the baml-ts-sandbox runtime. The upstream workspace ships a full toolchain and observability stack you can use alongside this project:

- **Agent builder**: lint, typegen, compile TS, and package agents.
- **Agent runner**: load packaged agents and serve A2A requests.
- **A2A protocol + eventing**: structured agent-to-agent RPC with streaming.
- **Observability**: tracing setup + OpenTelemetry metrics hooks.
- **Provenance**: attach and persist execution context for replay/debug.

Example (from the upstream repo checkout):

```bash
# Build + package this agent
cd ../baml-ts-sandbox
cargo run -p baml-rt-builder --bin baml-agent-builder -- \
  --agent-path ../defi-trading-agent/agent \
  --out ./agent.tar.gz

# Run the packaged agent with A2A support
cargo run -p baml-agent-runner --bin baml-agent-runner -- \
  --agent ./agent.tar.gz
```

Or use the helper script:

```bash
./scripts/demo_baml_runtime.sh
```

### A note on private key management

The `PRIVATE_KEY` environment variable is convenient for development and paper trading, but **not recommended for production** with significant funds. Better options:

- **Hardware wallets** (Ledger, Trezor) via WalletConnect or similar
- **Cloud KMS** (AWS KMS, GCP Cloud HSM, Azure Key Vault) for server-side signing
- **Dedicated signing services** (Fireblocks, Fordefi) for institutional setups
- **Multisig** (Safe) for additional authorization controls

The current implementation isolates the key within `SecureWallet` and never logs or serializes it, but defense-in-depth means the key ideally shouldn't be on the same machine running the agent.

## Safety Model

Private keys never leave the `SecureWallet` module. All trades pass through an interceptor pipeline:

1. **Spend Limit Guard** - Daily and per-trade caps
2. **Slippage Guard** - Price impact limits
3. **Cooldown Guard** - Rate limiting
4. **Audit Logger** - Compliance trail

## Development

```bash
# Enter Nix dev shell
nix develop

# Run tests
cargo test

# Type check
cargo check
```

## License

MIT
