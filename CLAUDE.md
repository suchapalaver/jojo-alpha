# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

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
# Build (binary name is defi-agent, crate is defi-trading-agent)
cargo build --release

# Type checking
cargo check

# Run all tests
cargo test

# Run specific test module
cargo test tools::the_graph
cargo test wallet::signer

# Run a single test by name
cargo test test_name_here

# Verbose test output
cargo test -- --nocapture
```

## CLI Usage

The binary is `defi-agent` (or `cargo run --`):

```bash
# Show help
cargo run -- --help

# Run the trading agent
cargo run -- run --agent ./agent
cargo run -- run --agent ./agent --dry-run

# Query The Graph subgraphs
cargo run -- query --protocol uniswap_v3 --network ethereum --query-type top_pools
cargo run -- query --protocol uniswap_v3 --network ethereum --query-type token_price \
  --params '{"token_address":"0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2"}'

# Get a swap quote from Odos
cargo run -- quote --input <token_address> --output <token_address> --amount <wei>

# Show configuration
cargo run -- config

# Enable debug logging
cargo run -- -v <command>
```

## Key Files

| File | Purpose |
|------|---------|
| `src/main.rs` | CLI entry point (`defi-agent` binary) |
| `src/runner.rs` | AgentRunner - loads agent, builds BAML runtime, registers tools |
| `src/tools/the_graph.rs` | Queries Uniswap V3 subgraphs via The Graph |
| `src/tools/odos.rs` | Wraps odos-sdk for DEX quotes and swaps |
| `src/interceptors/*.rs` | Risk controls (spend limits, slippage, cooldown, audit) |
| `src/wallet/signer.rs` | Secure private key management |
| `agent/baml_src/strategy.baml` | LLM strategy functions (InferStrategy, AnalyzeTrade) |
| `agent/src/index.ts` | TypeScript trading loop |

## Dependencies

- **baml-rt** (local path: `../baml-ts-sandbox`): BAML runtime for LLM function execution
- **odos-sdk**: Odos DEX aggregator SDK
- **graphql_client**: Compile-time typed GraphQL queries
- **alloy**: Ethereum primitives and signing
- **secrecy**: Private key protection

## Environment Variables

| Variable | Required | Purpose |
|----------|----------|---------|
| `GRAPH_API_KEY` | For queries | The Graph decentralized network API key |
| `OPENROUTER_API_KEY` | For trading | OpenRouter API for LLM inference |
| `PRIVATE_KEY` | Optional | Wallet private key (hex, optional 0x prefix) |

Get a Graph API key at: https://thegraph.com/studio/

## Development Environment

This project uses Nix flakes for reproducible development:

```bash
# Enter development shell (includes Rust toolchain)
nix develop

# Or use direnv with .envrc
```

## Testing Notes

- **No Odos testnet**: Use mainnet quotes (read-only) or mainnet fork via Anvil
- **Graph queries**: Can test against real subgraphs (read-only)
- **Full E2E**: Requires mainnet with small amounts + strict risk limits
- Unit tests exist in `src/tools/the_graph.rs` and `src/wallet/signer.rs`
