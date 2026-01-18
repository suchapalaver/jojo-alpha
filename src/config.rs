//! Configuration for the DeFi trading agent

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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

/// The Graph subgraph endpoints
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubgraphEndpoints {
    pub endpoints: HashMap<(Network, Protocol), String>,
}

impl Default for SubgraphEndpoints {
    fn default() -> Self {
        let mut endpoints = HashMap::new();

        // Uniswap V3
        endpoints.insert(
            (Network::Ethereum, Protocol::UniswapV3),
            "https://api.thegraph.com/subgraphs/name/uniswap/uniswap-v3".to_string(),
        );
        endpoints.insert(
            (Network::Arbitrum, Protocol::UniswapV3),
            "https://api.thegraph.com/subgraphs/name/ianlapham/uniswap-arbitrum-one".to_string(),
        );

        Self { endpoints }
    }
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
}

impl Default for RiskConfig {
    fn default() -> Self {
        Self {
            max_trade_usd: 100.0,      // Conservative default
            max_daily_usd: 500.0,      // Conservative default
            max_slippage_percent: 1.0, // 1% max slippage
            cooldown_seconds: 300,     // 5 minutes between trades
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
            check_interval_ms: 60_000, // 1 minute
            audit_log_path: Some("audit.jsonl".to_string()),
        }
    }
}
