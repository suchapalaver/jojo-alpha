//! The Graph subgraph query tool
//!
//! Queries DeFi protocol data from The Graph's subgraphs.

use crate::config::{Network, Protocol, SubgraphEndpoints};
use async_trait::async_trait;
use baml_rt::error::{BamlRtError, Result};
use baml_rt::tools::BamlTool;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Tool for querying The Graph subgraphs
pub struct TheGraphTool {
    client: Client,
    endpoints: SubgraphEndpoints,
}

impl TheGraphTool {
    /// Create a new TheGraphTool with default endpoints
    pub fn new() -> Self {
        Self {
            client: Client::new(),
            endpoints: SubgraphEndpoints::default(),
        }
    }

    /// Create with custom endpoints
    pub fn with_endpoints(endpoints: SubgraphEndpoints) -> Self {
        Self {
            client: Client::new(),
            endpoints,
        }
    }

    /// Execute a raw GraphQL query against a subgraph
    async fn query_subgraph(&self, endpoint: &str, query: &str, variables: Value) -> Result<Value> {
        let response = self
            .client
            .post(endpoint)
            .json(&json!({
                "query": query,
                "variables": variables
            }))
            .send()
            .await
            .map_err(|e| BamlRtError::ToolExecution(format!("GraphQL request failed: {}", e)))?;

        let result: GraphQLResponse = response.json().await.map_err(|e| {
            BamlRtError::ToolExecution(format!("Failed to parse GraphQL response: {}", e))
        })?;

        if let Some(errors) = result.errors {
            return Err(BamlRtError::ToolExecution(format!(
                "GraphQL errors: {:?}",
                errors
            )));
        }

        result
            .data
            .ok_or_else(|| BamlRtError::ToolExecution("No data in GraphQL response".to_string()))
    }

    /// Query top pools from Uniswap V3
    async fn query_uniswap_top_pools(&self, network: Network, limit: u32) -> Result<Value> {
        let endpoint = self
            .endpoints
            .endpoints
            .get(&(network, Protocol::UniswapV3))
            .ok_or_else(|| {
                BamlRtError::InvalidArgument(format!(
                    "No Uniswap V3 endpoint configured for {:?}",
                    network
                ))
            })?;

        let query = r#"
            query TopPools($first: Int!) {
                pools(
                    first: $first
                    orderBy: totalValueLockedUSD
                    orderDirection: desc
                ) {
                    id
                    token0 {
                        id
                        symbol
                        name
                        decimals
                    }
                    token1 {
                        id
                        symbol
                        name
                        decimals
                    }
                    feeTier
                    liquidity
                    sqrtPrice
                    token0Price
                    token1Price
                    volumeUSD
                    totalValueLockedUSD
                    txCount
                }
            }
        "#;

        let variables = json!({ "first": limit });
        let data = self.query_subgraph(endpoint, query, variables).await?;

        Ok(json!({
            "protocol": "uniswap_v3",
            "network": network.name(),
            "pools": data.get("pools").cloned().unwrap_or(json!([]))
        }))
    }

    /// Query a specific pool by ID
    async fn query_uniswap_pool(&self, network: Network, pool_id: &str) -> Result<Value> {
        let endpoint = self
            .endpoints
            .endpoints
            .get(&(network, Protocol::UniswapV3))
            .ok_or_else(|| {
                BamlRtError::InvalidArgument(format!(
                    "No Uniswap V3 endpoint configured for {:?}",
                    network
                ))
            })?;

        let query = r#"
            query PoolById($id: ID!) {
                pool(id: $id) {
                    id
                    token0 {
                        id
                        symbol
                        name
                        decimals
                        derivedETH
                    }
                    token1 {
                        id
                        symbol
                        name
                        decimals
                        derivedETH
                    }
                    feeTier
                    liquidity
                    sqrtPrice
                    tick
                    token0Price
                    token1Price
                    volumeUSD
                    totalValueLockedUSD
                    txCount
                }
            }
        "#;

        let variables = json!({ "id": pool_id });
        let data = self.query_subgraph(endpoint, query, variables).await?;

        Ok(json!({
            "protocol": "uniswap_v3",
            "network": network.name(),
            "pool": data.get("pool").cloned().unwrap_or(json!(null))
        }))
    }

