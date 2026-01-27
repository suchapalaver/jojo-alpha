//! Spend limit interceptor
//!
//! Enforces per-trade and daily spending limits to prevent runaway losses.

use alloy::primitives::{address, Address};
use async_trait::async_trait;
use baml_rt::error::Result;
use baml_rt::interceptor::{InterceptorDecision, ToolCallContext, ToolInterceptor};
use chrono::{DateTime, Utc};
use serde_json::Value;
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Well-known token info: (decimals, is_stablecoin)
struct TokenInfo {
    decimals: u8,
    is_stablecoin: bool,
}

impl TokenInfo {
    const fn stable(decimals: u8) -> Self {
        Self {
            decimals,
            is_stablecoin: true,
        }
    }
    const fn token(decimals: u8) -> Self {
        Self {
            decimals,
            is_stablecoin: false,
        }
    }
}

/// Well-known tokens across supported networks
fn known_tokens() -> HashMap<Address, TokenInfo> {
    HashMap::from([
        // === Ethereum Mainnet ===
        (
            address!("a0b86991c6218b36c1d19d4a2e9eb0ce3606eb48"),
            TokenInfo::stable(6),
        ), // USDC
        (
            address!("dac17f958d2ee523a2206206994597c13d831ec7"),
            TokenInfo::stable(6),
        ), // USDT
        (
            address!("6b175474e89094c44da98b954eedeac495271d0f"),
            TokenInfo::stable(18),
        ), // DAI
        (
            address!("c02aaa39b223fe8d0a0e5c4f27ead9083c756cc2"),
            TokenInfo::token(18),
        ), // WETH
        // === Arbitrum ===
        (
            address!("af88d065e77c8cc2239327c5edb3a432268e5831"),
            TokenInfo::stable(6),
        ), // USDC (native)
        (
            address!("ff970a61a04b1ca14834a43f5de4533ebddb5cc8"),
            TokenInfo::stable(6),
        ), // USDC.e (bridged)
        (
            address!("fd086bc7cd5c481dcc9c85ebe478a1c0b69fcbb9"),
            TokenInfo::stable(6),
        ), // USDT
        (
            address!("da10009cbd5d07dd0cecc66161fc93d7c9000da1"),
            TokenInfo::stable(18),
        ), // DAI
        (
            address!("82af49447d8a07e3bd95bd0d56f35241523fbab1"),
            TokenInfo::token(18),
        ), // WETH
        // === Optimism ===
        (
            address!("0b2c639c533813f4aa9d7837caf62653d097ff85"),
            TokenInfo::stable(6),
        ), // USDC (native)
        (
            address!("7f5c764cbc14f9669b88837ca1490cca17c31607"),
            TokenInfo::stable(6),
        ), // USDC.e (bridged)
        (
            address!("94b008aa00579c1307b0ef2c499ad98a8ce58e58"),
            TokenInfo::stable(6),
        ), // USDT
        (
            address!("4200000000000000000000000000000000000006"),
            TokenInfo::token(18),
        ), // WETH
        // === Base ===
        (
            address!("833589fcd6edb6e08f4c7c32d4f71b54bda02913"),
            TokenInfo::stable(6),
        ), // USDC
        (
            address!("50c5725949a6f0c72e6c4a641f24049a917db0cb"),
            TokenInfo::stable(18),
        ), // DAI
        // === Native ETH (both conventions) ===
        (
            address!("eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"),
            TokenInfo::token(18),
        ), // Common convention
        (
            address!("0000000000000000000000000000000000000000"),
            TokenInfo::token(18),
        ), // Odos zero address
    ])
}

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
}

impl SpendLimitInterceptor {
    /// Create a new spend limit interceptor
    ///
    /// # Arguments
    /// * `max_per_trade` - Maximum USD value for a single trade
    /// * `max_daily` - Maximum USD value for all trades in a day
    pub fn new(max_per_trade: f64, max_daily: f64) -> Self {
        Self {
            max_per_trade,
            max_daily,
            daily_spent: Arc::new(RwLock::new(DailySpending::new())),
        }
    }

