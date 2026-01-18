//! Cooldown interceptor
//!
//! Enforces a minimum time between trades to prevent rapid-fire trading.

use async_trait::async_trait;
use baml_rt::error::Result;
use baml_rt::interceptor::{InterceptorDecision, ToolCallContext, ToolInterceptor};
use serde_json::Value;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

/// Interceptor that enforces cooldown between trades
pub struct CooldownInterceptor {
    /// Minimum time between trades
    cooldown_duration: Duration,
    /// Last trade timestamp
    last_trade: Arc<RwLock<Option<Instant>>>,
}

impl CooldownInterceptor {
    /// Create a new cooldown interceptor
    ///
    /// # Arguments
    /// * `cooldown_seconds` - Minimum seconds between trades
    pub fn new(cooldown_seconds: u64) -> Self {
        Self {
            cooldown_duration: Duration::from_secs(cooldown_seconds),
            last_trade: Arc::new(RwLock::new(None)),
        }
    }
}

#[async_trait]
impl ToolInterceptor for CooldownInterceptor {
    async fn intercept_tool_call(&self, context: &ToolCallContext) -> Result<InterceptorDecision> {
        // Only intercept odos_swap prepare_swap actions
        if context.tool_name != "odos_swap" {
            return Ok(InterceptorDecision::Allow);
        }

        let action = context.args.get("action").and_then(|v| v.as_str());
        if action != Some("prepare_swap") {
            return Ok(InterceptorDecision::Allow);
        }

        // Check cooldown
        let last_trade = self.last_trade.read().await;
        if let Some(last) = *last_trade {
            let elapsed = last.elapsed();
            if elapsed < self.cooldown_duration {
                let remaining = self.cooldown_duration - elapsed;
                return Ok(InterceptorDecision::Block(format!(
                    "Trading cooldown active. Please wait {} more seconds.",
                    remaining.as_secs()
                )));
            }
        }

        tracing::debug!(
            cooldown_seconds = self.cooldown_duration.as_secs(),
            "Cooldown check passed"
        );

        Ok(InterceptorDecision::Allow)
    }

    async fn on_tool_call_complete(
        &self,
        context: &ToolCallContext,
        result: &Result<Value>,
        _duration_ms: u64,
    ) {
        // Update last trade time on successful prepare_swap
        if context.tool_name != "odos_swap" {
            return;
        }

        let action = context.args.get("action").and_then(|v| v.as_str());
        if action != Some("prepare_swap") {
            return;
        }

        if result.is_ok() {
            let mut last_trade = self.last_trade.write().await;
            *last_trade = Some(Instant::now());
            tracing::info!("Updated last trade timestamp for cooldown tracking");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn test_allows_first_trade() {
        let interceptor = CooldownInterceptor::new(60);

        let context = ToolCallContext {
            tool_name: "odos_swap".to_string(),
            function_name: None,
            args: json!({
                "action": "prepare_swap"
            }),
            metadata: json!({}),
        };

        let decision = interceptor.intercept_tool_call(&context).await.unwrap();
        assert!(matches!(decision, InterceptorDecision::Allow));
    }

    #[tokio::test]
    async fn test_blocks_rapid_trades() {
        let interceptor = CooldownInterceptor::new(60);

        let context = ToolCallContext {
            tool_name: "odos_swap".to_string(),
            function_name: None,
            args: json!({
                "action": "prepare_swap"
            }),
            metadata: json!({}),
        };

        // First trade should be allowed
        let decision = interceptor.intercept_tool_call(&context).await.unwrap();
        assert!(matches!(decision, InterceptorDecision::Allow));

        // Simulate successful trade
        interceptor
            .on_tool_call_complete(&context, &Ok(json!({})), 100)
            .await;

        // Second trade immediately after should be blocked
        let decision = interceptor.intercept_tool_call(&context).await.unwrap();
        assert!(matches!(decision, InterceptorDecision::Block(_)));
    }

    #[tokio::test]
    async fn test_allows_quotes_during_cooldown() {
        let interceptor = CooldownInterceptor::new(60);

        // Simulate a completed trade
        {
            let mut last_trade = interceptor.last_trade.write().await;
            *last_trade = Some(Instant::now());
        }

        // Quote should still be allowed
        let context = ToolCallContext {
            tool_name: "odos_swap".to_string(),
            function_name: None,
            args: json!({
                "action": "quote"
            }),
            metadata: json!({}),
        };

        let decision = interceptor.intercept_tool_call(&context).await.unwrap();
        assert!(matches!(decision, InterceptorDecision::Allow));
    }
}
