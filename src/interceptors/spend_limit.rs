//! Spend limit interceptor
//!
//! Enforces per-trade and daily spending limits to prevent runaway losses.
//! Uses the shared token registry for consistent token information.

use crate::config::SpendLimitMode;
use crate::tokens;
use crate::tools::TOOL_ODOS_SWAP;
use alloy::primitives::Address;
use async_trait::async_trait;
use baml_rt::error::Result;
use baml_rt::interceptor::{InterceptorDecision, ToolCallContext, ToolInterceptor};
use chrono::{DateTime, Utc};
use serde_json::Value;
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Tracks daily spending
struct DailySpending {
    /// Total spent today (USD)
    total: f64,
    /// Date of the current tracking period
    date: DateTime<Utc>,
    /// Individual trade amounts for audit
    trades: Vec<f64>,
}

impl DailySpending {
    fn new() -> Self {
        Self {
            total: 0.0,
            date: Utc::now(),
            trades: Vec::new(),
        }
    }

    /// Add a trade amount, resetting if it's a new day
    fn add(&mut self, amount: f64) {
        let now = Utc::now();
        if now.date_naive() != self.date.date_naive() {
            // New day - reset
            self.total = 0.0;
            self.trades.clear();
            self.date = now;
        }
        self.total += amount;
        self.trades.push(amount);
    }

    /// Get current daily total, resetting if it's a new day
    fn current_total(&mut self) -> f64 {
        let now = Utc::now();
        if now.date_naive() != self.date.date_naive() {
            self.total = 0.0;
            self.trades.clear();
            self.date = now;
        }
        self.total
    }
}

/// Interceptor that enforces spending limits
pub struct SpendLimitInterceptor {
    /// Maximum value per single trade (USD)
    max_per_trade: f64,
    /// Maximum daily spending (USD)
    max_daily: f64,
    /// Current daily spending tracker
    daily_spent: Arc<RwLock<DailySpending>>,
    /// Enforcement mode for unknown tokens
    mode: SpendLimitMode,
}

impl SpendLimitInterceptor {
    /// Create a new spend limit interceptor with fail-open mode (default)
    ///
    /// # Arguments
    /// * `max_per_trade` - Maximum USD value for a single trade
    /// * `max_daily` - Maximum USD value for all trades in a day
    pub fn new(max_per_trade: f64, max_daily: f64) -> Self {
        Self {
            max_per_trade,
            max_daily,
            daily_spent: Arc::new(RwLock::new(DailySpending::new())),
            mode: SpendLimitMode::FailOpen,
        }
    }

    /// Create a new spend limit interceptor with specified mode
    ///
    /// # Arguments
    /// * `max_per_trade` - Maximum USD value for a single trade
    /// * `max_daily` - Maximum USD value for all trades in a day
    /// * `mode` - Enforcement mode (fail-open or fail-closed)
    pub fn with_mode(max_per_trade: f64, max_daily: f64, mode: SpendLimitMode) -> Self {
        Self {
            max_per_trade,
            max_daily,
            daily_spent: Arc::new(RwLock::new(DailySpending::new())),
            mode,
        }
    }

