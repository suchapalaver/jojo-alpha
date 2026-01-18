//! Odos DEX aggregator tool
//!
//! Wraps the odos-sdk crate to provide swap quotes and transaction preparation.
//!
//! SECURITY NOTE:
//! - This tool only prepares transactions, it NEVER signs them
//! - Signing happens in the SecureWallet module after interceptor approval
//! - The tool has no access to private keys

use alloy::primitives::{Address, U256};
use async_trait::async_trait;
use baml_rt::error::{BamlRtError, Result};
use baml_rt::tools::BamlTool;
use odos_sdk::{Chain, Slippage};
use serde_json::{json, Value};
use std::str::FromStr;

/// Tool for interacting with Odos DEX aggregator
///
/// Provides two actions:
/// - `quote`: Get a swap quote (read-only, safe)
/// - `prepare_swap`: Prepare transaction data (requires interceptor approval)
pub struct OdosTool {
    /// Odos SDK client
    client: odos_sdk::OdosClient,
    /// Wallet address (public, safe to share)
    wallet_address: Address,
}

impl OdosTool {
    /// Create a new OdosTool
    ///
    /// # Arguments
    /// * `wallet_address` - The public address of the wallet (for quote requests)
    ///
    /// # Panics
    /// Panics if the wallet address is invalid or if the Odos client fails to initialize
    pub fn new(wallet_address: &str) -> Self {
        let addr = Address::from_str(wallet_address).expect("Invalid wallet address");
        Self {
            client: odos_sdk::OdosClient::new().expect("Failed to create Odos client"),
            wallet_address: addr,
        }
    }

    /// Create a new OdosTool with error handling
    ///
    /// # Arguments
    /// * `wallet_address` - The public address of the wallet (for quote requests)
    pub fn try_new(wallet_address: &str) -> Result<Self> {
        let addr = Address::from_str(wallet_address)
            .map_err(|e| BamlRtError::InvalidArgument(format!("Invalid wallet address: {}", e)))?;
        let client = odos_sdk::OdosClient::new().map_err(|e| {
            BamlRtError::ToolExecution(format!("Failed to create Odos client: {}", e))
        })?;
        Ok(Self {
            client,
            wallet_address: addr,
        })
    }

    /// Get a swap quote from Odos using the SwapBuilder API
    async fn get_quote(&self, args: &Value) -> Result<Value> {
        let input_token = args
            .get("input_token")
            .and_then(|v| v.as_str())
            .ok_or_else(|| BamlRtError::InvalidArgument("Missing 'input_token'".to_string()))?;

        let output_token = args
            .get("output_token")
            .and_then(|v| v.as_str())
            .ok_or_else(|| BamlRtError::InvalidArgument("Missing 'output_token'".to_string()))?;

        let amount = args
            .get("amount")
            .and_then(|v| v.as_str())
            .ok_or_else(|| BamlRtError::InvalidArgument("Missing 'amount'".to_string()))?;

        let chain_id = args.get("chain_id").and_then(|v| v.as_u64()).unwrap_or(1);

        let slippage_percent = args
            .get("slippage_percent")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.5);

        // Parse token addresses and amount
        let input_addr = Address::from_str(input_token).map_err(|e| {
            BamlRtError::InvalidArgument(format!("Invalid input token address: {}", e))
        })?;
        let output_addr = Address::from_str(output_token).map_err(|e| {
            BamlRtError::InvalidArgument(format!("Invalid output token address: {}", e))
        })?;
        let amount_u256 = U256::from_str(amount)
            .map_err(|e| BamlRtError::InvalidArgument(format!("Invalid amount: {}", e)))?;

        // Get chain from chain_id
        let chain = Self::chain_from_id(chain_id).ok_or_else(|| {
            BamlRtError::InvalidArgument(format!("Unsupported chain ID: {}", chain_id))
        })?;

        // Create slippage
        let slippage = Slippage::percent(slippage_percent)
            .map_err(|e| BamlRtError::InvalidArgument(format!("Invalid slippage: {}", e)))?;