    /// Query token price from Uniswap V3
    async fn query_token_price(&self, network: Network, token_address: &str) -> Result<Value> {
        let endpoint = self
            .endpoints
            .endpoints
            .get(&(network, Protocol::UniswapV3))
            .ok_or_else(|| {
                BamlRtError::InvalidArgument(format!(
                    "No Uniswap V3 endpoint configured for {:?}",
                    network
                ))
            })?;

        let query = r#"
            query TokenPrice($id: ID!) {
                token(id: $id) {
                    id
                    symbol
                    name
                    decimals
                    derivedETH
                    volumeUSD
                    totalValueLockedUSD
                }
                bundle(id: "1") {
                    ethPriceUSD
                }
            }
        "#;

        let variables = json!({ "id": token_address.to_lowercase() });
        let data = self.query_subgraph(endpoint, query, variables).await?;

        // Calculate USD price from ETH price
        let token = data.get("token");
        let bundle = data.get("bundle");

        let price_usd = match (token, bundle) {
            (Some(t), Some(b)) => {
                let derived_eth = t
                    .get("derivedETH")
                    .and_then(|v| v.as_str())
                    .and_then(|s| s.parse::<f64>().ok())
                    .unwrap_or(0.0);
                let eth_price = b
                    .get("ethPriceUSD")
                    .and_then(|v| v.as_str())
                    .and_then(|s| s.parse::<f64>().ok())
                    .unwrap_or(0.0);
                derived_eth * eth_price
            }
            _ => 0.0,
        };

        Ok(json!({
            "network": network.name(),
            "token": token.cloned().unwrap_or(json!(null)),
            "price_usd": price_usd,
            "eth_price_usd": bundle.and_then(|b| b.get("ethPriceUSD")).cloned().unwrap_or(json!(null))
        }))
    }

    fn parse_network(s: &str) -> Result<Network> {
        match s.to_lowercase().as_str() {
            "ethereum" | "mainnet" => Ok(Network::Ethereum),
            "arbitrum" => Ok(Network::Arbitrum),
            "optimism" => Ok(Network::Optimism),
            "base" => Ok(Network::Base),
            _ => Err(BamlRtError::InvalidArgument(format!(
                "Unknown network: {}. Supported: ethereum, arbitrum, optimism, base",
                s
            ))),
        }
    }
}

impl Default for TheGraphTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl BamlTool for TheGraphTool {
    const NAME: &'static str = "query_subgraph";

    fn description(&self) -> &'static str {
        "Queries DeFi protocol subgraphs (Uniswap V3) for pool data, liquidity, \
         prices, and trading volumes. Supports Ethereum and Arbitrum networks."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "protocol": {
                    "type": "string",
                    "enum": ["uniswap_v3"],
                    "description": "The DeFi protocol to query"
                },
                "network": {
                    "type": "string",
                    "enum": ["ethereum", "arbitrum", "optimism", "base"],
                    "description": "The blockchain network"
                },
                "query_type": {
                    "type": "string",
                    "enum": ["top_pools", "pool_info", "token_price"],
                    "description": "Type of data to retrieve"
                },
                "params": {
                    "type": "object",
                    "description": "Query-specific parameters",
                    "properties": {
                        "limit": {
                            "type": "integer",
                            "description": "Number of results for top_pools (default: 10)"
                        },
                        "pool_id": {
                            "type": "string",
                            "description": "Pool address for pool_info query"
                        },
                        "token_address": {
                            "type": "string",
                            "description": "Token address for token_price query"
                        }
                    }
                }
            },
            "required": ["protocol", "network", "query_type"]
        })
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let protocol = args
            .get("protocol")
            .and_then(|v| v.as_str())
            .ok_or_else(|| BamlRtError::InvalidArgument("Missing 'protocol' field".to_string()))?;

        let network_str = args
            .get("network")
            .and_then(|v| v.as_str())
            .ok_or_else(|| BamlRtError::InvalidArgument("Missing 'network' field".to_string()))?;

        let query_type = args
            .get("query_type")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                BamlRtError::InvalidArgument("Missing 'query_type' field".to_string())
            })?;

        let params = args.get("params").cloned().unwrap_or(json!({}));
        let network = Self::parse_network(network_str)?;

        match (protocol, query_type) {
            ("uniswap_v3", "top_pools") => {
                let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as u32;
                self.query_uniswap_top_pools(network, limit).await
            }
            ("uniswap_v3", "pool_info") => {
                let pool_id = params
                    .get("pool_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        BamlRtError::InvalidArgument("Missing 'pool_id' in params".to_string())
                    })?;
                self.query_uniswap_pool(network, pool_id).await
            }
            ("uniswap_v3", "token_price") => {
                let token_address = params
                    .get("token_address")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        BamlRtError::InvalidArgument(
                            "Missing 'token_address' in params".to_string(),
                        )
                    })?;
                self.query_token_price(network, token_address).await
            }
            _ => Err(BamlRtError::InvalidArgument(format!(
                "Unsupported query: {}/{}",
                protocol, query_type
            ))),
        }
    }
}

/// GraphQL response structure
#[derive(Debug, Deserialize)]
struct GraphQLResponse {
    data: Option<Value>,
    errors: Option<Vec<GraphQLError>>,
}

#[derive(Debug, Deserialize, Serialize)]
struct GraphQLError {
    message: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_network() {
        assert!(matches!(
            TheGraphTool::parse_network("ethereum"),
            Ok(Network::Ethereum)
        ));
        assert!(matches!(
            TheGraphTool::parse_network("arbitrum"),
            Ok(Network::Arbitrum)
        ));
        assert!(TheGraphTool::parse_network("invalid").is_err());
    }

    #[test]
    fn test_input_schema() {
        let tool = TheGraphTool::new();
        let schema = tool.input_schema();

        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["protocol"].is_object());
        assert!(schema["properties"]["network"].is_object());
        assert!(schema["properties"]["query_type"].is_object());
    }
}
