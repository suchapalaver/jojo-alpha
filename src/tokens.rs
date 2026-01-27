//! Shared token registry
//!
//! Centralizes token metadata (addresses, decimals, symbols) to avoid duplication
//! between Rust modules (spend_limit.rs, wallet.rs) and ensure consistency.
//!
//! This module is the single source of truth for token information.

use alloy::primitives::{address, Address};
use std::collections::HashMap;

/// Token metadata
#[derive(Debug, Clone, Copy)]
pub struct TokenInfo {
    /// Token symbol (e.g., "USDC", "WETH")
    pub symbol: &'static str,
    /// Number of decimals
    pub decimals: u8,
    /// Whether this is a stablecoin (pegged to $1)
    pub is_stablecoin: bool,
    /// Approximate USD price (for non-stablecoins, used as fallback only)
    /// This is only used when no price oracle is available
    pub approx_price_usd: Option<f64>,
}

impl TokenInfo {
    /// Create a stablecoin token info
    pub const fn stablecoin(symbol: &'static str, decimals: u8) -> Self {
        Self {
            symbol,
            decimals,
            is_stablecoin: true,
            approx_price_usd: Some(1.0),
        }
    }

    /// Create a non-stablecoin token info
    pub const fn token(symbol: &'static str, decimals: u8, approx_price: Option<f64>) -> Self {
        Self {
            symbol,
            decimals,
            is_stablecoin: false,
            approx_price_usd: approx_price,
        }
    }
}

/// Chain ID constants (re-exported from config::rpc)
pub mod chains {
    pub const ETHEREUM: u64 = 1;
    pub const ARBITRUM: u64 = 42161;
    pub const OPTIMISM: u64 = 10;
    pub const BASE: u64 = 8453;
}

/// Well-known token addresses per chain
pub mod addresses {
    use super::*;

    // === Ethereum Mainnet ===
    pub const USDC_ETH: Address = address!("a0b86991c6218b36c1d19d4a2e9eb0ce3606eb48");
    pub const USDT_ETH: Address = address!("dac17f958d2ee523a2206206994597c13d831ec7");
    pub const DAI_ETH: Address = address!("6b175474e89094c44da98b954eedeac495271d0f");
    pub const WETH_ETH: Address = address!("c02aaa39b223fe8d0a0e5c4f27ead9083c756cc2");
    pub const WBTC_ETH: Address = address!("2260fac5e5542a773aa44fbcfedf7c193bc2c599");

    // === Arbitrum ===
    pub const USDC_ARB: Address = address!("af88d065e77c8cc2239327c5edb3a432268e5831");
    pub const USDC_E_ARB: Address = address!("ff970a61a04b1ca14834a43f5de4533ebddb5cc8"); // Bridged
    pub const USDT_ARB: Address = address!("fd086bc7cd5c481dcc9c85ebe478a1c0b69fcbb9");
    pub const DAI_ARB: Address = address!("da10009cbd5d07dd0cecc66161fc93d7c9000da1");
    pub const WETH_ARB: Address = address!("82af49447d8a07e3bd95bd0d56f35241523fbab1");

    // === Optimism ===
    pub const USDC_OPT: Address = address!("0b2c639c533813f4aa9d7837caf62653d097ff85");
    pub const USDC_E_OPT: Address = address!("7f5c764cbc14f9669b88837ca1490cca17c31607"); // Bridged
    pub const USDT_OPT: Address = address!("94b008aa00579c1307b0ef2c499ad98a8ce58e58");
    pub const WETH_OPT: Address = address!("4200000000000000000000000000000000000006");

    // === Base ===
    pub const USDC_BASE: Address = address!("833589fcd6edb6e08f4c7c32d4f71b54bda02913");
    pub const DAI_BASE: Address = address!("50c5725949a6f0c72e6c4a641f24049a917db0cb");
    pub const WETH_BASE: Address = address!("4200000000000000000000000000000000000006");

    // === Native ETH representations ===
    pub const NATIVE_ETH: Address = address!("eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee");
    pub const ZERO_ADDRESS: Address = address!("0000000000000000000000000000000000000000");
}

/// Token registry providing token info lookups
pub struct TokenRegistry {
    /// Token info by address (chain-independent for now, addresses are unique)
    tokens: HashMap<Address, TokenInfo>,
    /// Tokens per chain for balance queries
    tokens_per_chain: HashMap<u64, Vec<Address>>,
}