        // Use SwapBuilder to get quote
        let quote = self
            .client
            .swap()
            .chain(chain)
            .from_token(input_addr, amount_u256)
            .to_token(output_addr)
            .slippage(slippage)
            .signer(self.wallet_address)
            .quote()
            .await
            .map_err(|e| BamlRtError::ToolExecution(format!("Odos quote failed: {}", e)))?;

        Ok(json!({
            "action": "quote",
            "input_token": input_token,
            "output_token": output_token,
            "input_amount": amount,
            "output_amount": quote.out_amount().unwrap_or(&"0".to_string()),
            "price_impact_percent": quote.price_impact(),
            "gas_estimate": quote.gas_estimate(),
            "path_id": quote.path_id(),
        }))
    }

    /// Prepare a swap transaction (does NOT sign or submit)
    async fn prepare_swap(&self, args: &Value) -> Result<Value> {
        let input_token = args
            .get("input_token")
            .and_then(|v| v.as_str())
            .ok_or_else(|| BamlRtError::InvalidArgument("Missing 'input_token'".to_string()))?;

        let output_token = args
            .get("output_token")
            .and_then(|v| v.as_str())
            .ok_or_else(|| BamlRtError::InvalidArgument("Missing 'output_token'".to_string()))?;

        let amount = args
            .get("amount")
            .and_then(|v| v.as_str())
            .ok_or_else(|| BamlRtError::InvalidArgument("Missing 'amount'".to_string()))?;

        let chain_id = args.get("chain_id").and_then(|v| v.as_u64()).unwrap_or(1);

        let slippage_percent = args
            .get("slippage_percent")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.5);

        // Parse token addresses and amount
        let input_addr = Address::from_str(input_token).map_err(|e| {
            BamlRtError::InvalidArgument(format!("Invalid input token address: {}", e))
        })?;
        let output_addr = Address::from_str(output_token).map_err(|e| {
            BamlRtError::InvalidArgument(format!("Invalid output token address: {}", e))
        })?;
        let amount_u256 = U256::from_str(amount)
            .map_err(|e| BamlRtError::InvalidArgument(format!("Invalid amount: {}", e)))?;

        // Get chain from chain_id
        let chain = Self::chain_from_id(chain_id).ok_or_else(|| {
            BamlRtError::InvalidArgument(format!("Unsupported chain ID: {}", chain_id))
        })?;

        // Create slippage
        let slippage = Slippage::percent(slippage_percent)
            .map_err(|e| BamlRtError::InvalidArgument(format!("Invalid slippage: {}", e)))?;

        // Use SwapBuilder to build transaction
        let tx = self
            .client
            .swap()
            .chain(chain)
            .from_token(input_addr, amount_u256)
            .to_token(output_addr)
            .slippage(slippage)
            .signer(self.wallet_address)
            .build_transaction()
            .await
            .map_err(|e| {
                BamlRtError::ToolExecution(format!("Odos transaction build failed: {}", e))
            })?;

        // Get quote for details
        let quote = self
            .client
            .swap()
            .chain(chain)
            .from_token(input_addr, amount_u256)
            .to_token(output_addr)
            .slippage(slippage)
            .signer(self.wallet_address)
            .quote()
            .await
            .map_err(|e| BamlRtError::ToolExecution(format!("Odos quote failed: {}", e)))?;

        // Extract transaction fields
        let to_address = tx
            .to
            .and_then(|kind| kind.to().map(|a| a.to_string()))
            .unwrap_or_default();
        let input_data = tx
            .input
            .input
            .as_ref()
            .map(|b| format!("{}", b))
            .unwrap_or_default();
        let value_str = tx
            .value
            .map(|v| v.to_string())
            .unwrap_or_else(|| "0".to_string());

        // Return the prepared transaction - NOT signed
        Ok(json!({
            "action": "prepare_swap",
            "status": "prepared_pending_execution",
            "transaction": {
                "to": to_address,
                "data": input_data,
                "value": value_str,
                "gas_limit": tx.gas,
                "chain_id": chain_id,
            },
            "quote_details": {
                "input_token": input_token,
                "output_token": output_token,
                "input_amount": amount,
                "expected_output": quote.out_amount().unwrap_or(&"0".to_string()),
                "price_impact_percent": quote.price_impact(),
            },
            "path_id": quote.path_id(),
            "note": "Transaction prepared but NOT signed. Requires interceptor approval and wallet signature."
        }))
    }

    fn parse_chain_id(network: &str) -> u64 {
        match network.to_lowercase().as_str() {
            "ethereum" | "mainnet" => 1,
            "arbitrum" => 42161,
            "optimism" => 10,
            "base" => 8453,
            _ => 1, // Default to mainnet
        }
    }

    /// Convert chain ID to Chain type
    fn chain_from_id(chain_id: u64) -> Option<Chain> {
        match chain_id {
            1 => Some(Chain::ethereum()),
            42161 => Some(Chain::arbitrum()),
            10 => Some(Chain::optimism()),
            8453 => Some(Chain::base()),
            137 => Some(Chain::polygon()),
            43114 => Some(Chain::avalanche()),
            56 => Some(Chain::bsc()),
            _ => None,
        }
    }
}

