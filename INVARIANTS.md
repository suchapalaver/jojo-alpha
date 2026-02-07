# Invariant Analysis: DeFi Trading Agent

This document identifies and formalizes the critical invariant properties that must hold across all subsystems of the DeFi trading agent. These invariants represent the "laws that cannot be broken" and inform both code comments and property-based testing.

## Overview

The DeFi trading agent is a multi-layered system with critical security and correctness guarantees:

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

Each layer enforces specific invariants that protect user funds and ensure correct operation.

---

## 1. Private Key Isolation Invariant

**Property:**
```
∀ operation op, ∀ data structure d:
  IF op accesses private_key
  THEN op ∈ {sign_hash, sign_transaction} AND op ∈ SecureWallet module
  AND private_key ∉ serialized_data
  AND private_key ∉ log_output
  AND private_key ∉ QuickJS_sandbox
```

The private key must never leave the `SecureWallet` module and must never be serialized, logged, or exposed to the JavaScript sandbox.

**Enforcement:**

| Layer | Mechanism |
|-------|-----------|
| **Type System** | `SecureWallet` has no `Serialize` impl; `signer` field is private |
| **Code Review** | Only `sign_hash()` and `wallet()` methods expose signing capability |
| **Debug** | `Debug` impl redacts signer: `"[REDACTED]"` |
| **Sandbox** | QuickJS has no access to Rust memory; tools only receive addresses |
| **Testing** | `test_debug_redacts_key` verifies key never appears in debug output |

**Code Location:** `src/wallet/signer.rs:34-123`

**Violation Impact:** CRITICAL - Complete loss of funds if key is exposed.

---

## 2. Spend Limit Conservation Invariant

**Property:**
```
∀ trade t, ∀ time T:
  IF t.executed_at = T
  THEN daily_spent(T) = Σ(trades WHERE date = T.date)
  AND daily_spent(T) <= max_daily
  AND t.value_usd <= max_per_trade
```

The daily spending tracker must accurately sum all executed trades and never exceed configured limits.

**Enforcement:**

| Layer | Mechanism |
|-------|-----------|
| **Application** | `DailySpending::add()` atomically increments total; `current_total()` resets on date change |
| **Interceptor** | `SpendLimitInterceptor::intercept_tool_call()` checks limits before execution |
| **Post-Execution** | `on_tool_call_complete()` only updates tracker if `result.is_ok()` |
| **Date Handling** | `current_total()` checks `date_naive()` to reset on day boundary |
| **Testing** | Unit tests verify limit enforcement and daily reset behavior |

**Code Location:** `src/interceptors/spend_limit.rs:18-269`

**Violation Impact:** HIGH - Uncontrolled spending could drain wallet.

**Edge Cases:**
- **Date boundary**: Daily total resets at midnight UTC (`date_naive()` comparison)
- **Unknown tokens**: Mode-dependent (fail-open vs fail-closed)
- **Failed trades**: Only successful `prepare_swap` operations update tracker

---

## 3. Interceptor Pipeline Ordering Invariant

**Property:**
```
∀ tool_call tc:
  tc MUST pass through interceptors in order:
  1. PolicyInterceptor (policy allow/deny)
  2. SpendLimitInterceptor (funds check)
  3. SlippageGuardInterceptor (price impact check)
  4. CooldownInterceptor (rate limiting)
  5. AuditLogInterceptor (logging)
  
  AND ∀ interceptor i:
    IF i.intercept_tool_call(tc) = Block(reason)
    THEN tc is NOT executed
    AND subsequent interceptors are NOT called
```

All tool calls must pass through the interceptor pipeline in a fixed order, and any interceptor can block execution.

**Enforcement:**

| Layer | Mechanism |
|-------|-----------|
| **Runtime** | `RuntimeBuilder` registers interceptors in order; BAML runtime calls them sequentially |
| **Early Return** | `InterceptorDecision::Block` short-circuits pipeline |
| **Read-Only Operations** | Quotes (`action = "quote"`) bypass spend limit checks |
| **Testing** | Integration tests verify interceptor ordering and blocking behavior |