    /// Estimate trade value in USD from the args
    ///
    /// Priority:
    /// 1. Use `amount_usd` if explicitly provided (most accurate)
    /// 2. Use shared token registry to calculate USD value
    /// 3. Return None for unknown tokens (handled by mode)
    fn estimate_trade_value(&self, args: &Value) -> Option<f64> {
        // Priority 1: Use explicit amount_usd if provided
        if let Some(usd) = args.get("amount_usd").and_then(|v| v.as_f64()) {
            tracing::debug!(amount_usd = usd, "Using explicit amount_usd");
            return Some(usd);
        }

        // Priority 2: Calculate from token amount using shared registry
        let amount_str = args.get("amount").and_then(|v| v.as_str())?;
        let input_token_str = args.get("input_token").and_then(|v| v.as_str())?;
        let input_token = Address::from_str(input_token_str).ok()?;

        let registry = tokens::registry();

        match registry.get(&input_token) {
            Some(info) => {
                let amount: f64 = amount_str.parse().ok()?;
                let divisor = 10_f64.powi(info.decimals as i32);
                let token_amount = amount / divisor;

                if info.is_stablecoin {
                    tracing::debug!(
                        token = %input_token,
                        symbol = info.symbol,
                        decimals = info.decimals,
                        token_amount = token_amount,
                        "Stablecoin detected, using 1:1 USD value"
                    );
                    Some(token_amount)
                } else if let Some(price) = info.approx_price_usd {
                    // Use approximate price (with warning)
                    let usd_value = token_amount * price;
                    tracing::warn!(
                        token = %input_token,
                        symbol = info.symbol,
                        approx_price = price,
                        usd_value = usd_value,
                        "Using approximate price for non-stablecoin. \
                         Pass amount_usd explicitly for accurate limit enforcement."
                    );
                    Some(usd_value)
                } else {
                    tracing::warn!(
                        token = %input_token,
                        symbol = info.symbol,
                        "Known token without price - cannot estimate USD value. \
                         Pass amount_usd explicitly for accurate limit enforcement."
                    );
                    None
                }
            }
            None => {
                // Unknown token - log and return None
                tracing::warn!(
                    token = %input_token,
                    "Unknown token address - cannot determine decimals or USD value. \
                     Pass amount_usd explicitly for accurate limit enforcement."
                );
                None
            }
        }
    }
}

#[async_trait]
impl ToolInterceptor for SpendLimitInterceptor {
    async fn intercept_tool_call(&self, context: &ToolCallContext) -> Result<InterceptorDecision> {
        // Only intercept odos_swap tool
        if context.tool_name != TOOL_ODOS_SWAP {
            return Ok(InterceptorDecision::Allow);
        }

        // Only check prepare_swap actions (not quotes)
        let action = context.args.get("action").and_then(|v| v.as_str());
        if action != Some("prepare_swap") {
            return Ok(InterceptorDecision::Allow);
        }

        // Estimate trade value
        let trade_value = match self.estimate_trade_value(&context.args) {
            Some(v) => v,
            None => {
                // Handle based on mode
                match self.mode {
                    SpendLimitMode::FailOpen => {
                        tracing::warn!(
                            "Could not estimate trade value, allowing with caution (fail-open mode)"
                        );
                        return Ok(InterceptorDecision::Allow);
                    }
                    SpendLimitMode::FailClosed => {
                        return Ok(InterceptorDecision::Block(
                            "Cannot determine USD value for spend limit check. \
                             Provide amount_usd parameter or use a known token."
                                .to_string(),
                        ));
                    }
                }
            }
        };

        // Check per-trade limit
        if trade_value > self.max_per_trade {
            return Ok(InterceptorDecision::Block(format!(
                "Trade value ${:.2} exceeds per-trade limit of ${:.2}",
                trade_value, self.max_per_trade
            )));
        }

        // Check daily limit
        let mut daily_spent = self.daily_spent.write().await;
        let current_daily = daily_spent.current_total();

        if current_daily + trade_value > self.max_daily {
            return Ok(InterceptorDecision::Block(format!(
                "Trade would exceed daily limit. Current: ${:.2}, This trade: ${:.2}, Limit: ${:.2}",
                current_daily, trade_value, self.max_daily
            )));
        }

        tracing::info!(
            trade_value = trade_value,
            daily_total = current_daily,
            max_per_trade = self.max_per_trade,
            max_daily = self.max_daily,
            "Spend limit check passed"
        );

        Ok(InterceptorDecision::Allow)
    }