#[async_trait]
impl BamlTool for OdosTool {
    const NAME: &'static str = "odos_swap";

    fn description(&self) -> &'static str {
        "Interacts with Odos DEX aggregator for optimal swap routing. \
         Can get quotes (safe, read-only) or prepare swap transactions \
         (requires approval through risk interceptors). Supports Ethereum, \
         Arbitrum, Optimism, and Base networks."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["quote", "prepare_swap"],
                    "description": "Action to perform. 'quote' is read-only; 'prepare_swap' prepares a transaction."
                },
                "input_token": {
                    "type": "string",
                    "description": "Input token address (use 0xEeeeeEeeeEeEeeEeEeEeeEEEeeeeEeeeeeeeEEeE for native ETH)"
                },
                "output_token": {
                    "type": "string",
                    "description": "Output token address"
                },
                "amount": {
                    "type": "string",
                    "description": "Input amount in wei (as string for precision)"
                },
                "slippage_percent": {
                    "type": "number",
                    "description": "Maximum slippage tolerance (e.g., 0.5 for 0.5%)",
                    "default": 0.5
                },
                "chain_id": {
                    "type": "integer",
                    "description": "Chain ID (1=Ethereum, 42161=Arbitrum, 10=Optimism, 8453=Base)",
                    "default": 1
                },
                "network": {
                    "type": "string",
                    "enum": ["ethereum", "arbitrum", "optimism", "base"],
                    "description": "Network name (alternative to chain_id)"
                }
            },
            "required": ["action", "input_token", "output_token", "amount"]
        })
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| BamlRtError::InvalidArgument("Missing 'action' field".to_string()))?;

        // If network is provided instead of chain_id, convert it
        let mut args = args.clone();
        if let Some(network) = args.get("network").and_then(|v| v.as_str()) {
            let chain_id = Self::parse_chain_id(network);
            args["chain_id"] = json!(chain_id);
        }

        match action {
            "quote" => self.get_quote(&args).await,
            "prepare_swap" => self.prepare_swap(&args).await,
            _ => Err(BamlRtError::InvalidArgument(format!(
                "Unknown action: {}. Use 'quote' or 'prepare_swap'",
                action
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_chain_id() {
        assert_eq!(OdosTool::parse_chain_id("ethereum"), 1);
        assert_eq!(OdosTool::parse_chain_id("arbitrum"), 42161);
        assert_eq!(OdosTool::parse_chain_id("optimism"), 10);
        assert_eq!(OdosTool::parse_chain_id("base"), 8453);
    }

    #[test]
    fn test_input_schema() {
        let tool = OdosTool::new("0x0000000000000000000000000000000000000000");
        let schema = tool.input_schema();

        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["action"].is_object());
        assert!(schema["properties"]["input_token"].is_object());
        assert!(schema["properties"]["output_token"].is_object());
        assert!(schema["properties"]["amount"].is_object());
    }
}
