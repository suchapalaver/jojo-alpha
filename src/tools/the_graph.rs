//! The Graph subgraph query tool
//!
//! Queries DeFi protocol data from The Graph's subgraphs.
//! Supports query planning for bidirectional Graph-Inference flow.
//!
//! ## Gateway Integration
//!
//! When constructed with a gateway (`with_gateway`), queries are routed through
//! `BasicGraphGateway` which provides:
//! - **Caching**: Results cached with configurable TTL (default 60s)
//! - **Latency tracking**: Query performance metrics
//! - **Future x402 support**: Same interface for advanced routing

use crate::config::{Network, Protocol, SubgraphEndpoints, SubgraphIds};
use crate::tools::graph_gateway::{
    BasicGraphGateway, GatewayError, GraphGateway, QueryRoutingHints,
};
use async_trait::async_trait;
use baml_rt::error::{BamlRtError, Result};
use baml_rt::tools::BamlTool;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::Arc;

/// Query filters for intelligent data fetching (from InferQueryPlan)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct QueryFilters {
    #[serde(default)]
    pub min_tvl_usd: Option<f64>,
    #[serde(default)]
    pub min_volume_tvl_ratio: Option<f64>,
    #[serde(default)]
    pub token_pairs: Option<Vec<String>>,
    #[serde(default)]
    pub exclude_tokens: Option<Vec<String>>,
    #[serde(default)]
    pub min_volume_24h_usd: Option<f64>,
    #[serde(default)]
    pub fee_tiers: Option<Vec<u32>>,
}

/// Query plan from inference strategist (InferQueryPlan)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryPlan {
    pub target_networks: Vec<String>,
    pub target_protocols: Vec<String>,
    pub data_filters: QueryFilters,
    pub query_priority: u32,
    pub expected_data_points: u32,
}

/// Tool for querying The Graph subgraphs
///
/// Optionally uses a `GraphGateway` for caching and routing.
pub struct TheGraphTool {
    client: Client,
    endpoints: SubgraphEndpoints,
    /// Optional gateway for caching and x402 routing
    gateway: Option<Arc<dyn GraphGateway>>,
}

impl TheGraphTool {
    /// Create a new TheGraphTool with default endpoints (no caching)
    pub fn new() -> Self {
        Self {
            client: Client::new(),
            endpoints: SubgraphEndpoints::default(),
            gateway: None,
        }
    }

    /// Create with custom endpoints (no caching)
    pub fn with_endpoints(endpoints: SubgraphEndpoints) -> Self {
        Self {
            client: Client::new(),
            endpoints,
            gateway: None,
        }
    }

    /// Create with gateway for caching and x402 routing
    ///
    /// The gateway provides:
    /// - Query result caching with configurable TTL
    /// - Latency tracking and metrics
    /// - Future x402 protocol support
    ///
    /// # Arguments
    /// * `api_key` - The Graph API key for authentication
    pub fn with_gateway(api_key: String) -> Self {
        let endpoints = SubgraphEndpoints::with_api_key(&api_key);
        Self {
            client: Client::new(),
            endpoints,
            gateway: Some(Arc::new(BasicGraphGateway::new(api_key))),
        }
    }

    /// Create with custom endpoints and gateway
    pub fn with_endpoints_and_gateway(
        endpoints: SubgraphEndpoints,
        gateway: Arc<dyn GraphGateway>,
    ) -> Self {
        Self {
            client: Client::new(),
            endpoints,
            gateway: Some(gateway),
        }
    }