    async fn on_tool_call_complete(
        &self,
        context: &ToolCallContext,
        result: &Result<Value>,
        _duration_ms: u64,
    ) {
        // Only track successful prepare_swap operations
        if context.tool_name != TOOL_ODOS_SWAP {
            return;
        }

        let action = context.args.get("action").and_then(|v| v.as_str());
        if action != Some("prepare_swap") {
            return;
        }

        if result.is_ok() {
            if let Some(trade_value) = self.estimate_trade_value(&context.args) {
                let mut daily_spent = self.daily_spent.write().await;
                daily_spent.add(trade_value);
                tracing::info!(
                    trade_value = trade_value,
                    new_daily_total = daily_spent.total,
                    "Updated daily spending tracker"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokens::addresses;
    use baml_rt::generate_context_id;
    use serde_json::json;

    #[tokio::test]
    async fn test_allows_small_trade_with_explicit_usd() {
        let interceptor = SpendLimitInterceptor::new(100.0, 500.0);

        let context = ToolCallContext {
            tool_name: TOOL_ODOS_SWAP.to_string(),
            function_name: None,
            args: json!({
                "action": "prepare_swap",
                "input_token": addresses::USDC_ETH.to_string(),
                "amount": "50000000", // 50 USDC (6 decimals)
                "amount_usd": 50.0    // Explicit USD value
            }),
            context_id: generate_context_id(),
            metadata: json!({}),
        };

        let decision = interceptor.intercept_tool_call(&context).await.unwrap();
        assert!(matches!(decision, InterceptorDecision::Allow));
    }

    #[tokio::test]
    async fn test_allows_small_stablecoin_trade() {
        let interceptor = SpendLimitInterceptor::new(100.0, 500.0);

        let context = ToolCallContext {
            tool_name: TOOL_ODOS_SWAP.to_string(),
            function_name: None,
            args: json!({
                "action": "prepare_swap",
                "input_token": addresses::USDC_ETH.to_string(),
                "amount": "50000000" // 50 USDC (6 decimals) - no explicit amount_usd
            }),
            context_id: generate_context_id(),
            metadata: json!({}),
        };

        let decision = interceptor.intercept_tool_call(&context).await.unwrap();
        assert!(matches!(decision, InterceptorDecision::Allow));
    }

    #[tokio::test]
    async fn test_blocks_large_trade_with_explicit_usd() {
        let interceptor = SpendLimitInterceptor::new(100.0, 500.0);

        let context = ToolCallContext {
            tool_name: TOOL_ODOS_SWAP.to_string(),
            function_name: None,
            args: json!({
                "action": "prepare_swap",
                "input_token": addresses::USDC_ETH.to_string(),
                "amount": "200000000", // 200 USDC
                "amount_usd": 200.0    // Explicit USD value
            }),
            context_id: generate_context_id(),
            metadata: json!({}),
        };

        let decision = interceptor.intercept_tool_call(&context).await.unwrap();
        assert!(matches!(decision, InterceptorDecision::Block(_)));
    }

    #[tokio::test]
    async fn test_blocks_large_stablecoin_trade() {
        let interceptor = SpendLimitInterceptor::new(100.0, 500.0);

        let context = ToolCallContext {
            tool_name: TOOL_ODOS_SWAP.to_string(),
            function_name: None,
            args: json!({
                "action": "prepare_swap",
                "input_token": addresses::USDC_ETH.to_string(),
                "amount": "200000000" // 200 USDC - no explicit amount_usd
            }),
            context_id: generate_context_id(),
            metadata: json!({}),
        };

        let decision = interceptor.intercept_tool_call(&context).await.unwrap();
        assert!(matches!(decision, InterceptorDecision::Block(_)));
    }

    #[tokio::test]
    async fn test_allows_quotes() {
        let interceptor = SpendLimitInterceptor::new(100.0, 500.0);

        let context = ToolCallContext {
            tool_name: TOOL_ODOS_SWAP.to_string(),
            function_name: None,
            args: json!({
                "action": "quote",
                "input_token": addresses::USDC_ETH.to_string(),
                "amount": "999999999999" // Huge amount
            }),
            context_id: generate_context_id(),
            metadata: json!({}),
        };

        // Quotes should always be allowed (read-only)
        let decision = interceptor.intercept_tool_call(&context).await.unwrap();
        assert!(matches!(decision, InterceptorDecision::Allow));
    }

    #[tokio::test]
    async fn test_fail_open_allows_unknown_token() {
        // Fail-open mode (default) - unknown tokens without explicit amount_usd should be allowed
        let interceptor = SpendLimitInterceptor::new(100.0, 500.0);
        let unknown_token = "0x1234567890123456789012345678901234567890";

        let context = ToolCallContext {
            tool_name: TOOL_ODOS_SWAP.to_string(),
            function_name: None,
            args: json!({
                "action": "prepare_swap",
                "input_token": unknown_token,
                "amount": "999999999999999999999" // Huge amount
            }),
            context_id: generate_context_id(),
            metadata: json!({}),
        };

        let decision = interceptor.intercept_tool_call(&context).await.unwrap();
        assert!(matches!(decision, InterceptorDecision::Allow));
    }

    #[tokio::test]
    async fn test_fail_closed_blocks_unknown_token() {
        // Fail-closed mode - unknown tokens without explicit amount_usd should be blocked
        let interceptor =
            SpendLimitInterceptor::with_mode(100.0, 500.0, SpendLimitMode::FailClosed);
        let unknown_token = "0x1234567890123456789012345678901234567890";

        let context = ToolCallContext {
            tool_name: TOOL_ODOS_SWAP.to_string(),
            function_name: None,
            args: json!({
                "action": "prepare_swap",
                "input_token": unknown_token,
                "amount": "1000000" // Even small amount blocked
            }),
            context_id: generate_context_id(),
            metadata: json!({}),
        };

        let decision = interceptor.intercept_tool_call(&context).await.unwrap();
        assert!(matches!(decision, InterceptorDecision::Block(_)));
    }

    #[tokio::test]
    async fn test_fail_closed_allows_with_explicit_usd() {
        // Fail-closed mode - but explicit amount_usd should still work
        let interceptor =
            SpendLimitInterceptor::with_mode(100.0, 500.0, SpendLimitMode::FailClosed);
        let unknown_token = "0x1234567890123456789012345678901234567890";

        let context = ToolCallContext {
            tool_name: TOOL_ODOS_SWAP.to_string(),
            function_name: None,
            args: json!({
                "action": "prepare_swap",
                "input_token": unknown_token,
                "amount": "1000000",
                "amount_usd": 50.0 // Explicit USD bypasses unknown token check
            }),
            context_id: generate_context_id(),
            metadata: json!({}),
        };

        let decision = interceptor.intercept_tool_call(&context).await.unwrap();
        assert!(matches!(decision, InterceptorDecision::Allow));
    }

    #[tokio::test]
    async fn test_weth_uses_approx_price() {
        // WETH has an approximate price in the registry
        let interceptor = SpendLimitInterceptor::new(100.0, 500.0);

        let context = ToolCallContext {
            tool_name: TOOL_ODOS_SWAP.to_string(),
            function_name: None,
            args: json!({
                "action": "prepare_swap",
                "input_token": addresses::WETH_ETH.to_string(),
                "amount": "1000000000000000000" // 1 WETH (18 decimals) ~ $3500
            }),
            context_id: generate_context_id(),
            metadata: json!({}),
        };

        // Should be blocked because 1 WETH ~= $3500 > $100 limit
        let decision = interceptor.intercept_tool_call(&context).await.unwrap();
        assert!(matches!(decision, InterceptorDecision::Block(_)));
    }

    #[tokio::test]
    async fn test_blocks_non_stablecoin_with_explicit_large_usd() {
        // Non-stablecoins with explicit amount_usd should be blocked if too large
        let interceptor = SpendLimitInterceptor::new(100.0, 500.0);

        let context = ToolCallContext {
            tool_name: TOOL_ODOS_SWAP.to_string(),
            function_name: None,
            args: json!({
                "action": "prepare_swap",
                "input_token": addresses::WETH_ETH.to_string(),
                "amount": "1000000000000000000", // 1 WETH
                "amount_usd": 3500.0             // ~$3500 at current prices
            }),
            context_id: generate_context_id(),
            metadata: json!({}),
        };

        let decision = interceptor.intercept_tool_call(&context).await.unwrap();
        assert!(matches!(decision, InterceptorDecision::Block(_)));
    }
}
