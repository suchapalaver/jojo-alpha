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
