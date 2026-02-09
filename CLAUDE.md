# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

**DeFi Trading Agent** - An AI-powered trading agent that uses a bidirectional inference loop: LLM reasoning (via BAML) directs which on-chain data to fetch from The Graph, and query results inform trading decisions executed through Odos DEX aggregator.

## Architecture

```
TypeScript Agent (QuickJS sandbox)
         ↓
    BAML Functions (LLM inference)
         ↓
    Interceptor Pipeline (ordered, short-circuiting)
      1. PolicyInterceptor (allow/deny per tool from policy.json)
      2. SpendLimitInterceptor (per-trade + daily USD caps)
      3. SlippageGuardInterceptor (price impact limits)
      4. CooldownInterceptor (rate limiting between trades)
      5. AuditLogInterceptor (compliance trail)
         ↓
    Rust Tools (BamlTool trait, "defi/" namespace)
         ↓
    SecureWallet (transaction signing, key isolation)
```

The TypeScript agent runs in a QuickJS sandbox with no filesystem, network, or key access. The only escape hatch is `invokeTool()` which dispatches to Rust tools registered with the BAML manager. Interceptors are ordered and cannot be bypassed.

### Key Dependency: agent-platform

This project has **local path dependencies** to `../../semiotic-agentium/agent-platform` for `baml-rt`, `baml-rt-a2a`, `baml-rt-core`, `baml-rt-provenance`, and `baml-rt-tools`. You must have that workspace checked out locally.

## Build Commands

```bash
# Build (binary: defi-agent, crate: defi-trading-agent)
cargo build --release

# Type checking
cargo check

# Lint (run before finishing any piece of work)
cargo clippy --all-targets --all-features -- -D warnings

# Run all tests
cargo test

# Run specific test module
cargo test tools::the_graph
cargo test tools::graph_gateway
cargo test tools::odos
cargo test wallet::signer
cargo test interceptors::spend_limit

# Run a single test by name
cargo test test_name_here

# Verbose test output
cargo test -- --nocapture
```

## CLI Usage

The binary is `defi-agent` (or `cargo run --`):

```bash
cargo run -- --help
cargo run -- run --agent ./agent --dry-run
cargo run -- run --agent ./agent --paper-trading --initial-balance 10000
cargo run -- query --protocol uniswap_v3 --network ethereum --query-type top_pools
cargo run -- quote --input <token_address> --output <token_address> --amount <wei>
cargo run -- price --token <token_address>
cargo run -- simulate --to <contract> --data <hex_calldata> --network ethereum
cargo run -- config
cargo run -- -v <command>   # debug logging
```

### Telemetry Harness (second binary)

Exercises A2A handling and provenance logging:

```bash
cargo run --bin telemetry_harness -- \
  --agent ./node_archives/minimal-policy-node \
  --provenance-out ./telemetry/provenance.jsonl \
  --snapshot-out ./telemetry/snapshot.json \
  --message "telemetry harness ping"
```

## How Tools Work

Tools implement `BamlTool` (from `baml-rt-tools`) and are namespaced under `defi/`:

| Tool Name | Struct | Purpose |
|-----------|--------|---------|
| `defi/query_subgraph` | `TheGraphTool` | Query Uniswap V3 subgraphs via The Graph |
| `defi/odos_swap` | `OdosTool` | DEX quotes and swaps via Odos |
| `defi/wallet_balance` | `WalletTool` | Wallet balance queries |
| `defi/wallet_derive_address` | `WalletDeriveAddressTool` | Derive address (read-only) |
| `defi/wallet_sign_message` | `WalletSignMessageTool` | Sign arbitrary messages |
| `defi/wallet_sign_tx` | `WalletSignTxTool` | Sign transactions |
| `defi/paper_trading` | `PaperTradingTool` | Simulated trading without real funds |

Tools are registered in `src/runner.rs::register_tools()`. The JS agent calls `invokeTool("defi/query_subgraph", {...})` which routes through the interceptor pipeline to the Rust implementation.

### Adding a New Tool

1. Create a struct implementing `BamlTool` in `src/tools/`
2. Define input/output types with `schemars::JsonSchema` + `ts_rs::TS` derives
3. Register it in `AgentRunner::register_tools()` in `src/runner.rs`
4. Add a tool name constant in `src/tools/mod.rs`
5. Review interceptor ordering impact

## Key Modules

| Module | Purpose |
|--------|---------|
| `src/lib.rs` | Library root; re-exports `Config`, `AgentRunner`, `Error`, `Result` |
| `src/runner.rs` | Builds BAML runtime, wires interceptors, registers tools, drives QuickJS event loop |
| `src/config/` | `Config`, `RiskConfig`, `PolicySettings`, network/protocol enums, subgraph IDs |
| `src/tools/` | `BamlTool` implementations (`DefiBundle` with `defi/` namespace) |
| `src/interceptors/` | Ordered governance pipeline (policy, spend, slippage, cooldown, audit) |
| `src/wallet/` | `SecureWallet` (key isolation via `secrecy`), `TransactionSimulator` |
| `src/paper_trading/` | Paper trading state and portfolio tracking |
| `src/bin/telemetry_harness.rs` | Second binary for A2A + provenance testing |
| `agent/src/index.ts` | TypeScript trading loop (runs sandboxed in QuickJS) |

## Environment Variables

| Variable | Required | Purpose |
|----------|----------|---------|
| `GRAPH_API_KEY` | For queries | The Graph decentralized network API key |
| `OPENROUTER_API_KEY` | For trading | OpenRouter API for LLM inference |
| `PRIVATE_KEY` | Optional | Wallet private key (hex, optional 0x prefix) |

QuickJS memory tuning: `BAML_QJS_MEMORY_LIMIT_BYTES`, `BAML_QJS_MAX_STACK_BYTES`, `BAML_QJS_GC_THRESHOLD`, `BAML_QJS_GC_INTERVAL_SECS`

## Development Environment

```bash
# Enter Nix dev shell (includes Rust toolchain)
nix develop
```

## Testing Notes

- **No Odos testnet**: Use mainnet quotes (read-only) or mainnet fork via Anvil
- **Graph queries**: Can test against real subgraphs (read-only)
- **Full E2E**: Requires mainnet with small amounts + strict risk limits
- Targeted tests exist in `tools::the_graph`, `tools::graph_gateway`, `tools::odos`, `wallet::signer`, `interceptors::spend_limit`

## Safety Invariants

Private keys never leave `SecureWallet`. All tool calls pass through the interceptor pipeline in order. Quotes are read-only and must not affect spend tracking. Dry-run mode must never trigger signing or on-chain sends. See `INVARIANTS.md` for formal contracts and `AGENTS.md` for operational guidance.
