//! Wallet balance query tool
//!
//! Queries native ETH and ERC20 token balances from blockchain RPCs.
//! Uses the shared token registry and RPC configuration.
//!
//! SECURITY NOTE:
//! - This tool is READ-ONLY - it only queries balances
//! - It never accesses or exposes private keys
//! - The wallet address is public information

use crate::config::RpcConfig;
use crate::tokens::{self, TokenInfo};
use crate::tools::{AnyJson, DefiBundle};
use alloy::primitives::{Address, Bytes, U256};
use alloy::providers::{Provider, ProviderBuilder};
use alloy::rpc::types::TransactionRequest;
use async_trait::async_trait;
use baml_rt::error::{BamlRtError, Result};
use baml_rt::tools::BamlTool;
use futures::future::join_all;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::str::FromStr;
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum WalletAction {
    NativeBalance,
    TokenBalance,
    AllBalances,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[ts(export)]
pub struct WalletInput {
    pub action: WalletAction,
    pub network: Option<String>,
    pub chain_id: Option<u64>,
    pub token_address: Option<String>,
}

/// Tool for querying wallet balances
pub struct WalletTool {
    /// Wallet address to query
    wallet_address: Address,
    /// RPC URLs per chain ID
    rpc_urls: HashMap<u64, String>,
}

impl WalletTool {
    /// Create a new WalletTool with RPC config from environment
    pub fn new(wallet_address: &str) -> std::result::Result<Self, String> {
        let rpc_config = RpcConfig::from_env();
        Self::with_rpc_config(wallet_address, &rpc_config)
    }

    /// Create a WalletTool with explicit RPC configuration
    pub fn with_rpc_config(
        wallet_address: &str,
        rpc_config: &RpcConfig,
    ) -> std::result::Result<Self, String> {
        let addr = Address::from_str(wallet_address)
            .map_err(|e| format!("Invalid wallet address: {}", e))?;

        Ok(Self {
            wallet_address: addr,
            rpc_urls: rpc_config.to_hashmap(),
        })
    }

    /// Create with custom RPC URLs (for testing)
    pub fn with_rpc_urls(
        wallet_address: &str,
        rpc_urls: HashMap<u64, String>,
    ) -> std::result::Result<Self, String> {
        let addr = Address::from_str(wallet_address)
            .map_err(|e| format!("Invalid wallet address: {}", e))?;

        Ok(Self {
            wallet_address: addr,
            rpc_urls,
        })
    }

    /// Get token info from the shared registry
    fn get_token_info(chain_id: u64, address: &Address) -> Option<&'static TokenInfo> {
        // First check the shared registry
        tokens::registry().get(address).or_else(|| {
            // Fall back to chain-specific lookup for tokens that might have
            // different addresses on different chains
            let addr_str = address.to_string().to_lowercase();

            // WETH has the same address on Optimism and Base
            if addr_str == "0x4200000000000000000000000000000000000006" {
                match chain_id {
                    10 | 8453 => tokens::registry().get(&tokens::addresses::WETH_OPT),
                    _ => None,
                }
            } else {
                None
            }
        })
    }

    /// Convert chain name to chain ID
    fn parse_chain_id(network: &str) -> u64 {
        match network.to_lowercase().as_str() {
            "ethereum" | "mainnet" => 1,
            "arbitrum" => 42161,
            "optimism" => 10,
            "base" => 8453,
            _ => 1, // Default to mainnet
        }
    }

    /// Get native ETH balance
    async fn get_native_balance(&self, chain_id: u64) -> Result<Value> {
        let rpc_url = self.rpc_urls.get(&chain_id).ok_or_else(|| {
            BamlRtError::InvalidArgument(format!("No RPC URL configured for chain {}", chain_id))
        })?;

        let url: url::Url = rpc_url
            .parse()
            .map_err(|e| BamlRtError::ToolExecution(format!("Invalid RPC URL: {}", e)))?;

        let provider = ProviderBuilder::new().connect_http(url);

        let balance = provider
            .get_balance(self.wallet_address)
            .await
            .map_err(|e| BamlRtError::ToolExecution(format!("Failed to get balance: {}", e)))?;

        // Convert to ETH (18 decimals)
        let balance_eth = format_units(balance, 18);

        Ok(json!({
            "token": "ETH",
            "symbol": "ETH",
            "balance_raw": balance.to_string(),
            "balance_formatted": balance_eth,
            "decimals": 18,
            "chain_id": chain_id,
            "is_native": true
        }))
    }

    /// Get ERC20 token balance using eth_call
    async fn get_token_balance(&self, chain_id: u64, token_address: &str) -> Result<Value> {
        let rpc_url = self.rpc_urls.get(&chain_id).ok_or_else(|| {
            BamlRtError::InvalidArgument(format!("No RPC URL configured for chain {}", chain_id))
        })?;

        let token_addr = Address::from_str(token_address)
            .map_err(|e| BamlRtError::InvalidArgument(format!("Invalid token address: {}", e)))?;

        let url: url::Url = rpc_url
            .parse()
            .map_err(|e| BamlRtError::ToolExecution(format!("Invalid RPC URL: {}", e)))?;

        let provider = ProviderBuilder::new().connect_http(url);

        // ERC20 balanceOf(address) selector: 0x70a08231
        // Encode: selector + padded address
        let mut calldata = vec![0x70, 0xa0, 0x82, 0x31]; // balanceOf selector
        calldata.extend_from_slice(&[0u8; 12]); // pad address to 32 bytes
        calldata.extend_from_slice(self.wallet_address.as_slice());

        let tx = TransactionRequest::default()
            .to(token_addr)
            .input(Bytes::from(calldata).into());

        let result = provider.call(tx).await.map_err(|e| {
            BamlRtError::ToolExecution(format!("Failed to get token balance: {}", e))
        })?;

        // Decode U256 from result bytes
        let balance = if result.len() >= 32 {
            U256::from_be_slice(&result[..32])
        } else {
            U256::ZERO
        };

        // Get token info from shared registry
        let (decimals, symbol) = if let Some(info) = Self::get_token_info(chain_id, &token_addr) {
            (info.decimals, info.symbol.to_string())
        } else {
            // Default to 18 decimals and unknown symbol
            (18, "UNKNOWN".to_string())
        };

        let balance_formatted = format_units(balance, decimals as u32);

        Ok(json!({
            "token": token_address,
            "symbol": symbol,
            "balance_raw": balance.to_string(),
            "balance_formatted": balance_formatted,
            "decimals": decimals,
            "chain_id": chain_id,
            "is_native": false
        }))
    }

    /// Get balances for all common tokens on a network (parallelized)
    async fn get_all_balances(&self, chain_id: u64) -> Result<Value> {
        // Get native ETH balance first
        let native_balance = self.get_native_balance(chain_id).await;

        // Get tokens for this chain from the shared registry
        let token_addresses = tokens::registry().tokens_for_chain(chain_id);

        // Convert to owned strings to avoid lifetime issues with async closures
        let token_addr_strings: Vec<String> =
            token_addresses.iter().map(|a| a.to_string()).collect();
        let num_tokens = token_addr_strings.len();

        // Query all token balances in parallel
        let token_futures: Vec<_> = token_addr_strings
            .iter()
            .map(|addr_str| self.get_token_balance(chain_id, addr_str))
            .collect();

        let token_results = join_all(token_futures).await;

        // Collect results
        let mut balances = Vec::new();

        // Add native balance if successful
        match native_balance {
            Ok(bal) => balances.push(bal),
            Err(e) => {
                tracing::warn!("Failed to get native balance: {}", e);
            }
        }

        // Add token balances
        for result in token_results {
            match result {
                Ok(bal) => balances.push(bal),
                Err(e) => {
                    tracing::warn!("Failed to get token balance: {}", e);
                }
            }
        }

        // Filter out zero balances for cleaner output
        let non_zero_balances: Vec<Value> = balances
            .into_iter()
            .filter(|b| {
                b.get("balance_raw")
                    .and_then(|v| v.as_str())
                    .map(|s| s != "0")
                    .unwrap_or(false)
            })
            .collect();

        Ok(json!({
            "wallet": self.wallet_address.to_string(),
            "chain_id": chain_id,
            "balances": non_zero_balances,
            "total_tokens_checked": num_tokens + 1 // +1 for native
        }))
    }
}

