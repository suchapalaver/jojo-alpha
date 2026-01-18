//! Slippage guard interceptor
//!
//! Blocks trades that exceed the configured maximum slippage tolerance.

use async_trait::async_trait;
use baml_rt::error::Result;
use baml_rt::interceptor::{InterceptorDecision, ToolCallContext, ToolInterceptor};
use serde_json::Value;

/// Interceptor that blocks trades with excessive slippage
pub struct SlippageGuardInterceptor {
    /// Maximum allowed slippage (e.g., 1.0 for 1%)
    max_slippage_percent: f64,
}

impl SlippageGuardInterceptor {
    /// Create a new slippage guard
    ///
    /// # Arguments
    /// * `max_slippage_percent` - Maximum allowed slippage percentage (e.g., 1.0 for 1%)
    pub fn new(max_slippage_percent: f64) -> Self {
        Self {
            max_slippage_percent,
        }
    }
}

#[async_trait]
impl ToolInterceptor for SlippageGuardInterceptor {
    async fn intercept_tool_call(&self, context: &ToolCallContext) -> Result<InterceptorDecision> {
        // Only intercept odos_swap tool
        if context.tool_name != "odos_swap" {
            return Ok(InterceptorDecision::Allow);
        }

        // Check the slippage parameter
        let slippage = context
            .args
            .get("slippage_percent")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.5); // Default slippage if not specified

        if slippage > self.max_slippage_percent {
            return Ok(InterceptorDecision::Block(format!(
                "Requested slippage {:.2}% exceeds maximum allowed {:.2}%",
                slippage, self.max_slippage_percent
            )));
        }

        tracing::debug!(
            requested_slippage = slippage,
            max_slippage = self.max_slippage_percent,
            "Slippage check passed"
        );

        Ok(InterceptorDecision::Allow)
    }

    async fn on_tool_call_complete(
        &self,
        _context: &ToolCallContext,
        _result: &Result<Value>,
        _duration_ms: u64,
    ) {
        // No post-execution action needed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn test_allows_low_slippage() {
        let interceptor = SlippageGuardInterceptor::new(1.0);

        let context = ToolCallContext {
            tool_name: "odos_swap".to_string(),
            function_name: None,
            args: json!({
                "action": "prepare_swap",
                "slippage_percent": 0.5
            }),
            metadata: json!({}),
        };

        let decision = interceptor.intercept_tool_call(&context).await.unwrap();
        assert!(matches!(decision, InterceptorDecision::Allow));
    }

    #[tokio::test]
    async fn test_blocks_high_slippage() {
        let interceptor = SlippageGuardInterceptor::new(1.0);

        let context = ToolCallContext {
            tool_name: "odos_swap".to_string(),
            function_name: None,
            args: json!({
                "action": "prepare_swap",
                "slippage_percent": 5.0
            }),
            metadata: json!({}),
        };

        let decision = interceptor.intercept_tool_call(&context).await.unwrap();
        assert!(matches!(decision, InterceptorDecision::Block(_)));
    }
}