    /// Get the subgraph ID for a network/protocol combination
    #[allow(dead_code)] // Used in tests, may be useful for future direct lookups
    fn get_subgraph_id(network: Network, protocol: Protocol) -> Option<&'static str> {
        match (network, protocol) {
            (Network::Ethereum, Protocol::UniswapV3) => Some(SubgraphIds::UNISWAP_V3_ETHEREUM),
            (Network::Arbitrum, Protocol::UniswapV3) => Some(SubgraphIds::UNISWAP_V3_ARBITRUM),
            (Network::Optimism, Protocol::UniswapV3) => Some(SubgraphIds::UNISWAP_V3_OPTIMISM),
            (Network::Base, Protocol::UniswapV3) => Some(SubgraphIds::UNISWAP_V3_BASE),
            _ => None,
        }
    }

    /// Execute a raw GraphQL query against a subgraph
    ///
    /// If a gateway is configured, routes the query through the gateway for caching.
    /// Otherwise falls back to direct HTTP requests.
    async fn query_subgraph(&self, endpoint: &str, query: &str, variables: Value) -> Result<Value> {
        // Try to use gateway if available
        if let Some(ref gateway) = self.gateway {
            // Extract subgraph ID from endpoint URL
            // Format: https://gateway.thegraph.com/api/{api_key}/subgraphs/id/{subgraph_id}
            if let Some(subgraph_id) = Self::extract_subgraph_id(endpoint) {
                return self
                    .query_via_gateway(gateway, subgraph_id, query, variables)
                    .await;
            }
            // If we can't extract subgraph ID, fall through to direct query
            tracing::debug!(
                endpoint = endpoint,
                "Could not extract subgraph ID from endpoint, falling back to direct query"
            );
        }

        // Direct query (no gateway or couldn't extract subgraph ID)
        self.query_direct(endpoint, query, variables).await
    }

    /// Extract subgraph ID from a Graph API endpoint URL
    fn extract_subgraph_id(endpoint: &str) -> Option<&str> {
        // Format: https://gateway.thegraph.com/api/{api_key}/subgraphs/id/{subgraph_id}
        endpoint.rsplit("/subgraphs/id/").next().and_then(|s| {
            // Make sure we got something that looks like a subgraph ID
            if s.len() > 20 && !s.contains('/') {
                Some(s)
            } else {
                None
            }
        })
    }

    /// Query via gateway (with caching)
    async fn query_via_gateway(
        &self,
        gateway: &Arc<dyn GraphGateway>,
        subgraph_id: &str,
        query: &str,
        variables: Value,
    ) -> Result<Value> {
        let result = gateway
            .query_with_routing(subgraph_id, query, variables, QueryRoutingHints::default())
            .await
            .map_err(Self::gateway_error_to_baml_error)?;

        if result.cached {
            tracing::debug!(
                subgraph_id = subgraph_id,
                latency_ms = result.latency_ms,
                "Served from gateway cache"
            );
        } else {
            tracing::debug!(
                subgraph_id = subgraph_id,
                latency_ms = result.latency_ms,
                "Fresh query via gateway"
            );
        }

        Ok(result.data)
    }

    /// Direct HTTP query (no caching)
    async fn query_direct(&self, endpoint: &str, query: &str, variables: Value) -> Result<Value> {
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

    /// Convert gateway error to BAML error
    fn gateway_error_to_baml_error(e: GatewayError) -> BamlRtError {
        match e {
            GatewayError::HttpError(msg) => {
                BamlRtError::ToolExecution(format!("Gateway HTTP error: {}", msg))
            }
            GatewayError::GraphQLError(errors) => {
                BamlRtError::ToolExecution(format!("GraphQL errors: {}", errors.join(", ")))
            }
            GatewayError::NoData => {
                BamlRtError::ToolExecution("No data in GraphQL response".to_string())
            }
            GatewayError::SubgraphNotFound(id) => {
                BamlRtError::InvalidArgument(format!("Subgraph not found: {}", id))
            }
            GatewayError::AllIndexersFailed => {
                BamlRtError::ToolExecution("All indexers failed to respond".to_string())
            }
        }
    }

    /// Check if gateway caching is enabled
    pub fn has_gateway(&self) -> bool {
        self.gateway.is_some()
    }

    /// Get the gateway name (for logging/debugging)
    pub fn gateway_name(&self) -> Option<&'static str> {
        self.gateway.as_ref().map(|g| g.name())
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

    /// Query pools with filters applied (for bidirectional Graph-Inference flow)
    async fn query_filtered_pools(
        &self,
        network: Network,
        filters: &QueryFilters,
        limit: u32,
    ) -> Result<Value> {
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

        // Build GraphQL where clause from filters
        let mut where_clauses = Vec::new();

        if let Some(min_tvl) = filters.min_tvl_usd {
            where_clauses.push(format!("totalValueLockedUSD_gte: \"{}\"", min_tvl));
        }

        if let Some(min_vol) = filters.min_volume_24h_usd {
            where_clauses.push(format!("volumeUSD_gte: \"{}\"", min_vol));
        }

        if let Some(ref fee_tiers) = filters.fee_tiers {
            let fee_list = fee_tiers
                .iter()
                .map(|f| f.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            where_clauses.push(format!("feeTier_in: [{}]", fee_list));
        }

        let where_clause = if where_clauses.is_empty() {
            String::new()
        } else {
            format!("where: {{ {} }}", where_clauses.join(", "))
        };

        let query = format!(
            r#"
            query FilteredPools($first: Int!) {{
                pools(
                    first: $first
                    orderBy: totalValueLockedUSD
                    orderDirection: desc
                    {}
                ) {{
                    id
                    token0 {{
                        id
                        symbol
                        name
                        decimals
                    }}
                    token1 {{
                        id
                        symbol
                        name
                        decimals
                    }}
                    feeTier
                    liquidity
                    sqrtPrice
                    token0Price
                    token1Price
                    volumeUSD
                    totalValueLockedUSD
                    txCount
                }}
            }}
            "#,
            where_clause
        );

        let variables = json!({ "first": limit });
        let data = self.query_subgraph(endpoint, &query, variables).await?;

        // Get pools array for post-query filtering
        let mut pools: Vec<Value> = data
            .get("pools")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        // Apply post-query filters (volume/TVL ratio, token pairs, exclude tokens)

        // Filter by volume/TVL ratio
        if let Some(min_ratio) = filters.min_volume_tvl_ratio {
            pools.retain(|pool| {
                let tvl = pool
                    .get("totalValueLockedUSD")
                    .and_then(|v| v.as_str())
                    .and_then(|s| s.parse::<f64>().ok())
                    .unwrap_or(0.0);
                let volume = pool
                    .get("volumeUSD")
                    .and_then(|v| v.as_str())
                    .and_then(|s| s.parse::<f64>().ok())
                    .unwrap_or(0.0);

                if tvl > 0.0 {
                    (volume / tvl) >= min_ratio
                } else {
                    false
                }
            });
        }

        // Filter by token pairs (if specified)
        if let Some(ref pairs) = filters.token_pairs {
            let pair_set: HashSet<String> = pairs
                .iter()
                .map(|p| p.to_lowercase().replace('/', "-"))
                .collect();

            pools.retain(|pool| {
                let token0 = pool
                    .get("token0")
                    .and_then(|t| t.get("symbol"))
                    .and_then(|s| s.as_str())
                    .unwrap_or("");
                let token1 = pool
                    .get("token1")
                    .and_then(|t| t.get("symbol"))
                    .and_then(|s| s.as_str())
                    .unwrap_or("");

                let pair1 = format!("{}-{}", token0, token1).to_lowercase();
                let pair2 = format!("{}-{}", token1, token0).to_lowercase();

                pair_set.contains(&pair1) || pair_set.contains(&pair2)
            });
        }

        // Exclude tokens
        if let Some(ref exclude) = filters.exclude_tokens {
            let exclude_set: HashSet<String> = exclude.iter().map(|t| t.to_lowercase()).collect();

            pools.retain(|pool| {
                let token0_id = pool
                    .get("token0")
                    .and_then(|t| t.get("id"))
                    .and_then(|s| s.as_str())
                    .unwrap_or("")
                    .to_lowercase();
                let token1_id = pool
                    .get("token1")
                    .and_then(|t| t.get("id"))
                    .and_then(|s| s.as_str())
                    .unwrap_or("")
                    .to_lowercase();

                !exclude_set.contains(&token0_id) && !exclude_set.contains(&token1_id)
            });
        }

        let count = pools.len();
        Ok(json!({
            "protocol": "uniswap_v3",
            "network": network.name(),
            "pools": pools,
            "filters_applied": true,
            "count": count
        }))
    }

    /// Execute a full query plan across multiple networks/protocols
    async fn execute_query_plan(&self, plan: &QueryPlan) -> Result<Value> {
        let mut results: Vec<Value> = Vec::new();

        // Execute queries for each network/protocol combination
        for network_str in &plan.target_networks {
            let network = match Self::parse_network(network_str) {
                Ok(n) => n,
                Err(e) => {
                    tracing::warn!(
                        network = network_str,
                        error = %e,
                        "Skipping unknown network in query plan"
                    );
                    continue;
                }
            };

            for protocol_str in &plan.target_protocols {
                if protocol_str == "uniswap_v3" {
                    // Use filtered_pools with plan's filters
                    let limit = plan.expected_data_points.clamp(10, 100);
                    match self
                        .query_filtered_pools(network, &plan.data_filters, limit)
                        .await
                    {
                        Ok(result) => {
                            results.push(json!({
                                "network": network_str,
                                "protocol": protocol_str,
                                "data": result
                            }));
                        }
                        Err(e) => {
                            // Log error but continue with other queries
                            tracing::warn!(
                                network = network_str,
                                protocol = protocol_str,
                                error = %e,
                                "Query failed in query plan execution"
                            );
                        }
                    }
                }
            }
        }

        Ok(json!({
            "query_plan": {
                "target_networks": plan.target_networks,
                "target_protocols": plan.target_protocols,
                "priority": plan.query_priority,
                "expected_data_points": plan.expected_data_points
            },
            "results": results
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
                    "enum": ["top_pools", "pool_info", "token_price", "filtered_pools", "query_plan"],
                    "description": "Type of data to retrieve. 'filtered_pools' applies filters to pool queries. 'query_plan' executes a full QueryPlan from InferQueryPlan."
                },
                "params": {
                    "type": "object",
                    "description": "Query-specific parameters",
                    "properties": {
                        "limit": {
                            "type": "integer",
                            "description": "Number of results for top_pools/filtered_pools (default: 10)"
                        },
                        "pool_id": {
                            "type": "string",
                            "description": "Pool address for pool_info query"
                        },
                        "token_address": {
                            "type": "string",
                            "description": "Token address for token_price query"
                        },
                        "filters": {
                            "type": "object",
                            "description": "Filters for filtered_pools query",
                            "properties": {
                                "min_tvl_usd": {"type": "number", "description": "Minimum TVL in USD"},
                                "min_volume_tvl_ratio": {"type": "number", "description": "Minimum volume/TVL ratio"},
                                "token_pairs": {"type": "array", "items": {"type": "string"}, "description": "Token pairs to include (e.g., ['WETH/USDC'])"},
                                "exclude_tokens": {"type": "array", "items": {"type": "string"}, "description": "Token addresses to exclude"},
                                "min_volume_24h_usd": {"type": "number", "description": "Minimum 24h volume in USD"},
                                "fee_tiers": {"type": "array", "items": {"type": "integer"}, "description": "Fee tiers to include (e.g., [3000, 5000])"}
                            }
                        },
                        "query_plan": {
                            "type": "object",
                            "description": "Full QueryPlan from InferQueryPlan (for query_plan query_type)",
                            "properties": {
                                "target_networks": {"type": "array", "items": {"type": "string"}},
                                "target_protocols": {"type": "array", "items": {"type": "string"}},
                                "data_filters": {"type": "object"},
                                "query_priority": {"type": "integer"},
                                "expected_data_points": {"type": "integer"}
                            }
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
            ("uniswap_v3", "filtered_pools") => {
                let filters_json = params.get("filters").cloned().unwrap_or(json!({}));
                let filters: QueryFilters = serde_json::from_value(filters_json)
                    .map_err(|e| BamlRtError::InvalidArgument(format!("Invalid filters: {}", e)))?;
                let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as u32;
                self.query_filtered_pools(network, &filters, limit).await
            }
            ("uniswap_v3", "query_plan") => {
                let plan_json = params.get("query_plan").ok_or_else(|| {
                    BamlRtError::InvalidArgument("Missing 'query_plan' in params".to_string())
                })?;
                let plan: QueryPlan = serde_json::from_value(plan_json.clone()).map_err(|e| {
                    BamlRtError::InvalidArgument(format!("Invalid query plan: {}", e))
                })?;
                self.execute_query_plan(&plan).await
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

    #[test]
    fn test_gateway_disabled_by_default() {
        let tool = TheGraphTool::new();
        assert!(!tool.has_gateway());
        assert!(tool.gateway_name().is_none());
    }

    #[test]
    fn test_gateway_enabled_with_api_key() {
        let tool = TheGraphTool::with_gateway("test-api-key".to_string());
        assert!(tool.has_gateway());
        assert_eq!(tool.gateway_name(), Some("BasicGraphGateway"));
    }

    #[test]
    fn test_extract_subgraph_id() {
        // Valid endpoint
        let endpoint =
            "https://gateway.thegraph.com/api/abc123/subgraphs/id/5zvR82QoaXYFyDEKLZ9t6v9adgnptxYpKpSbxtgVENFV";
        assert_eq!(
            TheGraphTool::extract_subgraph_id(endpoint),
            Some("5zvR82QoaXYFyDEKLZ9t6v9adgnptxYpKpSbxtgVENFV")
        );

        // Invalid - no subgraph ID
        let invalid = "https://api.example.com/graphql";
        assert!(TheGraphTool::extract_subgraph_id(invalid).is_none());

        // Invalid - too short
        let short = "https://gateway.thegraph.com/api/key/subgraphs/id/abc";
        assert!(TheGraphTool::extract_subgraph_id(short).is_none());
    }

    #[test]
    fn test_get_subgraph_id() {
        assert_eq!(
            TheGraphTool::get_subgraph_id(Network::Ethereum, Protocol::UniswapV3),
            Some(SubgraphIds::UNISWAP_V3_ETHEREUM)
        );
        assert_eq!(
            TheGraphTool::get_subgraph_id(Network::Arbitrum, Protocol::UniswapV3),
            Some(SubgraphIds::UNISWAP_V3_ARBITRUM)
        );
        // AaveV3 not configured for Uniswap V3
        assert!(TheGraphTool::get_subgraph_id(Network::Ethereum, Protocol::AaveV3).is_none());
    }
}
