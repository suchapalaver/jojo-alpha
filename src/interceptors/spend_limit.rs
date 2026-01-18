//! Spend limit interceptor
//!
//! Enforces per-trade and daily spending limits to prevent runaway losses.

use async_trait::async_trait;
use baml_rt::error::Result;
use baml_rt::interceptor::{InterceptorDecision, ToolCallContext, ToolInterceptor};
use chrono::{DateTime, Utc};
use serde_json::Value;
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
    /// This is a simplified estimation - in production you'd want to
    /// query current prices from an oracle.
    fn estimate_trade_value(&self, args: &Value) -> Option<f64> {
        // For now, assume the amount field contains a USD value
        // In production, you'd convert based on token prices
        args.get("amount").and_then(|v| v.as_str()).and_then(|s| {
            // Parse wei amount and convert to USD estimate
            // This is a placeholder - real implementation needs price feeds
            s.parse::<f64>().ok().map(|wei| {
                // Rough estimate: assume input is in 6 decimals (USDC)
                wei / 1_000_000.0
            })
        })
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
    use serde_json::json;

    #[tokio::test]
    async fn test_allows_small_trade() {
        let interceptor = SpendLimitInterceptor::new(100.0, 500.0);

        let context = ToolCallContext {
            tool_name: "odos_swap".to_string(),
            function_name: None,
            args: json!({
                "action": "prepare_swap",
                "amount": "50000000" // 50 USDC (6 decimals)
            }),
            metadata: json!({}),
        };

        let decision = interceptor.intercept_tool_call(&context).await.unwrap();
        assert!(matches!(decision, InterceptorDecision::Allow));
    }

    #[tokio::test]
    async fn test_blocks_large_trade() {
        let interceptor = SpendLimitInterceptor::new(100.0, 500.0);

        let context = ToolCallContext {
            tool_name: "odos_swap".to_string(),
            function_name: None,
            args: json!({
                "action": "prepare_swap",
                "amount": "200000000" // 200 USDC
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
                "amount": "999999999999" // Huge amount
            }),
            metadata: json!({}),
        };

        // Quotes should always be allowed (read-only)
        let decision = interceptor.intercept_tool_call(&context).await.unwrap();
        assert!(matches!(decision, InterceptorDecision::Allow));
    }
}