**Code Location:** `src/runner.rs:198-245` (interceptor registration)

**Violation Impact:** HIGH - Risk controls could be bypassed.

---

## 3.1 Policy Enforcement Invariant

**Property:**
```
∀ tool_call tc:
  IF policy.json exists AND policy decision for tc.tool_name = deny
  THEN tc is blocked with an explainable reason
  AND tool implementation is never invoked
```

Policy enforcement must be evaluated before risk controls so explicit deny rules
cannot be bypassed by downstream checks.

**Enforcement:**

| Layer | Mechanism |
|-------|-----------|
| **Interceptor** | `PolicyInterceptor` loads `policy.json` and blocks denied tools |
| **Explainability** | Block reason includes policy rule id and reason when present |
| **Fallback** | Missing `policy.json` defaults to allow-all for backward compatibility |
| **Testing** | Unit tests cover allow/deny decisions and tool name validation |

**Code Location:** `src/interceptors/policy.rs`

**Violation Impact:** HIGH - Policies could be ignored or misapplied.

---

## 4. Paper Trading Balance Conservation Invariant

**Property:**
```
∀ paper_swap ps:
  IF ps executed successfully
  THEN portfolio.holdings[ps.input_token] = old_holdings[ps.input_token] - ps.input_amount
  AND portfolio.holdings[ps.output_token] = old_holdings[ps.output_token] + ps.expected_output
  AND Σ(token_balance * token_price_usd) = portfolio.total_value_usd
  AND portfolio.holdings[ps.input_token] >= 0 (before swap)
```

Paper trading portfolio balances must be conserved: input tokens decrease, output tokens increase, and total value is consistent.

**Enforcement:**

| Layer | Mechanism |
|-------|-----------|
| **Application** | `PaperPortfolio::execute_swap()` checks balance before deducting; updates both tokens atomically |
| **Balance Check** | `current_balance < input_amount` check prevents negative balances |
| **Zero Removal** | Zero balances are removed from `holdings` map |
| **Price Updates** | Both input and output token prices updated in `prices` map |
| **Testing** | Unit tests verify balance conservation and insufficient balance rejection |

**Code Location:** `src/paper_trading/portfolio.rs:110-200`

**Violation Impact:** MEDIUM - Incorrect P&L tracking, misleading metrics.

**Edge Cases:**
- **Zero balance**: Removed from `holdings` map (not stored as `U256::ZERO`)
- **Insufficient balance**: Swap rejected before any state changes
- **Price updates**: Both tokens' prices updated even if one is new

---

## 5. Query Plan Execution Atomicity Invariant

**Property:**
```
∀ query_plan qp:
  IF qp executed
  THEN ∀ network n ∈ qp.target_networks:
    EITHER all queries for n succeed
    OR all queries for n fail
    AND partial results are NOT returned
  
  AND IF any query fails:
    THEN remaining queries may continue
    BUT error is logged
```

Query plan execution should be resilient: individual network/protocol failures don't abort the entire plan, but partial results are clearly marked.

**Enforcement:**

| Layer | Mechanism |
|-------|-----------|
| **Application** | `TheGraphTool::execute_query_plan()` iterates networks/protocols independently |
| **Error Handling** | `match` statement logs errors but continues with other queries |
| **Result Structure** | Results array contains per-network entries; failures are logged, not returned |
| **Testing** | Integration tests verify partial failure handling |

**Code Location:** `src/tools/the_graph.rs:700-800` (query plan execution)

**Violation Impact:** LOW - Degraded data quality, but system continues.

**Current Gap:** Partial results are returned even if some queries fail. Consider adding a `success_rate` field to query plan results.

---

## 6. Tool Call Sandbox Invariant

**Property:**
```
∀ JavaScript code js in QuickJS sandbox:
  js CANNOT:
    - Access filesystem
    - Make network requests (except via invokeTool)
    - Access private keys
    - Modify Rust memory directly
  
  js CAN ONLY:
    - Call registered tools via invokeTool(name, args)
    - Call BAML functions (InferStrategy, etc.)
    - Access JavaScript globals and standard library
```

