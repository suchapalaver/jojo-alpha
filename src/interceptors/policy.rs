//! Policy enforcement interceptor for tool calls.

use baml_rt::error::Result as BamlResult;
use baml_rt::interceptor::{InterceptorDecision, ToolCallContext, ToolInterceptor};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;
use tracing::warn;

#[derive(Debug, Clone, Copy)]
enum PolicyMode {
    AllowAll,
    DefaultDeny,
}

#[derive(Debug, Clone)]
struct PolicyDecision {
    allowed: bool,
    rule_id: Option<String>,
    reason: String,
}

#[derive(Debug, Clone)]
pub struct PolicyConfig {
    mode: PolicyMode,
    rules: HashMap<String, PolicyDecision>,
}

impl PolicyConfig {
    pub fn allow_all() -> Self {
        Self {
            mode: PolicyMode::AllowAll,
            rules: HashMap::new(),
        }
    }

    pub async fn load_from_dir(agent_dir: &Path) -> crate::Result<Self> {
        let policy_path = agent_dir.join("policy.json");
        if !policy_path.exists() {
            return Ok(Self::allow_all());
        }

        let contents = tokio::fs::read_to_string(&policy_path)
            .await
            .map_err(|e| crate::Error::Config(e.to_string()))?;
        let parsed: PolicyFile = serde_json::from_str(&contents)?;

        let mode = match parsed.mode.as_str() {
            "default-deny" => PolicyMode::DefaultDeny,
            "allow-all" => PolicyMode::AllowAll,
            other => {
                warn!(mode = other, "Unknown policy mode, defaulting to allow-all");
                PolicyMode::AllowAll
            }
        };

        let mut rules = HashMap::new();
        for rule in parsed.rules {
            if !is_valid_tool_name(&rule.tool) {
                warn!(
                    tool = %rule.tool,
                    "Invalid tool name in policy.json; skipping rule"
                );
                continue;
            }

            rules.insert(
                rule.tool,
                PolicyDecision {
                    allowed: rule.allowed,
                    rule_id: rule.rule_id,
                    reason: rule.reason.unwrap_or_else(|| "policy rule".to_string()),
                },
            );
        }

        Ok(Self { mode, rules })
    }

    fn decision_for_tool(&self, tool: &str) -> PolicyDecision {
        if let Some(decision) = self.rules.get(tool) {
            return decision.clone();
        }

        match self.mode {
            PolicyMode::AllowAll => PolicyDecision {
                allowed: true,
                rule_id: None,
                reason: "allowed by default policy".to_string(),
            },
            PolicyMode::DefaultDeny => PolicyDecision {
                allowed: false,
                rule_id: None,
                reason: "denied by default policy".to_string(),
            },
        }
    }
}

#[derive(Debug, Clone)]
pub struct PolicyInterceptor {
    policy: PolicyConfig,
}

impl PolicyInterceptor {
    pub fn new(policy: PolicyConfig) -> Self {
        Self { policy }
    }
}

#[async_trait::async_trait]
impl ToolInterceptor for PolicyInterceptor {
    async fn intercept_tool_call(
        &self,
        context: &ToolCallContext,
    ) -> BamlResult<InterceptorDecision> {
        let decision = self.policy.decision_for_tool(&context.tool_name);
        if decision.allowed {
            return Ok(InterceptorDecision::Allow);
        }

        let rule_id = decision
            .rule_id
            .as_ref()
            .map(|id| format!(" rule_id={}", id))
            .unwrap_or_default();
        Ok(InterceptorDecision::Block(format!(
            "Policy denied tool {}: {}{}",
            context.tool_name, decision.reason, rule_id
        )))
    }

    async fn on_tool_call_complete(
        &self,
        _context: &ToolCallContext,
        _result: &std::result::Result<serde_json::Value, baml_rt::error::BamlRtError>,
        _duration_ms: u64,
    ) {
    }
}

#[derive(Debug, Clone, Deserialize)]
struct PolicyFile {
    mode: String,
    rules: Vec<PolicyRule>,
}

#[derive(Debug, Clone, Deserialize)]
struct PolicyRule {
    tool: String,
    allowed: bool,
    rule_id: Option<String>,
    reason: Option<String>,
}

fn is_valid_tool_name(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    name.bytes()
        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == b'_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_allow_policy_allows_unknown_tools() {
        let policy = PolicyConfig::allow_all();
        let decision = policy.decision_for_tool("odos_swap");
        assert!(decision.allowed);
    }

    #[test]
    fn default_deny_policy_blocks_unknown_tools() {
        let policy = PolicyConfig {
            mode: PolicyMode::DefaultDeny,
            rules: HashMap::new(),
        };
        let decision = policy.decision_for_tool("odos_swap");
        assert!(!decision.allowed);
    }

    #[test]
    fn rule_overrides_default_mode() {
        let mut rules = HashMap::new();
        rules.insert(
            "odos_swap".to_string(),
            PolicyDecision {
                allowed: true,
                rule_id: Some("allow:odos_swap".to_string()),
                reason: "explicit allow".to_string(),
            },
        );
        let policy = PolicyConfig {
            mode: PolicyMode::DefaultDeny,
            rules,
        };
        let decision = policy.decision_for_tool("odos_swap");
        assert!(decision.allowed);
    }

    #[test]
    fn tool_name_validation_rejects_invalid() {
        assert!(is_valid_tool_name("odos_swap"));
        assert!(!is_valid_tool_name("odos-swap"));
        assert!(!is_valid_tool_name("Odos"));
        assert!(!is_valid_tool_name(""));
    }
}
