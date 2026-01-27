//! RPC endpoint configuration
//!
//! Supports multiple configuration methods following Ethereum ecosystem conventions:
//! 1. Per-chain env vars (ETH_RPC_URL, ARBITRUM_RPC_URL, etc.) - highest priority
//! 2. Provider API keys (ALCHEMY_API_KEY, INFURA_API_KEY) - builds URLs automatically
//! 3. Public RPC fallbacks - for testing only
//!
//! # Examples
//!
//! ```bash
//! # Option 1: Per-chain URLs (recommended for production)
//! export ETH_RPC_URL="https://eth-mainnet.g.alchemy.com/v2/YOUR_KEY"
//! export ARBITRUM_RPC_URL="https://arb-mainnet.g.alchemy.com/v2/YOUR_KEY"
//!
//! # Option 2: Single provider API key
//! export ALCHEMY_API_KEY="YOUR_KEY"
//!
//! # Option 3: No env vars - uses public RPCs (rate limited, for testing only)
//! ```

use std::collections::HashMap;

/// RPC configuration for multiple chains
#[derive(Debug, Clone)]
pub struct RpcConfig {
    /// RPC URLs indexed by chain ID
    urls: HashMap<u64, String>,
}

/// Chain ID constants
pub mod chains {
    pub const ETHEREUM: u64 = 1;
    pub const ARBITRUM: u64 = 42161;
    pub const OPTIMISM: u64 = 10;
    pub const BASE: u64 = 8453;
    pub const POLYGON: u64 = 137;
}

/// Environment variable names
mod env_vars {
    // Per-chain URLs (highest priority)
    pub const ETH_RPC_URL: &str = "ETH_RPC_URL";
    pub const ARBITRUM_RPC_URL: &str = "ARBITRUM_RPC_URL";
    pub const OPTIMISM_RPC_URL: &str = "OPTIMISM_RPC_URL";
    pub const BASE_RPC_URL: &str = "BASE_RPC_URL";
    pub const POLYGON_RPC_URL: &str = "POLYGON_RPC_URL";

    // Provider API keys
    pub const ALCHEMY_API_KEY: &str = "ALCHEMY_API_KEY";
    pub const INFURA_API_KEY: &str = "INFURA_API_KEY";
    pub const QUICKNODE_API_KEY: &str = "QUICKNODE_API_KEY";
    pub const QUICKNODE_SUBDOMAIN: &str = "QUICKNODE_SUBDOMAIN";
}

/// Public RPC endpoints (rate limited, for testing only)
mod public_rpcs {
    pub const ETHEREUM: &str = "https://eth.llamarpc.com";
    pub const ARBITRUM: &str = "https://arb1.arbitrum.io/rpc";
    pub const OPTIMISM: &str = "https://mainnet.optimism.io";
    pub const BASE: &str = "https://mainnet.base.org";
    pub const POLYGON: &str = "https://polygon-rpc.com";
}

