# CLAUDE.md

This file provides guidance to Claude Code when working with code in this repository.

## Project Overview

**DeFi Trading Agent** - An AI-powered trading agent that queries The Graph for DeFi pool data, uses LLMs (via BAML) to infer trading strategy, and executes swaps through Odos DEX aggregator.

### Architecture

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

### Security Model

- **Private keys** never leave `src/wallet/signer.rs` (protected by `secrecy` crate)
- **TypeScript agent** runs sandboxed - no filesystem, network, or key access
- **All trades** pass through interceptor pipeline before execution
- **Audit logging** captures all operations for compliance

## Build Commands

```bash
# Build
cargo build --release

# Check compilation
cargo check

# Run tests
cargo test

# Run CLI
cargo run -- --help
cargo run -- query --protocol uniswap_v3 --network ethereum --query-type top_pools
cargo run -- quote --input <token> --output <token> --amount <wei>
cargo run -- run --agent ./agent
```

## Key Files

| File | Purpose |
|------|---------|
| `src/tools/the_graph.rs` | Queries Uniswap V3 subgraphs via The Graph |
| `src/tools/odos.rs` | Wraps odos-sdk for DEX quotes and swaps |
| `src/interceptors/*.rs` | Risk controls (spend limits, slippage, cooldown) |
| `src/wallet/signer.rs` | Secure private key management |
| `agent/baml_src/strategy.baml` | LLM strategy functions (InferStrategy, AnalyzeTrade) |
| `agent/src/index.ts` | TypeScript trading loop |

## Dependencies

- **baml-rt** (local): BAML runtime from `../baml-ts-sandbox`
- **odos-sdk** (git): Odos DEX aggregator SDK
- **graphql_client**: Compile-time typed GraphQL queries
- **alloy**: Ethereum primitives and signing
- **secrecy**: Private key protection

## Environment Variables

- `GRAPH_API_KEY`: The Graph decentralized network API key (required for subgraph queries)
- `OPENROUTER_API_KEY`: For LLM inference via OpenRouter
- `PRIVATE_KEY`: Wallet private key (hex, optional 0x prefix)

Get a Graph API key at: https://thegraph.com/studio/

## Testing Notes

- **No Odos testnet**: Use mainnet quotes (read-only) or mainnet fork via Anvil
- **Graph queries**: Can test against real subgraphs (read-only)
- **Full E2E**: Requires mainnet with small amounts + strict risk limits