The QuickJS sandbox must prevent all direct system access; tools are the only bridge to the outside world.

**Enforcement:**

| Layer | Mechanism |
|-------|-----------|
| **Runtime** | QuickJS sandbox configured with no filesystem/network access |
| **Tool Registry** | Only registered tools accessible via `invokeTool()` |
| **Type System** | Rust tools are strongly typed; JavaScript receives JSON only |
| **Testing** | E2E tests verify sandbox isolation |

**Code Location:** `src/runner.rs:325-353` (agent code loading)

**Violation Impact:** CRITICAL - Security breach if sandbox is compromised.

### Runtime Change Impact (QuickJS / BAML)

Any change to QuickJS or baml-rt must preserve the sandbox boundary and tool-only IO.

**Minimum requirements:**
- JS cannot access filesystem or network directly.
- JS cannot access or infer private keys.
- All external effects happen via tool calls only.
- Tool inputs must be validated and rejected if malformed.

**Verification checklist:**
- QuickJS bridge exposes only `invokeTool()` and approved globals.
- No new host functions are registered without review.
- Tool schemas remain strict and defensive.

---

## 7. Daily Spending Reset Invariant

**Property:**
```
∀ time T1, T2:
  IF T1.date_naive() != T2.date_naive()
  THEN daily_spent(T2) = 0
  AND daily_spent(T1) is preserved in history (if logged)
```

Daily spending must reset at midnight UTC, and the previous day's total must not affect the new day.

**Enforcement:**

| Layer | Mechanism |
|-------|-----------|
| **Application** | `DailySpending::current_total()` checks `date_naive()` and resets if different |
| **Date Comparison** | Uses `chrono::NaiveDate` comparison (timezone-independent) |
| **Atomic Reset** | `total = 0.0` and `trades.clear()` happen together |
| **Testing** | Unit tests verify date boundary reset behavior |

**Code Location:** `src/interceptors/spend_limit.rs:50-59`

**Violation Impact:** MEDIUM - Incorrect daily limit enforcement.

**Edge Case:** If system runs across midnight, `current_total()` will reset on first call of new day.

---

## 8. Quote vs Execution Separation Invariant

**Property:**
```
∀ tool_call tc:
  IF tc.action = "quote"
  THEN tc bypasses spend_limit check
  AND tc does NOT update daily_spent tracker
  AND tc does NOT require wallet signature
  
  IF tc.action = "prepare_swap"
  THEN tc MUST pass spend_limit check
  AND tc updates daily_spent on success
  AND tc requires wallet signature (if not paper trading)
```

Quotes are read-only operations that must not affect spending limits or require authentication.

**Enforcement:**

| Layer | Mechanism |
|-------|-----------|
| **Interceptor** | `SpendLimitInterceptor` checks `action != "prepare_swap"` and allows quotes |
| **Post-Execution** | `on_tool_call_complete()` only tracks `prepare_swap` actions |
| **Tool Implementation** | `OdosTool` separates quote and swap logic |
| **Testing** | `test_allows_quotes` verifies quotes bypass limits |

**Code Location:** `src/interceptors/spend_limit.rs:182-186`

**Violation Impact:** MEDIUM - Quotes incorrectly blocked or tracked.

---

## 9. Context Update Monotonicity Invariant

**Property:**
```
∀ context update u:
  IF u updates cycleCount
  THEN u.cycleCount = old_context.cycleCount + 1
  
  AND ∀ context update u:
    IF u updates queryHistory
    THEN u.queryHistory.length <= old_context.queryHistory.length + 1
    AND u.queryHistory contains old_context.queryHistory as prefix
```

Context updates must be monotonic: cycle count only increases, query history only appends.

**Enforcement:**

| Layer | Mechanism |
|-------|-----------|
| **Application** | `updateContext()` always increments `cycleCount` |
| **Query History** | New queries appended; history kept to last 10 entries |
| **Immutable Updates** | Context is cloned and updated (functional style) |
| **Testing** | Unit tests verify context update behavior |

