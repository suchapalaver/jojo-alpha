//! Configuration for the DeFi trading agent

pub mod rpc;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// Re-export RPC config
pub use rpc::RpcConfig;

/// The Graph API key environment variable name
pub const GRAPH_API_KEY_ENV: &str = "GRAPH_API_KEY";

/// Supported blockchain networks
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Network {
    Ethereum,
    Arbitrum,
    Optimism,
    Base,
}

impl Network {
    pub fn chain_id(&self) -> u64 {
        match self {
            Network::Ethereum => 1,
            Network::Arbitrum => 42161,
            Network::Optimism => 10,
            Network::Base => 8453,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Network::Ethereum => "ethereum",
            Network::Arbitrum => "arbitrum",
            Network::Optimism => "optimism",
            Network::Base => "base",
        }
    }
}

/// Supported DeFi protocols
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Protocol {
    UniswapV3,
    AaveV3,
}

impl Protocol {
    pub fn name(&self) -> &'static str {
        match self {
            Protocol::UniswapV3 => "uniswap_v3",
            Protocol::AaveV3 => "aave_v3",
        }
    }
}

/// The Graph subgraph IDs for decentralized network
pub struct SubgraphIds;

impl SubgraphIds {
    /// Uniswap V3 subgraph IDs on The Graph decentralized network
    pub const UNISWAP_V3_ETHEREUM: &'static str = "5zvR82QoaXYFyDEKLZ9t6v9adgnptxYpKpSbxtgVENFV";
    pub const UNISWAP_V3_ARBITRUM: &'static str = "FbCGRftH4a3yZugY7TnbYgPJVEv2LvMT6oF1fxPe9aJM";
    pub const UNISWAP_V3_OPTIMISM: &'static str = "Cghf4LfVqPiFw6fp6Y5X5Ubc8UpmUhSfJL82zwiBFLaj";
    pub const UNISWAP_V3_BASE: &'static str = "43Hwfi3dJSoGpyas9VwNoDAv28pNwMgNGVi8CKNS9r6R";
}

/// The Graph subgraph endpoints
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubgraphEndpoints {
    pub endpoints: HashMap<(Network, Protocol), String>,
}

impl SubgraphEndpoints {
    /// Build endpoints using The Graph decentralized network with API key
    pub fn with_api_key(api_key: &str) -> Self {
        let mut endpoints = HashMap::new();

        // Uniswap V3 on The Graph decentralized network
        endpoints.insert(
            (Network::Ethereum, Protocol::UniswapV3),
            format!(
                "https://gateway.thegraph.com/api/{}/subgraphs/id/{}",
                api_key,
                SubgraphIds::UNISWAP_V3_ETHEREUM
            ),
        );
        endpoints.insert(
            (Network::Arbitrum, Protocol::UniswapV3),
            format!(
                "https://gateway.thegraph.com/api/{}/subgraphs/id/{}",
                api_key,
                SubgraphIds::UNISWAP_V3_ARBITRUM
            ),
        );
        endpoints.insert(
            (Network::Optimism, Protocol::UniswapV3),
            format!(
                "https://gateway.thegraph.com/api/{}/subgraphs/id/{}",
                api_key,
                SubgraphIds::UNISWAP_V3_OPTIMISM
            ),
        );
        endpoints.insert(
            (Network::Base, Protocol::UniswapV3),
            format!(
                "https://gateway.thegraph.com/api/{}/subgraphs/id/{}",
                api_key,
                SubgraphIds::UNISWAP_V3_BASE
            ),
        );

        Self { endpoints }
    }

    /// Try to build endpoints from GRAPH_API_KEY environment variable
    pub fn from_env() -> Option<Self> {
        std::env::var(GRAPH_API_KEY_ENV)
            .ok()
            .map(|key| Self::with_api_key(&key))
    }
}

impl Default for SubgraphEndpoints {
    fn default() -> Self {
        // Try to load from environment, fall back to placeholder
        Self::from_env().unwrap_or_else(|| {
            let mut endpoints = HashMap::new();
            // Placeholder - requires GRAPH_API_KEY to be set
            endpoints.insert(
                (Network::Ethereum, Protocol::UniswapV3),
                "https://gateway.thegraph.com/api/YOUR_API_KEY/subgraphs/id/5zvR82QoaXYFyDEKLZ9t6v9adgnptxYpKpSbxtgVENFV".to_string(),
            );
            Self { endpoints }
        })
    }
}

