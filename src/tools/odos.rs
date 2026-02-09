//! Odos DEX aggregator tool
//!
//! Wraps the odos-sdk crate to provide swap quotes, transaction preparation,
//! and real-time token pricing.
//!
//! SECURITY NOTE:
//! - This tool only prepares transactions, it NEVER signs them
//! - Signing happens in the SecureWallet module after interceptor approval
//! - The tool has no access to private keys

use crate::tokens::{addresses, registry};
use crate::tools::{AnyJson, DefiBundle};
use alloy::primitives::{Address, U256};
use async_trait::async_trait;
use baml_rt::error::{BamlRtError, Result};
use baml_rt::tools::BamlTool;
use odos_sdk::{Chain, Slippage};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::str::FromStr;
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum OdosAction {
    Quote,
    PrepareSwap,
    GetPrice,
    GetPrices,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[ts(export)]
pub struct OdosInput {
    pub action: OdosAction,
    pub input_token: Option<String>,
    pub output_token: Option<String>,
    pub amount: Option<String>,
    pub token: Option<String>,
    pub tokens: Option<Vec<String>>,
    pub slippage_percent: Option<f64>,
    pub chain_id: Option<u64>,
    pub network: Option<String>,
}

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
    async fn get_quote(&self, args: &OdosInput) -> Result<Value> {
        let input_token = args
            .input_token
            .as_deref()
            .ok_or_else(|| BamlRtError::InvalidArgument("Missing 'input_token'".to_string()))?;

        let output_token = args
            .output_token
            .as_deref()
            .ok_or_else(|| BamlRtError::InvalidArgument("Missing 'output_token'".to_string()))?;

        let amount = args
            .amount
            .as_deref()
            .ok_or_else(|| BamlRtError::InvalidArgument("Missing 'amount'".to_string()))?;

        let chain_id = args.chain_id.unwrap_or(1);

        let slippage_percent = args.slippage_percent.unwrap_or(0.5);

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
    async fn prepare_swap(&self, args: &OdosInput) -> Result<Value> {
        let input_token = args
            .input_token
            .as_deref()
            .ok_or_else(|| BamlRtError::InvalidArgument("Missing 'input_token'".to_string()))?;

        let output_token = args
            .output_token
            .as_deref()
            .ok_or_else(|| BamlRtError::InvalidArgument("Missing 'output_token'".to_string()))?;

        let amount = args
            .amount
            .as_deref()
            .ok_or_else(|| BamlRtError::InvalidArgument("Missing 'amount'".to_string()))?;

        let chain_id = args.chain_id.unwrap_or(1);

        let slippage_percent = args.slippage_percent.unwrap_or(0.5);

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

    /// Get real-time token price in USD via Odos quote to USDC
    ///
    /// For stablecoins, returns $1 without making an API call.
    /// For other tokens, quotes 1 unit of the token to USDC.
    async fn get_price(&self, args: &OdosInput) -> Result<Value> {
        let token = args
            .token
            .as_deref()
            .ok_or_else(|| BamlRtError::InvalidArgument("Missing 'token'".to_string()))?;
        let chain_id = args.chain_id.unwrap_or(1);

        self.get_price_for_token(token, chain_id).await
    }

    async fn get_price_for_token(&self, token: &str, chain_id: u64) -> Result<Value> {
        let token_addr = Address::from_str(token)
            .map_err(|e| BamlRtError::InvalidArgument(format!("Invalid token address: {}", e)))?;

        // Check if it's a known stablecoin - return $1 immediately
        let token_registry = registry();
        if let Some(info) = token_registry.get(&token_addr) {
            if info.is_stablecoin {
                return Ok(json!({
                    "action": "get_price",
                    "token": token,
                    "symbol": info.symbol,
                    "price_usd": 1.0,
                    "source": "stablecoin",
                    "chain_id": chain_id,
                }));
            }
        }

        // Get the USDC address for this chain
        let usdc_addr = Self::usdc_for_chain(chain_id).ok_or_else(|| {
            BamlRtError::InvalidArgument(format!("No USDC address for chain {}", chain_id))
        })?;

        // Get token decimals (default to 18 for unknown tokens)
        let decimals = token_registry
            .get(&token_addr)
            .map(|info| info.decimals)
            .unwrap_or(18);

        // Quote 1 unit of the token to USDC
        let one_unit = U256::from(10).pow(U256::from(decimals));

        let chain = Self::chain_from_id(chain_id).ok_or_else(|| {
            BamlRtError::InvalidArgument(format!("Unsupported chain ID: {}", chain_id))
        })?;

        // Use a small slippage just for price discovery
        let slippage = Slippage::percent(1.0)
            .map_err(|e| BamlRtError::InvalidArgument(format!("Invalid slippage: {}", e)))?;

        let quote = self
            .client
            .swap()
            .chain(chain)
            .from_token(token_addr, one_unit)
            .to_token(usdc_addr)
            .slippage(slippage)
            .signer(self.wallet_address)
            .quote()
            .await
            .map_err(|e| BamlRtError::ToolExecution(format!("Odos price quote failed: {}", e)))?;

        // Parse USDC output amount (6 decimals)
        let usdc_out_str = quote.out_amount().unwrap_or(&"0".to_string()).to_owned();
        let usdc_out: f64 = usdc_out_str.parse().unwrap_or(0.0);
        let price_usd = usdc_out / 1_000_000.0; // USDC has 6 decimals

        let symbol = token_registry
            .get(&token_addr)
            .map(|info| info.symbol)
            .unwrap_or("UNKNOWN");

        Ok(json!({
            "action": "get_price",
            "token": token,
            "symbol": symbol,
            "price_usd": price_usd,
            "source": "odos_quote",
            "chain_id": chain_id,
            "price_impact_percent": quote.price_impact(),
        }))
    }

    /// Get multiple token prices in a single call
    async fn get_prices(&self, args: &OdosInput) -> Result<Value> {
        let tokens = args
            .tokens
            .as_ref()
            .ok_or_else(|| BamlRtError::InvalidArgument("Missing 'tokens' array".to_string()))?;

        let chain_id = args.chain_id.unwrap_or(1);

        let mut prices = Vec::new();
        for token in tokens {
            match self.get_price_for_token(token, chain_id).await {
                Ok(price_result) => prices.push(price_result),
                Err(e) => {
                    // Include error but don't fail the whole batch
                    prices.push(json!({
                        "token": token,
                        "error": e.to_string(),
                    }));
                }
            }
        }

        Ok(json!({
            "action": "get_prices",
            "chain_id": chain_id,
            "prices": prices,
        }))
    }

    /// Get USDC address for a chain
    fn usdc_for_chain(chain_id: u64) -> Option<Address> {
        match chain_id {
            1 => Some(addresses::USDC_ETH),
            42161 => Some(addresses::USDC_ARB),
            10 => Some(addresses::USDC_OPT),
            8453 => Some(addresses::USDC_BASE),
            _ => None,
        }
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
    type Bundle = DefiBundle;
    const LOCAL_NAME: &'static str = "odos_swap";
    type OpenInput = ();
    type Input = OdosInput;
    type Output = AnyJson;

    fn description(&self) -> &'static str {
        "Interacts with Odos DEX aggregator for optimal swap routing and real-time pricing. \
         Actions: 'quote' (read-only swap quote), 'prepare_swap' (prepare transaction), \
         'get_price' (get token USD price via quote), 'get_prices' (batch price lookup). \
         Supports Ethereum, Arbitrum, Optimism, and Base networks."
    }

    async fn execute(&self, args: Self::Input) -> Result<Self::Output> {
        let mut args = args;
        if let Some(network) = args.network.as_deref() {
            let chain_id = Self::parse_chain_id(network);
            args.chain_id = Some(chain_id);
        }

        let result = match args.action {
            OdosAction::Quote => self.get_quote(&args).await?,
            OdosAction::PrepareSwap => self.prepare_swap(&args).await?,
            OdosAction::GetPrice => self.get_price(&args).await?,
            OdosAction::GetPrices => self.get_prices(&args).await?,
        };

        Ok(AnyJson::new(result))
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

    #[test]
    fn test_usdc_for_chain() {
        // Ethereum mainnet
        let usdc_eth = OdosTool::usdc_for_chain(1);
        assert!(usdc_eth.is_some());
        assert_eq!(usdc_eth.unwrap(), addresses::USDC_ETH);

        // Arbitrum
        let usdc_arb = OdosTool::usdc_for_chain(42161);
        assert!(usdc_arb.is_some());
        assert_eq!(usdc_arb.unwrap(), addresses::USDC_ARB);

        // Optimism
        let usdc_opt = OdosTool::usdc_for_chain(10);
        assert!(usdc_opt.is_some());
        assert_eq!(usdc_opt.unwrap(), addresses::USDC_OPT);

        // Base
        let usdc_base = OdosTool::usdc_for_chain(8453);
        assert!(usdc_base.is_some());
        assert_eq!(usdc_base.unwrap(), addresses::USDC_BASE);

        // Unknown chain
        let usdc_unknown = OdosTool::usdc_for_chain(999);
        assert!(usdc_unknown.is_none());
    }

    #[test]
    fn test_chain_from_id() {
        // Supported chains
        assert!(OdosTool::chain_from_id(1).is_some()); // Ethereum
        assert!(OdosTool::chain_from_id(42161).is_some()); // Arbitrum
        assert!(OdosTool::chain_from_id(10).is_some()); // Optimism
        assert!(OdosTool::chain_from_id(8453).is_some()); // Base
        assert!(OdosTool::chain_from_id(137).is_some()); // Polygon
        assert!(OdosTool::chain_from_id(43114).is_some()); // Avalanche
        assert!(OdosTool::chain_from_id(56).is_some()); // BSC

        // Unsupported chain
        assert!(OdosTool::chain_from_id(999).is_none());
    }

    #[test]
    fn test_input_schema_includes_price_actions() {
        let tool = OdosTool::new("0x0000000000000000000000000000000000000000");
        let schema = tool.input_schema();

        // Check that action enum includes price actions
        fn collect_actions(
            schema_root: &serde_json::Value,
            action_schema: &serde_json::Value,
        ) -> Vec<String> {
            let mut actions: Vec<String> = Vec::new();

            if let Some(action_enum) = action_schema.get("enum").and_then(|v| v.as_array()) {
                actions.extend(
                    action_enum
                        .iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string())),
                );
                return actions;
            }

            if let Some(one_of) = action_schema.get("oneOf").and_then(|v| v.as_array()) {
                for entry in one_of {
                    if let Some(value) = entry.get("const").and_then(|v| v.as_str()) {
                        actions.push(value.to_string());
                    } else if let Some(values) = entry.get("enum").and_then(|v| v.as_array()) {
                        actions.extend(
                            values
                                .iter()
                                .filter_map(|v| v.as_str().map(|s| s.to_string())),
                        );
                    }
                }
                return actions;
            }

            if let Some(reference) = action_schema.get("$ref").and_then(|v| v.as_str()) {
                if let Some(def_name) = reference.rsplit('/').next() {
                    let defs = schema_root
                        .get("$defs")
                        .or_else(|| schema_root.get("definitions"));
                    if let Some(def_schema) = defs.and_then(|d| d.get(def_name)) {
                        return collect_actions(schema_root, def_schema);
                    }
                }
            }

            actions
        }

        let action_schema = &schema["properties"]["action"];
        let actions = collect_actions(&schema, action_schema);
        assert!(actions.iter().any(|v| v == "get_price"));
        assert!(actions.iter().any(|v| v == "get_prices"));

        // Check that price-specific properties exist
        assert!(schema["properties"]["token"].is_object());
        assert!(schema["properties"]["tokens"].is_object());
    }
}