**Code Location:** `agent/src/index.ts:130-160` (context management)

**Violation Impact:** LOW - Incorrect state tracking, but doesn't affect safety.

**Current Gap:** No explicit verification that cycle count is monotonic. Consider adding assertion.

---

## 10. Gateway Cache Consistency Invariant

**Property:**
```
∀ cached query result c:
  IF c.timestamp + c.ttl < current_time
  THEN c is considered stale
  AND new query MUST be executed (cache miss)
  
  AND ∀ query q:
    IF q matches cached query (same subgraph_id, query, variables)
    AND cache is fresh
    THEN cached result is returned
    AND no network request is made
```

Gateway cache must respect TTL and return stale results only when explicitly allowed.

**Enforcement:**

| Layer | Mechanism |
|-------|-----------|
| **Application** | `BasicGraphGateway` checks `timestamp + ttl < now` before returning cache |
| **Cache Key** | Hash of `(subgraph_id, query, variables)` used as key |
| **TTL Default** | 60 seconds default TTL |
| **Testing** | Unit tests verify cache hit/miss behavior and TTL expiration |

**Code Location:** `src/tools/graph_gateway.rs` (gateway implementation)

**Violation Impact:** LOW - Stale data, but doesn't affect safety.

---

## Recommendations

### High Priority

1. **Add Property Tests**: Encode invariants 1-4 as property tests using `proptest`:
   ```rust
   #[proptest]
   fn prop_spend_limit_conservation() {
       // Verify daily_spent always <= max_daily
   }
   ```

2. **Add Invariant Assertions**: Add runtime checks for critical invariants:
   ```rust
   // In DailySpending::add()
   debug_assert!(self.total <= MAX_REASONABLE_DAILY);
   ```

3. **Document Edge Cases**: Add explicit documentation for date boundary behavior, unknown tokens, and partial query failures.

### Medium Priority

4. **Query Plan Partial Failure Handling**: Add `success_rate` and `failed_networks` fields to query plan results.

5. **Context Monotonicity Verification**: Add assertion that `cycleCount` only increases.

6. **Gateway Cache Metrics**: Add metrics for cache hit rate and stale result detection.

### Low Priority

7. **Formal Verification**: Consider using `kani` or `creusot` for formal verification of critical invariants.

8. **Invariant Monitoring**: Add Prometheus metrics that track invariant violations (e.g., daily spending approaching limit).

---

## Testing Strategy

### Unit Tests (Current)

- ✅ Spend limit enforcement
- ✅ Wallet key redaction
- ✅ Paper trading balance conservation
- ✅ Quote vs execution separation

### Property Tests (Recommended)

```rust
// Example property test for spend limit
#[proptest]
fn prop_daily_spending_never_exceeds_limit(
    trades: Vec<f64>,
    max_daily: f64,
) {
    let interceptor = SpendLimitInterceptor::new(100.0, max_daily);
    let mut daily_spent = DailySpending::new();
    
    for trade_value in trades {
        if daily_spent.current_total() + trade_value <= max_daily {
            daily_spent.add(trade_value);
        }
    }
    
    prop_assert!(daily_spent.current_total() <= max_daily);
}
```

### Integration Tests (Recommended)

- End-to-end interceptor pipeline ordering
- Query plan execution with partial failures
- Context updates across multiple cycles
- Gateway cache behavior under load

---

## Conclusion

These invariants represent the core safety and correctness guarantees of the DeFi trading agent. They should be:

1. **Documented in code** - Add module-level comments referencing this document
2. **Tested** - Encode as property tests to catch regressions
3. **Monitored** - Add metrics and alerts for invariant violations
4. **Reviewed** - Include in code review checklist

Violations of these invariants could lead to:
- **Critical**: Fund loss (private key exposure)
- **High**: Uncontrolled spending, bypassed risk controls
- **Medium**: Incorrect P&L tracking, degraded data quality
- **Low**: Performance issues, stale data

Maintain vigilance: these are the laws that cannot be broken.