/// Spend limit enforcement mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SpendLimitMode {
    /// Allow trades when USD value cannot be determined (log warning)
    #[default]
    FailOpen,
    /// Block trades when USD value cannot be determined (safer)
    FailClosed,
}

/// Default policy behavior when policy.json is missing
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PolicyDefaultMode {
    #[default]
    AllowAll,
    DefaultDeny,
}

/// Policy settings for tool execution
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PolicySettings {
    /// Default mode when policy.json is missing
    #[serde(default)]
    pub default_mode: PolicyDefaultMode,
    /// Require policy.json to be present (fail closed if missing)
    #[serde(default)]
    pub require_file: bool,
}

/// Risk management configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskConfig {
    /// Maximum value per single trade (USD)
    pub max_trade_usd: f64,
    /// Maximum daily spending (USD)
    pub max_daily_usd: f64,
    /// Maximum slippage tolerance (e.g., 0.5 for 0.5%)
    pub max_slippage_percent: f64,
    /// Minimum seconds between trades
    pub cooldown_seconds: u64,
    /// Spend limit enforcement mode
    #[serde(default)]
    pub spend_limit_mode: SpendLimitMode,
}

impl Default for RiskConfig {
    fn default() -> Self {
        Self {
            max_trade_usd: 100.0,                       // Conservative default
            max_daily_usd: 500.0,                       // Conservative default
            max_slippage_percent: 1.0,                  // 1% max slippage
            cooldown_seconds: 300,                      // 5 minutes between trades
            spend_limit_mode: SpendLimitMode::FailOpen, // Default to existing behavior
        }
    }
}

/// Main configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Networks to monitor
    pub networks: Vec<Network>,
    /// Protocols to query
    pub protocols: Vec<Protocol>,
    /// Subgraph endpoints
    pub subgraphs: SubgraphEndpoints,
    /// Risk management settings
    pub risk: RiskConfig,
    /// Policy settings
    #[serde(default)]
    pub policy: PolicySettings,
    /// Trading loop interval (milliseconds)
    pub check_interval_ms: u64,
    /// Path to audit log file
    pub audit_log_path: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            networks: vec![Network::Ethereum, Network::Arbitrum],
            protocols: vec![Protocol::UniswapV3],
            subgraphs: SubgraphEndpoints::default(),
            risk: RiskConfig::default(),
            policy: PolicySettings::default(),
            check_interval_ms: 60_000, // 1 minute
            audit_log_path: Some("audit.jsonl".to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_settings_deserialize_defaults() {
        let value = serde_json::json!({
            "networks": ["ethereum"],
            "protocols": ["uniswap_v3"],
            "subgraphs": { "endpoints": {} },
            "risk": {
                "max_trade_usd": 100.0,
                "max_daily_usd": 500.0,
                "max_slippage_percent": 1.0,
                "cooldown_seconds": 300,
                "spend_limit_mode": "fail_open"
            },
            "check_interval_ms": 60000,
            "audit_log_path": "audit.jsonl"
        });
        let parsed: Config = serde_json::from_value(value).expect("parse config");
        assert_eq!(parsed.policy.default_mode, PolicyDefaultMode::AllowAll);
        assert!(!parsed.policy.require_file);
    }

    #[test]
    fn policy_settings_deserialize_explicit() {
        let value = serde_json::json!({
            "networks": ["ethereum"],
            "protocols": ["uniswap_v3"],
            "subgraphs": { "endpoints": {} },
            "risk": {
                "max_trade_usd": 100.0,
                "max_daily_usd": 500.0,
                "max_slippage_percent": 1.0,
                "cooldown_seconds": 300,
                "spend_limit_mode": "fail_open"
            },
            "policy": {
                "default_mode": "default_deny",
                "require_file": true
            },
            "check_interval_ms": 60000,
            "audit_log_path": "audit.jsonl"
        });
        let parsed: Config = serde_json::from_value(value).expect("parse config");
        assert_eq!(parsed.policy.default_mode, PolicyDefaultMode::DefaultDeny);
        assert!(parsed.policy.require_file);
    }
}