impl TokenRegistry {
    /// Create a new token registry with all known tokens
    pub fn new() -> Self {
        use addresses::*;

        let mut tokens = HashMap::new();

        // Stablecoins
        tokens.insert(USDC_ETH, TokenInfo::stablecoin("USDC", 6));
        tokens.insert(USDC_ARB, TokenInfo::stablecoin("USDC", 6));
        tokens.insert(USDC_E_ARB, TokenInfo::stablecoin("USDC.e", 6));
        tokens.insert(USDC_OPT, TokenInfo::stablecoin("USDC", 6));
        tokens.insert(USDC_E_OPT, TokenInfo::stablecoin("USDC.e", 6));
        tokens.insert(USDC_BASE, TokenInfo::stablecoin("USDC", 6));

        tokens.insert(USDT_ETH, TokenInfo::stablecoin("USDT", 6));
        tokens.insert(USDT_ARB, TokenInfo::stablecoin("USDT", 6));
        tokens.insert(USDT_OPT, TokenInfo::stablecoin("USDT", 6));

        tokens.insert(DAI_ETH, TokenInfo::stablecoin("DAI", 18));
        tokens.insert(DAI_ARB, TokenInfo::stablecoin("DAI", 18));
        tokens.insert(DAI_BASE, TokenInfo::stablecoin("DAI", 18));

        // Non-stablecoins (with approximate prices as fallback)
        tokens.insert(WETH_ETH, TokenInfo::token("WETH", 18, Some(3500.0)));
        tokens.insert(WETH_ARB, TokenInfo::token("WETH", 18, Some(3500.0)));
        tokens.insert(WETH_OPT, TokenInfo::token("WETH", 18, Some(3500.0)));
        tokens.insert(WETH_BASE, TokenInfo::token("WETH", 18, Some(3500.0)));

        tokens.insert(WBTC_ETH, TokenInfo::token("WBTC", 8, Some(95000.0)));

        // Native ETH representations
        tokens.insert(NATIVE_ETH, TokenInfo::token("ETH", 18, Some(3500.0)));
        tokens.insert(ZERO_ADDRESS, TokenInfo::token("ETH", 18, Some(3500.0)));

        // Build per-chain token lists for balance queries
        let mut tokens_per_chain = HashMap::new();

        tokens_per_chain.insert(
            chains::ETHEREUM,
            vec![USDC_ETH, USDT_ETH, WETH_ETH, DAI_ETH, WBTC_ETH],
        );
        tokens_per_chain.insert(
            chains::ARBITRUM,
            vec![USDC_ARB, USDT_ARB, WETH_ARB, DAI_ARB],
        );
        tokens_per_chain.insert(chains::OPTIMISM, vec![USDC_OPT, USDT_OPT, WETH_OPT]);
        tokens_per_chain.insert(chains::BASE, vec![USDC_BASE, WETH_BASE, DAI_BASE]);

        Self {
            tokens,
            tokens_per_chain,
        }
    }

    /// Get token info by address
    pub fn get(&self, address: &Address) -> Option<&TokenInfo> {
        self.tokens.get(address)
    }

    /// Get token info by address string (handles lowercase comparison)
    pub fn get_by_str(&self, address: &str) -> Option<&TokenInfo> {
        let addr = address.parse::<Address>().ok()?;
        self.get(&addr)
    }

    /// Get tokens to query for a chain
    pub fn tokens_for_chain(&self, chain_id: u64) -> &[Address] {
        self.tokens_per_chain
            .get(&chain_id)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Check if an address is a known stablecoin
    pub fn is_stablecoin(&self, address: &Address) -> bool {
        self.tokens
            .get(address)
            .map(|t| t.is_stablecoin)
            .unwrap_or(false)
    }

    /// Estimate USD value for a token amount
    ///
    /// Returns Some(usd_value) if we can estimate, None if unknown token
    pub fn estimate_usd_value(&self, address: &Address, amount_raw: &str) -> Option<f64> {
        let info = self.tokens.get(address)?;
        let amount: f64 = amount_raw.parse().ok()?;
        let divisor = 10_f64.powi(info.decimals as i32);
        let token_amount = amount / divisor;

        if info.is_stablecoin {
            Some(token_amount)
        } else {
            info.approx_price_usd.map(|price| token_amount * price)
        }
    }
}

impl Default for TokenRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Global token registry (lazy initialized)
static REGISTRY: std::sync::OnceLock<TokenRegistry> = std::sync::OnceLock::new();

/// Get the global token registry
pub fn registry() -> &'static TokenRegistry {
    REGISTRY.get_or_init(TokenRegistry::new)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_usdc_is_stablecoin() {
        let registry = TokenRegistry::new();
        assert!(registry.is_stablecoin(&addresses::USDC_ETH));
        assert!(registry.is_stablecoin(&addresses::USDT_ARB));
        assert!(registry.is_stablecoin(&addresses::DAI_ETH));
    }

    #[test]
    fn test_weth_not_stablecoin() {
        let registry = TokenRegistry::new();
        assert!(!registry.is_stablecoin(&addresses::WETH_ETH));
        assert!(!registry.is_stablecoin(&addresses::WBTC_ETH));
    }

    #[test]
    fn test_token_info() {
        let registry = TokenRegistry::new();

        let usdc = registry.get(&addresses::USDC_ETH).unwrap();
        assert_eq!(usdc.symbol, "USDC");
        assert_eq!(usdc.decimals, 6);
        assert!(usdc.is_stablecoin);

        let weth = registry.get(&addresses::WETH_ETH).unwrap();
        assert_eq!(weth.symbol, "WETH");
        assert_eq!(weth.decimals, 18);
        assert!(!weth.is_stablecoin);
    }

    #[test]
    fn test_estimate_usd_value() {
        let registry = TokenRegistry::new();

        // 100 USDC (6 decimals)
        let usdc_value = registry
            .estimate_usd_value(&addresses::USDC_ETH, "100000000")
            .unwrap();
        assert!((usdc_value - 100.0).abs() < 0.001);

        // 1 WETH (18 decimals) at $3500
        let weth_value = registry
            .estimate_usd_value(&addresses::WETH_ETH, "1000000000000000000")
            .unwrap();
        assert!((weth_value - 3500.0).abs() < 0.001);
    }

    #[test]
    fn test_tokens_for_chain() {
        let registry = TokenRegistry::new();

        let eth_tokens = registry.tokens_for_chain(chains::ETHEREUM);
        assert!(!eth_tokens.is_empty());
        assert!(eth_tokens.contains(&addresses::USDC_ETH));

        let arb_tokens = registry.tokens_for_chain(chains::ARBITRUM);
        assert!(arb_tokens.contains(&addresses::USDC_ARB));
    }

    #[test]
    fn test_global_registry() {
        let reg = registry();
        assert!(reg.get(&addresses::USDC_ETH).is_some());
    }
}
