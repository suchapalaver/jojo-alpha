//! Wallet balance query tool
//!
//! Queries native ETH and ERC20 token balances from blockchain RPCs.
//!
//! SECURITY NOTE:
//! - This tool is READ-ONLY - it only queries balances
//! - It never accesses or exposes private keys
//! - The wallet address is public information

use alloy::primitives::{Address, Bytes, U256};
use alloy::providers::{Provider, ProviderBuilder};
use alloy::rpc::types::TransactionRequest;
use async_trait::async_trait;
use baml_rt::error::{BamlRtError, Result};
use baml_rt::tools::BamlTool;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::str::FromStr;

/// Well-known token addresses and metadata
struct TokenInfo {
    symbol: &'static str,
    decimals: u8,
}

/// Tool for querying wallet balances
pub struct WalletTool {
    /// Wallet address to query
    wallet_address: Address,
    /// RPC URLs per chain ID
    rpc_urls: HashMap<u64, String>,
}

impl WalletTool {
    /// Create a new WalletTool with default public RPC endpoints
    pub fn new(wallet_address: &str) -> std::result::Result<Self, String> {
        let addr = Address::from_str(wallet_address)
            .map_err(|e| format!("Invalid wallet address: {}", e))?;

        let mut rpc_urls = HashMap::new();
        // Default public RPC endpoints (rate-limited, for testing)
        // In production, use private RPC providers like Alchemy, Infura, etc.
        rpc_urls.insert(1, "https://eth.llamarpc.com".to_string());
        rpc_urls.insert(42161, "https://arb1.arbitrum.io/rpc".to_string());
        rpc_urls.insert(10, "https://mainnet.optimism.io".to_string());
        rpc_urls.insert(8453, "https://mainnet.base.org".to_string());

        Ok(Self {
            wallet_address: addr,
            rpc_urls,
        })
    }

    /// Create with custom RPC URLs
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

