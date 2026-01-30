# AGENTS.md

Operational guidance for building and extending the DeFi trading agent.

## Mission

Build a reusable agent template that:
- Consumes The Graph protocol data as a first-class primitive.
- Uses inference to plan queries (bidirectional loop).
- Executes via Odos quote API (dry-run mode today).
- Preserves strict safety invariants.

## What This Project Means

This is not a bot; it is a template for building safe, inference-guided financial agents. The core value is the bidirectional loop: inference decides which on-chain data to fetch, and fetched data reshapes inference. The product is a repeatable, auditable agent architecture for DeFi decision-making under uncertainty.

## How to Run (Local)

Prereqs:
- Rust toolchain (or `nix develop`)
- Env vars: `GRAPH_API_KEY`, `OPENROUTER_API_KEY` (optional for dry-run), `PRIVATE_KEY` (optional)

Commands:
```bash
# Build
cargo build --release

# Run agent (dry-run)
cargo run -- run --agent ./agent --dry-run

# Query The Graph directly
cargo run -- query --protocol uniswap_v3 --network ethereum --query-type top_pools

# Get a swap quote
cargo run -- quote --input <token> --output <token> --amount <wei>
```

## How to Test

Fast path:
```bash
# Unit + integration tests
cargo test
```

Targeted tests:
```bash
# Tools
cargo test tools::the_graph
cargo test tools::graph_gateway
cargo test tools::odos

# Safety and wallet
cargo test wallet::signer
cargo test interceptors::spend_limit
```

Debug assertions in release:
```bash
RUSTFLAGS="-C debug-assertions" cargo test --release
```

## Architecture (source of truth)

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

Reference: `README.md`, `CLAUDE.md`, `INVARIANTS.md`.

## Non-Determinism Discipline

LLM output, network I/O, and blockchain data are non-deterministic. Treat them as untrusted inputs.

Rules of engagement:
- Validate every LLM output before acting (schema checks, bounds, required fields).
- Treat time as a dependency (UTC boundaries in spend limits, cache TTLs).
- Never assume ordering of async results; make code resilient to partial failures.
- Use idempotent operations where possible; avoid stateful side effects unless guarded.
- Record reasoning and inputs in audit logs to enable replay debugging.
- Avoid implicit randomness in tests; prefer fixed seeds or deterministic test helpers.

### Non-Determinism Testing (Actionable)

When you add or modify non-deterministic behavior:
- Use fixed seeds for property tests (record the seed on failure).
- Mock tool outputs in unit tests to avoid network variance.
- Pin UTC boundaries in tests that depend on dates.
- Assert ordering only when you explicitly sort results.
- Record tool inputs/outputs for replayable failures.

## BAML Runtime and QuickJS Sandbox (baml-ts-sandbox)

This runtime is the heart of the agent. It is not just a bridge, it is a safety boundary.

Expectations:
- JS is sandboxed (no filesystem, no network, no private keys).
- The only escape hatch is tool invocation through the bridge.
- Tools are strongly typed in Rust; JS receives JSON only.

Do / Do Not:
- Do keep tool input schemas strict and minimal.
- Do treat tool calls as a public API surface.
- Do keep the QuickJS event loop alive and responsive (`src/runner.rs`).
- Do add explicit validation and explainable error messages in tools.
- Do not allow direct network or filesystem access in JS.
- Do not expose secrets through logs, serialization, or tool args.

## Safety Invariants

`INVARIANTS.md` is the law. Violations are unacceptable.

High-impact invariants to preserve:
- Private key isolation (only inside `SecureWallet`).
- Spend limits (daily + per-trade).
- Interceptor ordering (risk controls cannot be bypassed).
- Sandbox isolation (QuickJS has no direct system access).

When changing relevant code, update `INVARIANTS.md` and add tests.

## Odos Integration: Quote vs Prepare Swap

- Quotes are read-only and must not affect spend tracking.
- Swaps require spend limit checks and (future) signing.
- Dry-run mode must never perform on-chain signing or sending.

## The Graph Integration

- Use query plans to minimize data, maximize relevance.
- Expect partial failures across networks; do not abort the whole plan.
- Cache with TTL; never treat stale data as fresh.

## Tooling and Interceptors