impl RpcConfig {
    /// Create RPC config from environment variables
    ///
    /// Priority:
    /// 1. Per-chain env vars (ETH_RPC_URL, ARBITRUM_RPC_URL, etc.)
    /// 2. ALCHEMY_API_KEY - builds URLs for all chains
    /// 3. INFURA_API_KEY - builds URLs for supported chains
    /// 4. Public RPC fallbacks (for testing only)
    pub fn from_env() -> Self {
        let mut urls = HashMap::new();

        // Priority 1: Check per-chain env vars
        if let Ok(url) = std::env::var(env_vars::ETH_RPC_URL) {
            tracing::debug!("Using ETH_RPC_URL for Ethereum");
            urls.insert(chains::ETHEREUM, url);
        }
        if let Ok(url) = std::env::var(env_vars::ARBITRUM_RPC_URL) {
            tracing::debug!("Using ARBITRUM_RPC_URL for Arbitrum");
            urls.insert(chains::ARBITRUM, url);
        }
        if let Ok(url) = std::env::var(env_vars::OPTIMISM_RPC_URL) {
            tracing::debug!("Using OPTIMISM_RPC_URL for Optimism");
            urls.insert(chains::OPTIMISM, url);
        }
        if let Ok(url) = std::env::var(env_vars::BASE_RPC_URL) {
            tracing::debug!("Using BASE_RPC_URL for Base");
            urls.insert(chains::BASE, url);
        }
        if let Ok(url) = std::env::var(env_vars::POLYGON_RPC_URL) {
            tracing::debug!("Using POLYGON_RPC_URL for Polygon");
            urls.insert(chains::POLYGON, url);
        }

        // Priority 2: If no per-chain vars, try ALCHEMY_API_KEY
        if urls.is_empty() {
            if let Ok(key) = std::env::var(env_vars::ALCHEMY_API_KEY) {
                tracing::info!("Building RPC URLs from ALCHEMY_API_KEY");
                urls.insert(
                    chains::ETHEREUM,
                    format!("https://eth-mainnet.g.alchemy.com/v2/{}", key),
                );
                urls.insert(
                    chains::ARBITRUM,
                    format!("https://arb-mainnet.g.alchemy.com/v2/{}", key),
                );
                urls.insert(
                    chains::OPTIMISM,
                    format!("https://opt-mainnet.g.alchemy.com/v2/{}", key),
                );
                urls.insert(
                    chains::BASE,
                    format!("https://base-mainnet.g.alchemy.com/v2/{}", key),
                );
                urls.insert(
                    chains::POLYGON,
                    format!("https://polygon-mainnet.g.alchemy.com/v2/{}", key),
                );
            }
        }

        // Priority 3: If no Alchemy, try INFURA_API_KEY
        if urls.is_empty() {
            if let Ok(key) = std::env::var(env_vars::INFURA_API_KEY) {
                tracing::info!("Building RPC URLs from INFURA_API_KEY");
                urls.insert(
                    chains::ETHEREUM,
                    format!("https://mainnet.infura.io/v3/{}", key),
                );
                urls.insert(
                    chains::ARBITRUM,
                    format!("https://arbitrum-mainnet.infura.io/v3/{}", key),
                );
                urls.insert(
                    chains::OPTIMISM,
                    format!("https://optimism-mainnet.infura.io/v3/{}", key),
                );
                urls.insert(
                    chains::POLYGON,
                    format!("https://polygon-mainnet.infura.io/v3/{}", key),
                );
                // Note: Infura doesn't support Base
            }
        }

        // Priority 4: Try QUICKNODE (requires subdomain + optional API key)
        if urls.is_empty() {
            if let Ok(subdomain) = std::env::var(env_vars::QUICKNODE_SUBDOMAIN) {
                tracing::info!("Building RPC URLs from QUICKNODE_SUBDOMAIN");
                // QuickNode URL format: https://<subdomain>.quiknode.pro/<api_key>
                let api_key = std::env::var(env_vars::QUICKNODE_API_KEY).unwrap_or_default();
                let key_suffix = if api_key.is_empty() {
                    String::new()
                } else {
                    format!("/{}", api_key)
                };

                // QuickNode endpoint naming varies - using common patterns
                // Users should use per-chain URLs for more control
                urls.insert(
                    chains::ETHEREUM,
                    format!("https://{}.quiknode.pro{}", subdomain, key_suffix),
                );
                // Note: QuickNode uses separate endpoints per chain, so users typically
                // need different subdomains. Recommend using per-chain URLs for QuickNode.
            }
        }

        // Priority 5: Fall back to public RPCs for any missing chains
        if !urls.contains_key(&chains::ETHEREUM) {
            tracing::warn!("No RPC configured for Ethereum, using public RPC (rate limited)");
        }
        urls.entry(chains::ETHEREUM)
            .or_insert_with(|| public_rpcs::ETHEREUM.to_string());
        urls.entry(chains::ARBITRUM)
            .or_insert_with(|| public_rpcs::ARBITRUM.to_string());
        urls.entry(chains::OPTIMISM)
            .or_insert_with(|| public_rpcs::OPTIMISM.to_string());
        urls.entry(chains::BASE)
            .or_insert_with(|| public_rpcs::BASE.to_string());
        urls.entry(chains::POLYGON)
            .or_insert_with(|| public_rpcs::POLYGON.to_string());

        Self { urls }
    }

    /// Create with explicit RPC URLs
    pub fn with_urls(urls: HashMap<u64, String>) -> Self {
        Self { urls }
    }

    /// Get RPC URL for a chain
    pub fn get(&self, chain_id: u64) -> Option<&str> {
        self.urls.get(&chain_id).map(|s| s.as_str())
    }

    /// Get all configured chain IDs
    pub fn chains(&self) -> impl Iterator<Item = &u64> {
        self.urls.keys()
    }

    /// Check if a chain is configured
    pub fn has_chain(&self, chain_id: u64) -> bool {
        self.urls.contains_key(&chain_id)
    }

    /// Convert to HashMap for WalletTool
    pub fn to_hashmap(&self) -> HashMap<u64, String> {
        self.urls.clone()
    }
}

impl Default for RpcConfig {
    fn default() -> Self {
        Self::from_env()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_has_all_chains() {
        // Clear env vars for test
        std::env::remove_var(env_vars::ETH_RPC_URL);
        std::env::remove_var(env_vars::ALCHEMY_API_KEY);
        std::env::remove_var(env_vars::INFURA_API_KEY);

        let config = RpcConfig::from_env();

        assert!(config.has_chain(chains::ETHEREUM));
        assert!(config.has_chain(chains::ARBITRUM));
        assert!(config.has_chain(chains::OPTIMISM));
        assert!(config.has_chain(chains::BASE));
    }

    #[test]
    fn test_get_returns_url() {
        let mut urls = HashMap::new();
        urls.insert(1, "https://custom.rpc".to_string());
        let config = RpcConfig::with_urls(urls);

        assert_eq!(config.get(1), Some("https://custom.rpc"));
        assert_eq!(config.get(999), None);
    }

    #[test]
    fn test_public_rpc_fallbacks() {
        // Clear env vars
        std::env::remove_var(env_vars::ETH_RPC_URL);
        std::env::remove_var(env_vars::ALCHEMY_API_KEY);

        let config = RpcConfig::from_env();

        // Should fall back to public RPCs
        assert_eq!(config.get(chains::ETHEREUM), Some(public_rpcs::ETHEREUM));
        assert_eq!(config.get(chains::ARBITRUM), Some(public_rpcs::ARBITRUM));
    }
}
