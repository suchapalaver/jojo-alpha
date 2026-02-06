//! Audit log interceptor
//!
//! Logs all tool calls and LLM calls for compliance and debugging.

use async_trait::async_trait;
use baml_rt::error::Result;
use baml_rt::interceptor::{
    InterceptorDecision, LLMCallContext, LLMInterceptor, ToolCallContext, ToolInterceptor,
};
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Entry in the audit log
#[derive(Debug, Serialize)]
struct AuditEntry {
    timestamp: DateTime<Utc>,
    entry_type: &'static str,
    tool_name: Option<String>,
    function_name: Option<String>,
    args: Value,
    result: Option<Value>,
    error: Option<String>,
    duration_ms: u64,
    status: &'static str,
}

/// Writer for audit log entries
struct AuditLogWriter {
    path: PathBuf,
}

impl AuditLogWriter {
    fn new(path: PathBuf) -> Self {
        Self { path }
    }

    fn write(&self, entry: &AuditEntry) -> std::io::Result<()> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;

        let json = serde_json::to_string(entry)?;
        writeln!(file, "{}", json)?;
        Ok(())
    }
}

/// Interceptor that logs all operations to a file
pub struct AuditLogInterceptor {
    writer: Arc<Mutex<AuditLogWriter>>,
}

impl AuditLogInterceptor {
    /// Create a new audit log interceptor
    ///
    /// # Arguments
    /// * `log_path` - Path to the audit log file (JSONL format)
    pub fn new(log_path: impl Into<PathBuf>) -> Self {
        Self {
            writer: Arc::new(Mutex::new(AuditLogWriter::new(log_path.into()))),
        }
    }
}

#[async_trait]
impl ToolInterceptor for AuditLogInterceptor {
    async fn intercept_tool_call(&self, context: &ToolCallContext) -> Result<InterceptorDecision> {
        let entry = AuditEntry {
            timestamp: Utc::now(),
            entry_type: "tool_call_start",
            tool_name: Some(context.tool_name.clone()),
            function_name: context.function_name.clone(),
            args: context.args.clone(),
            result: None,
            error: None,
            duration_ms: 0,
            status: "pending",
        };

        let writer = self.writer.lock().await;
        if let Err(e) = writer.write(&entry) {
            tracing::warn!(error = %e, "Failed to write audit log entry");
        }

        // Audit logging never blocks
        Ok(InterceptorDecision::Allow)
    }

    async fn on_tool_call_complete(
        &self,
        context: &ToolCallContext,
        result: &Result<Value>,
        duration_ms: u64,
    ) {
        let (result_value, error, status) = match result {
            Ok(v) => (Some(v.clone()), None, "success"),
            Err(e) => (None, Some(e.to_string()), "error"),
        };

        let entry = AuditEntry {
            timestamp: Utc::now(),
            entry_type: "tool_call_complete",
            tool_name: Some(context.tool_name.clone()),
            function_name: context.function_name.clone(),
            args: context.args.clone(),
            result: result_value,
            error,
            duration_ms,
            status,
        };

        let writer = self.writer.lock().await;
        if let Err(e) = writer.write(&entry) {
            tracing::warn!(error = %e, "Failed to write audit log entry");
        }
    }
}

#[async_trait]
impl LLMInterceptor for AuditLogInterceptor {
    async fn intercept_llm_call(&self, context: &LLMCallContext) -> Result<InterceptorDecision> {
        let entry = AuditEntry {
            timestamp: Utc::now(),
            entry_type: "llm_call_start",
            tool_name: None,
            function_name: Some(context.function_name.clone()),
            args: serde_json::json!({
                "client": context.client,
                "model": context.model,
                "prompt_preview": truncate_prompt(&context.prompt)
            }),
            result: None,
            error: None,
            duration_ms: 0,
            status: "pending",
        };

        let writer = self.writer.lock().await;
        if let Err(e) = writer.write(&entry) {
            tracing::warn!(error = %e, "Failed to write audit log entry");
        }

        // Audit logging never blocks
        Ok(InterceptorDecision::Allow)
    }

    async fn on_llm_call_complete(
        &self,
        context: &LLMCallContext,
        result: &Result<Value>,
        duration_ms: u64,
    ) {
        let (result_value, error, status) = match result {
            Ok(v) => (Some(truncate_result(v)), None, "success"),
            Err(e) => (None, Some(e.to_string()), "error"),
        };

        let entry = AuditEntry {
            timestamp: Utc::now(),
            entry_type: "llm_call_complete",
            tool_name: None,
            function_name: Some(context.function_name.clone()),
            args: serde_json::json!({
                "client": context.client,
                "model": context.model,
            }),
            result: result_value,
            error,
            duration_ms,
            status,
        };

        let writer = self.writer.lock().await;
        if let Err(e) = writer.write(&entry) {
            tracing::warn!(error = %e, "Failed to write audit log entry");
        }
    }
}

/// Truncate prompt for logging (don't log full prompts)
fn truncate_prompt(prompt: &Value) -> Value {
    let s = serde_json::to_string(prompt).unwrap_or_default();
    if s.len() > 500 {
        serde_json::json!(format!("{}... [truncated]", &s[..500]))
    } else {
        prompt.clone()
    }
}

/// Truncate result for logging
fn truncate_result(result: &Value) -> Value {
    let s = serde_json::to_string(result).unwrap_or_default();
    if s.len() > 1000 {
        serde_json::json!(format!("{}... [truncated]", &s[..1000]))
    } else {
        result.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use baml_rt::generate_context_id;
    use serde_json::json;
    use tempfile::NamedTempFile;

    #[tokio::test]
    async fn test_logs_tool_call() {
        let temp_file = NamedTempFile::new().unwrap();
        let interceptor = AuditLogInterceptor::new(temp_file.path());

        let context = ToolCallContext {
            tool_name: "odos_swap".to_string(),
            function_name: Some("trading_loop".to_string()),
            args: json!({
                "action": "quote",
                "input_token": "0x...",
                "amount": "1000000"
            }),
            context_id: generate_context_id(),
            metadata: json!({}),
        };

        // Should always allow
        let decision = interceptor.intercept_tool_call(&context).await.unwrap();
        assert!(matches!(decision, InterceptorDecision::Allow));

        // Log completion
        interceptor
            .on_tool_call_complete(&context, &Ok(json!({"output_amount": "999000"})), 150)
            .await;

        // Check log file has entries
        let content = std::fs::read_to_string(temp_file.path()).unwrap();
        assert!(content.contains("tool_call_start"));
        assert!(content.contains("tool_call_complete"));
        assert!(content.contains("odos_swap"));
    }
}