    /// Estimate trade value in USD from the args
    ///
    /// Priority:
    /// 1. Use `amount_usd` if explicitly provided (most accurate)
    /// 2. For stablecoins, use token decimals to calculate USD value
    /// 3. For unknown tokens, log warning and allow (fail-open for safety)
    fn estimate_trade_value(&self, args: &Value) -> Option<f64> {
        // Priority 1: Use explicit amount_usd if provided
        if let Some(usd) = args.get("amount_usd").and_then(|v| v.as_f64()) {
            tracing::debug!(amount_usd = usd, "Using explicit amount_usd");
            return Some(usd);
        }

        // Priority 2: Calculate from token amount using decimals
        let amount_str = args.get("amount").and_then(|v| v.as_str())?;
        let amount: f64 = amount_str.parse().ok()?;

        let input_token_str = args.get("input_token").and_then(|v| v.as_str())?;
        let input_token = Address::from_str(input_token_str).ok()?;

        let tokens = known_tokens();

        match tokens.get(&input_token) {
            Some(info) => {
                let divisor = 10_f64.powi(info.decimals as i32);
                let token_amount = amount / divisor;

                if info.is_stablecoin {
                    tracing::debug!(
                        token = %input_token,
                        decimals = info.decimals,
                        token_amount = token_amount,
                        "Stablecoin detected, using 1:1 USD value"
                    );
                    Some(token_amount)
                } else {
                    // Non-stablecoin with known decimals but unknown price
                    // Log warning and return None to trigger fail-open behavior
                    tracing::warn!(
                        token = %input_token,
                        decimals = info.decimals,
                        token_amount = token_amount,
                        "Non-stablecoin token without price feed - cannot estimate USD value. \
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
        if context.tool_name != "odos_swap" {
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
                tracing::warn!("Could not estimate trade value, allowing with caution");
                return Ok(InterceptorDecision::Allow);
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
        if context.tool_name != "odos_swap" {
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
    use alloy::primitives::address;
    use serde_json::json;

    // Ethereum mainnet addresses (compile-time validated)
    const USDC_ETH: Address = address!("a0b86991c6218b36c1d19d4a2e9eb0ce3606eb48");
    const WETH_ETH: Address = address!("c02aaa39b223fe8d0a0e5c4f27ead9083c756cc2");

    #[tokio::test]
    async fn test_allows_small_trade_with_explicit_usd() {
        let interceptor = SpendLimitInterceptor::new(100.0, 500.0);

        let context = ToolCallContext {
            tool_name: "odos_swap".to_string(),
            function_name: None,
            args: json!({
                "action": "prepare_swap",
                "input_token": USDC_ETH.to_string(),
                "amount": "50000000", // 50 USDC (6 decimals)
                "amount_usd": 50.0    // Explicit USD value
            }),
            metadata: json!({}),
        };

        let decision = interceptor.intercept_tool_call(&context).await.unwrap();
        assert!(matches!(decision, InterceptorDecision::Allow));
    }

    #[tokio::test]
    async fn test_allows_small_stablecoin_trade() {
        let interceptor = SpendLimitInterceptor::new(100.0, 500.0);

        let context = ToolCallContext {
            tool_name: "odos_swap".to_string(),
            function_name: None,
            args: json!({
                "action": "prepare_swap",
                "input_token": USDC_ETH.to_string(),
                "amount": "50000000" // 50 USDC (6 decimals) - no explicit amount_usd
            }),
            metadata: json!({}),
        };

        let decision = interceptor.intercept_tool_call(&context).await.unwrap();
        assert!(matches!(decision, InterceptorDecision::Allow));
    }

    #[tokio::test]
    async fn test_blocks_large_trade_with_explicit_usd() {
        let interceptor = SpendLimitInterceptor::new(100.0, 500.0);

        let context = ToolCallContext {
            tool_name: "odos_swap".to_string(),
            function_name: None,
            args: json!({
                "action": "prepare_swap",
                "input_token": USDC_ETH.to_string(),
                "amount": "200000000", // 200 USDC
                "amount_usd": 200.0    // Explicit USD value
            }),
            metadata: json!({}),
        };

        let decision = interceptor.intercept_tool_call(&context).await.unwrap();
        assert!(matches!(decision, InterceptorDecision::Block(_)));
    }

    #[tokio::test]
    async fn test_blocks_large_stablecoin_trade() {
        let interceptor = SpendLimitInterceptor::new(100.0, 500.0);

        let context = ToolCallContext {
            tool_name: "odos_swap".to_string(),
            function_name: None,
            args: json!({
                "action": "prepare_swap",
                "input_token": USDC_ETH.to_string(),
                "amount": "200000000" // 200 USDC - no explicit amount_usd
            }),
            metadata: json!({}),
        };

        let decision = interceptor.intercept_tool_call(&context).await.unwrap();
        assert!(matches!(decision, InterceptorDecision::Block(_)));
    }

    #[tokio::test]
    async fn test_allows_quotes() {
        let interceptor = SpendLimitInterceptor::new(100.0, 500.0);

        let context = ToolCallContext {
            tool_name: "odos_swap".to_string(),
            function_name: None,
            args: json!({
                "action": "quote",
                "input_token": USDC_ETH.to_string(),
                "amount": "999999999999" // Huge amount
            }),
            metadata: json!({}),
        };

        // Quotes should always be allowed (read-only)
        let decision = interceptor.intercept_tool_call(&context).await.unwrap();
        assert!(matches!(decision, InterceptorDecision::Allow));
    }

    #[tokio::test]
    async fn test_allows_non_stablecoin_without_usd_value() {
        // Non-stablecoins without explicit amount_usd should be allowed (fail-open)
        // because we can't determine the USD value without a price feed
        let interceptor = SpendLimitInterceptor::new(100.0, 500.0);

        let context = ToolCallContext {
            tool_name: "odos_swap".to_string(),
            function_name: None,
            args: json!({
                "action": "prepare_swap",
                "input_token": WETH_ETH.to_string(),
                "amount": "1000000000000000000" // 1 WETH (18 decimals) - no amount_usd
            }),
            metadata: json!({}),
        };

        // Should allow because we can't estimate USD value for non-stablecoins
        let decision = interceptor.intercept_tool_call(&context).await.unwrap();
        assert!(matches!(decision, InterceptorDecision::Allow));
    }

    #[tokio::test]
    async fn test_blocks_non_stablecoin_with_explicit_large_usd() {
        // Non-stablecoins with explicit amount_usd should be blocked if too large
        let interceptor = SpendLimitInterceptor::new(100.0, 500.0);

        let context = ToolCallContext {
            tool_name: "odos_swap".to_string(),
            function_name: None,
            args: json!({
                "action": "prepare_swap",
                "input_token": WETH_ETH.to_string(),
                "amount": "1000000000000000000", // 1 WETH
                "amount_usd": 3500.0             // ~$3500 at current prices
            }),
            metadata: json!({}),
        };

        let decision = interceptor.intercept_tool_call(&context).await.unwrap();
        assert!(matches!(decision, InterceptorDecision::Block(_)));
    }

    #[tokio::test]
    async fn test_allows_unknown_token_without_usd_value() {
        // Unknown tokens without explicit amount_usd should be allowed (fail-open)
        let interceptor = SpendLimitInterceptor::new(100.0, 500.0);
        let unknown_token = address!("1234567890123456789012345678901234567890");

        let context = ToolCallContext {
            tool_name: "odos_swap".to_string(),
            function_name: None,
            args: json!({
                "action": "prepare_swap",
                "input_token": unknown_token.to_string(),
                "amount": "999999999999999999999" // Huge amount
            }),
            metadata: json!({}),
        };

        // Should allow because we can't estimate USD value for unknown tokens
        let decision = interceptor.intercept_tool_call(&context).await.unwrap();
        assert!(matches!(decision, InterceptorDecision::Allow));
    }
}