    /// Get well-known token info for common tokens
    fn get_token_info(chain_id: u64, address: &Address) -> Option<TokenInfo> {
        // Stablecoins and major tokens with known decimals
        let addr_str = address.to_string().to_lowercase();

        match chain_id {
            1 => {
                // Ethereum mainnet
                match addr_str.as_str() {
                    "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48" => Some(TokenInfo {
                        symbol: "USDC",
                        decimals: 6,
                    }),
                    "0xdac17f958d2ee523a2206206994597c13d831ec7" => Some(TokenInfo {
                        symbol: "USDT",
                        decimals: 6,
                    }),
                    "0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2" => Some(TokenInfo {
                        symbol: "WETH",
                        decimals: 18,
                    }),
                    "0x6b175474e89094c44da98b954eedeac495271d0f" => Some(TokenInfo {
                        symbol: "DAI",
                        decimals: 18,
                    }),
                    "0x2260fac5e5542a773aa44fbcfedf7c193bc2c599" => Some(TokenInfo {
                        symbol: "WBTC",
                        decimals: 8,
                    }),
                    _ => None,
                }
            }
            42161 => {
                // Arbitrum
                match addr_str.as_str() {
                    "0xaf88d065e77c8cc2239327c5edb3a432268e5831" => Some(TokenInfo {
                        symbol: "USDC",
                        decimals: 6,
                    }),
                    "0xfd086bc7cd5c481dcc9c85ebe478a1c0b69fcbb9" => Some(TokenInfo {
                        symbol: "USDT",
                        decimals: 6,
                    }),
                    "0x82af49447d8a07e3bd95bd0d56f35241523fbab1" => Some(TokenInfo {
                        symbol: "WETH",
                        decimals: 18,
                    }),
                    _ => None,
                }
            }
            10 => {
                // Optimism
                match addr_str.as_str() {
                    "0x0b2c639c533813f4aa9d7837caf62653d097ff85" => Some(TokenInfo {
                        symbol: "USDC",
                        decimals: 6,
                    }),
                    "0x94b008aa00579c1307b0ef2c499ad98a8ce58e58" => Some(TokenInfo {
                        symbol: "USDT",
                        decimals: 6,
                    }),
                    "0x4200000000000000000000000000000000000006" => Some(TokenInfo {
                        symbol: "WETH",
                        decimals: 18,
                    }),
                    _ => None,
                }
            }
            8453 => {
                // Base
                match addr_str.as_str() {
                    "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913" => Some(TokenInfo {
                        symbol: "USDC",
                        decimals: 6,
                    }),
                    "0x4200000000000000000000000000000000000006" => Some(TokenInfo {
                        symbol: "WETH",
                        decimals: 18,
                    }),
                    _ => None,
                }
            }
            _ => None,
        }
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

        // Get token info from known tokens or use defaults
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

    /// Get balances for all common tokens on a network
    async fn get_all_balances(&self, chain_id: u64) -> Result<Value> {
        let mut balances = Vec::new();

        // Get native ETH balance
        match self.get_native_balance(chain_id).await {
            Ok(bal) => balances.push(bal),
            Err(e) => {
                tracing::warn!("Failed to get native balance: {}", e);
            }
        }

        // Get common token balances based on chain
        let tokens: Vec<&str> = match chain_id {
            1 => vec![
                "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48", // USDC
                "0xdac17f958d2ee523a2206206994597c13d831ec7", // USDT
                "0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2", // WETH
                "0x6b175474e89094c44da98b954eedeac495271d0f", // DAI
            ],
            42161 => vec![
                "0xaf88d065e77c8cc2239327c5edb3a432268e5831", // USDC
                "0xfd086bc7cd5c481dcc9c85ebe478a1c0b69fcbb9", // USDT
                "0x82af49447d8a07e3bd95bd0d56f35241523fbab1", // WETH
            ],
            10 => vec![
                "0x0b2c639c533813f4aa9d7837caf62653d097ff85", // USDC
                "0x94b008aa00579c1307b0ef2c499ad98a8ce58e58", // USDT
                "0x4200000000000000000000000000000000000006", // WETH
            ],
            8453 => vec![
                "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913", // USDC
                "0x4200000000000000000000000000000000000006", // WETH
            ],
            _ => vec![],
        };

        // Store count before iterating
        let token_count = tokens.len();

        for token in tokens {
            match self.get_token_balance(chain_id, token).await {
                Ok(bal) => balances.push(bal),
                Err(e) => {
                    tracing::warn!("Failed to get balance for {}: {}", token, e);
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
            "total_tokens_checked": token_count + 1 // +1 for native
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
    const NAME: &'static str = "wallet_balance";

    fn description(&self) -> &'static str {
        "Queries wallet balances for native ETH and ERC20 tokens. \
         Supports Ethereum, Arbitrum, Optimism, and Base networks. \
         Read-only operation that never accesses private keys."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["native_balance", "token_balance", "all_balances"],
                    "description": "Action to perform: 'native_balance' for ETH, 'token_balance' for specific ERC20, 'all_balances' for common tokens"
                },
                "network": {
                    "type": "string",
                    "enum": ["ethereum", "arbitrum", "optimism", "base"],
                    "description": "Network to query"
                },
                "chain_id": {
                    "type": "integer",
                    "description": "Chain ID (alternative to network): 1=Ethereum, 42161=Arbitrum, 10=Optimism, 8453=Base"
                },
                "token_address": {
                    "type": "string",
                    "description": "ERC20 token address (required for 'token_balance' action)"
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| BamlRtError::InvalidArgument("Missing 'action' field".to_string()))?;

        // Get chain_id from either chain_id or network
        let chain_id = if let Some(id) = args.get("chain_id").and_then(|v| v.as_u64()) {
            id
        } else if let Some(network) = args.get("network").and_then(|v| v.as_str()) {
            Self::parse_chain_id(network)
        } else {
            1 // Default to Ethereum mainnet
        };

        match action {
            "native_balance" => self.get_native_balance(chain_id).await,
            "token_balance" => {
                let token_address = args
                    .get("token_address")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        BamlRtError::InvalidArgument(
                            "Missing 'token_address' for token_balance action".to_string(),
                        )
                    })?;
                self.get_token_balance(chain_id, token_address).await
            }
            "all_balances" => self.get_all_balances(chain_id).await,
            _ => Err(BamlRtError::InvalidArgument(format!(
                "Unknown action: {}. Use 'native_balance', 'token_balance', or 'all_balances'",
                action
            ))),
        }
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
        let tool = WalletTool::new("0x0000000000000000000000000000000000000000").unwrap();
        let schema = tool.input_schema();

        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["action"].is_object());
        assert!(schema["properties"]["network"].is_object());
        assert!(schema["properties"]["token_address"].is_object());
    }

    #[test]
    fn test_get_token_info() {
        // USDC on Ethereum
        let usdc_eth = Address::from_str("0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48").unwrap();
        let info = WalletTool::get_token_info(1, &usdc_eth).unwrap();
        assert_eq!(info.symbol, "USDC");
        assert_eq!(info.decimals, 6);

        // WETH on Arbitrum
        let weth_arb = Address::from_str("0x82af49447d8a07e3bd95bd0d56f35241523fbab1").unwrap();
        let info = WalletTool::get_token_info(42161, &weth_arb).unwrap();
        assert_eq!(info.symbol, "WETH");
        assert_eq!(info.decimals, 18);
    }
}