/// Format a U256 value with decimals
fn format_units(value: U256, decimals: u32) -> String {
    if value.is_zero() {
        return "0".to_string();
    }

    let divisor = U256::from(10).pow(U256::from(decimals));
    let whole = value / divisor;
    let remainder = value % divisor;

    if remainder.is_zero() {
        whole.to_string()
    } else {
        // Format with decimal places
        let remainder_str = format!("{:0>width$}", remainder, width = decimals as usize);
        let trimmed = remainder_str.trim_end_matches('0');
        if trimmed.is_empty() {
            whole.to_string()
        } else {
            format!("{}.{}", whole, trimmed)
        }
    }
}

#[async_trait]
impl BamlTool for WalletTool {
    type Bundle = DefiBundle;
    const LOCAL_NAME: &'static str = "wallet_balance";
    type OpenInput = ();
    type Input = WalletInput;
    type Output = AnyJson;

    fn description(&self) -> &'static str {
        "Queries wallet balances for native ETH and ERC20 tokens. \
         Supports Ethereum, Arbitrum, Optimism, and Base networks. \
         Read-only operation that never accesses private keys."
    }

    async fn execute(&self, args: Self::Input) -> Result<Self::Output> {
        // Get chain_id from either chain_id or network
        let chain_id = if let Some(id) = args.chain_id {
            id
        } else if let Some(network) = args.network.as_deref() {
            Self::parse_chain_id(network)
        } else {
            1 // Default to Ethereum mainnet
        };

        let result = match args.action {
            WalletAction::NativeBalance => self.get_native_balance(chain_id).await?,
            WalletAction::TokenBalance => {
                let token_address = args.token_address.ok_or_else(|| {
                    BamlRtError::InvalidArgument(
                        "Missing 'token_address' for token_balance action".to_string(),
                    )
                })?;
                self.get_token_balance(chain_id, &token_address).await?
            }
            WalletAction::AllBalances => self.get_all_balances(chain_id).await?,
        };

        Ok(AnyJson::new(result))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_units() {
        // 1 ETH = 1e18 wei
        let one_eth = U256::from(1_000_000_000_000_000_000u128);
        assert_eq!(format_units(one_eth, 18), "1");

        // 1.5 ETH
        let one_point_five = U256::from(1_500_000_000_000_000_000u128);
        assert_eq!(format_units(one_point_five, 18), "1.5");

        // 1000 USDC (6 decimals)
        let thousand_usdc = U256::from(1_000_000_000u64);
        assert_eq!(format_units(thousand_usdc, 6), "1000");

        // 0
        assert_eq!(format_units(U256::ZERO, 18), "0");
    }

    #[test]
    fn test_parse_chain_id() {
        assert_eq!(WalletTool::parse_chain_id("ethereum"), 1);
        assert_eq!(WalletTool::parse_chain_id("arbitrum"), 42161);
        assert_eq!(WalletTool::parse_chain_id("optimism"), 10);
        assert_eq!(WalletTool::parse_chain_id("base"), 8453);
        assert_eq!(WalletTool::parse_chain_id("mainnet"), 1);
    }

    #[test]
    fn test_input_schema() {
        let mut urls = HashMap::new();
        urls.insert(1, "https://test.rpc".to_string());
        let tool =
            WalletTool::with_rpc_urls("0x0000000000000000000000000000000000000000", urls).unwrap();
        let schema = tool.input_schema();

        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["action"].is_object());
        assert!(schema["properties"]["network"].is_object());
        assert!(schema["properties"]["token_address"].is_object());
    }

    #[test]
    fn test_get_token_info_from_registry() {
        // USDC on Ethereum should be found
        let usdc_info = WalletTool::get_token_info(1, &tokens::addresses::USDC_ETH);
        assert!(usdc_info.is_some());
        assert_eq!(usdc_info.unwrap().symbol, "USDC");
        assert_eq!(usdc_info.unwrap().decimals, 6);

        // WETH on Arbitrum should be found
        let weth_info = WalletTool::get_token_info(42161, &tokens::addresses::WETH_ARB);
        assert!(weth_info.is_some());
        assert_eq!(weth_info.unwrap().symbol, "WETH");
        assert_eq!(weth_info.unwrap().decimals, 18);
    }
}