- Tools live in `src/tools/*` and implement `BamlTool`.
- Interceptors are ordered and short-circuiting (see `src/runner.rs`).
- Adding a new tool or interceptor requires:
  - Schema definition
  - Tests
  - Interceptor order review
  - Invariant impact review

## Local Playbooks (Claude Artifacts)

These slash-command playbooks live in `../btpification/commands` and are useful references:
- `graph-inference-x402.md` - bidirectional query/inference flow.
- `implement-invariants-defi-agent.md` - property tests and assertions.
- `invariant-analysis.md` - invariant discovery and documentation.
- `quickjs-promise-issue.md` - QuickJS async caveats.
- `port-ts-agent.md` - agent porting checklist.

They are not required, but they capture institutional knowledge.

## BAML Runtime Improvement Ideas

These are high-leverage areas to grow the runtime for our use case:
- Structured validation hooks per tool (reject malformed args early).
- Better async error surfacing in QuickJS (promises should fail loudly).
- Deterministic replay mode (snapshot tool inputs/outputs).
- Visibility into tool latency for inference-aware planning.

## Change Checklist

When you touch safety-critical paths:
- Update or add invariant references in `INVARIANTS.md`.
- Add property tests (prefer `proptest`).
- Verify interceptor order is unchanged or intentionally modified.
- Confirm JS sandbox isolation is preserved.
- Note dry-run behavior explicitly if execution paths change.

## Docs and Entry Points

- `README.md` - product overview and quick start.
- `CLAUDE.md` - current maintainer notes.
- `INVARIANTS.md` - formal safety contracts.
- `src/runner.rs` - sandbox wiring + interceptor order.
- `agent/src/index.ts` - agent control loop.

## Strengths, Weaknesses, Trade-offs

Strengths:
- Bidirectional inference loop yields adaptive data collection.
- Clear safety pipeline with interceptor ordering.
- QuickJS sandbox limits blast radius of LLM behavior.
- Modular tools and schemas make auditing tractable.

Weaknesses:
- LLM decisions are non-deterministic and can be brittle.
- Reliance on external APIs (Graph, Odos, OpenRouter) adds latency and fragility.
- Limited observability of QuickJS failures without explicit instrumentation.
- Dry-run mode can mask real-world execution issues.

Trade-offs:
- Safety vs. speed: strict checks reduce risk but add latency.
- Flexibility vs. determinism: inference makes behavior adaptive but harder to reproduce.
- Abstraction vs. transparency: tools hide details, which can obscure root causes.

## Practical Risks (What Breaks First)

- QuickJS async loop stalls → tools never resolve.
- Tool schema drift between JS and Rust → silent misbehavior.
- Partial Graph query failures → skewed strategy inference.
- Spend limit edge cases around UTC rollover if time is mocked poorly.

## Roadmap Hotspots (High Leverage)

- Deterministic replay mode (log tool inputs/outputs).
- Strict schema validation hooks per tool.
- Better async error surfacing from QuickJS to host.
- Metrics for query success rate and tool latency.

## Action Runbook

Use this when you need to make changes quickly and safely.

### When to Use
- You touch `src/runner.rs`, `src/tools/*`, `src/interceptors/*`, or `src/wallet/*`.
- You change any agent loop behavior in `agent/src/index.ts`.
- You modify schemas or tool inputs.
- You change baml-rt / QuickJS behavior or versions.

### Inputs → Outputs

Inputs:
- A concrete change request (feature, bugfix, refactor).
- The affected files and invariants.

Outputs:
- A small, reviewed diff with invariant alignment.
- Updated docs or tests if safety paths changed.
- Verified tests or a clear rationale for deferring them.

### Steps (Do This Next)
1) Identify affected invariants in `INVARIANTS.md` and note them in your plan.
2) Verify interceptor ordering in `src/runner.rs` if tool/execution flow changes.
3) Validate tool input schemas; reject malformed args early.
4) Keep QuickJS isolation intact (no new escape hatches).
5) Update or add tests: unit + property tests for safety logic.
6) Run targeted tests or explain why not.

### Done When
- Invariants are still satisfied and documented.
- Tests for the touched safety path exist or are explicitly deferred.
- Dry-run behavior is unchanged unless intentionally updated.
- No secrets or network/file access are exposed to JS.

### Escalate If
- A change would weaken sandbox boundaries or key isolation.
- You need to bypass or reorder interceptors.
- You are unsure if a new tool call can affect spend limits.
