//! x402 Gateway Protocol Abstraction
//!
//! Provides intelligent subgraph routing, load balancing, and caching
//! for The Graph decentralized network queries.
//!
//! This module defines a trait-based abstraction that allows:
//! - Query routing with latency and indexer preferences
//! - Result caching with configurable TTL
//! - Indexer selection based on performance and stake
//!
//! The `BasicGraphGateway` provides a simple implementation using
//! the standard Graph API. When the x402 protocol is finalized,
//! an `X402GraphGateway` can be added that implements advanced
//! routing features.

use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

/// Query routing hints for x402 gateway
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct QueryRoutingHints {
    /// Preferred indexers (by address)
    #[serde(default)]
    pub preferred_indexers: Option<Vec<String>>,
    /// Maximum latency tolerance (ms)
    #[serde(default)]
    pub max_latency_ms: Option<u64>,
    /// Cache TTL (seconds)
    #[serde(default)]
    pub cache_ttl_secs: Option<u64>,
    /// Require fresh data (bypass cache)
    #[serde(default)]
    pub force_fresh: bool,
}

/// Gateway query result with metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayQueryResult {
    /// The query result data
    pub data: Value,
    /// The indexer that served this query (if known)
    pub indexer: Option<String>,
    /// Query latency in milliseconds
    pub latency_ms: u64,
    /// Whether this result was served from cache
    pub cached: bool,
    /// The subgraph ID that was queried
    pub subgraph_id: String,
}

/// Information about an indexer
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexerInfo {
    /// Indexer's Ethereum address
    pub address: String,
    /// Amount of GRT staked
    pub staked_tokens: String,
    /// Query fees charged
    pub query_fees: String,
    /// Average query latency (if measured)
    pub avg_latency_ms: Option<u64>,
}

/// Error type for gateway operations
#[derive(Debug)]
pub enum GatewayError {
    /// HTTP request failed
    HttpError(String),
    /// GraphQL query returned errors
    GraphQLError(Vec<String>),
    /// No data in response
    NoData,
    /// Subgraph not found
    SubgraphNotFound(String),
    /// All indexers failed
    AllIndexersFailed,
}

impl std::fmt::Display for GatewayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GatewayError::HttpError(msg) => write!(f, "HTTP error: {}", msg),
            GatewayError::GraphQLError(errors) => {
                write!(f, "GraphQL errors: {}", errors.join(", "))
            }
            GatewayError::NoData => write!(f, "No data in response"),
            GatewayError::SubgraphNotFound(id) => write!(f, "Subgraph not found: {}", id),
            GatewayError::AllIndexersFailed => write!(f, "All indexers failed to respond"),
        }
    }
}

impl std::error::Error for GatewayError {}

/// Trait for Graph gateway implementations
///
/// This trait abstracts over different ways to query The Graph:
/// - Direct API calls (BasicGraphGateway)
/// - x402 protocol routing (future X402GraphGateway)
/// - Custom routing logic
#[async_trait]
pub trait GraphGateway: Send + Sync {
    /// Execute a query with x402 routing hints
    ///
    /// # Arguments
    /// * `subgraph_id` - The subgraph deployment ID to query
    /// * `query` - The GraphQL query string
    /// * `variables` - Query variables as JSON
    /// * `routing_hints` - Optional routing preferences
    ///
    /// # Returns
    /// Query result with metadata about execution
    async fn query_with_routing(
        &self,
        subgraph_id: &str,
        query: &str,
        variables: Value,
        routing_hints: QueryRoutingHints,
    ) -> Result<GatewayQueryResult, GatewayError>;

    /// Get available indexers for a subgraph
    ///
    /// # Arguments
    /// * `subgraph_id` - The subgraph deployment ID
    ///
    /// # Returns
    /// List of indexers serving this subgraph
    async fn get_indexers(&self, subgraph_id: &str) -> Result<Vec<IndexerInfo>, GatewayError>;

    /// Get the gateway name for logging/metrics
    fn name(&self) -> &'static str;
}

/// Cache entry for query results
struct CacheEntry {
    result: GatewayQueryResult,
    expires_at: Instant,
}

/// Basic gateway implementation using current The Graph API
///
/// This implementation provides:
/// - Direct queries to The Graph's gateway API
/// - Simple in-memory caching with TTL
/// - No advanced routing (routing hints are recorded but not acted upon)
pub struct BasicGraphGateway {
    client: Client,
    api_key: String,
    cache: Arc<RwLock<HashMap<String, CacheEntry>>>,
    default_cache_ttl: Duration,
}

impl BasicGraphGateway {
    /// Create a new BasicGraphGateway
    ///
    /// # Arguments
    /// * `api_key` - The Graph API key for authentication
    pub fn new(api_key: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
            cache: Arc::new(RwLock::new(HashMap::new())),
            default_cache_ttl: Duration::from_secs(60), // 1 minute default
        }
    }

    /// Create with custom cache TTL
    pub fn with_cache_ttl(api_key: String, cache_ttl: Duration) -> Self {
        Self {
            client: Client::new(),
            api_key,
            cache: Arc::new(RwLock::new(HashMap::new())),
            default_cache_ttl: cache_ttl,
        }
    }

    /// Build the API endpoint URL for a subgraph
    fn build_endpoint(&self, subgraph_id: &str) -> String {
        format!(
            "https://gateway.thegraph.com/api/{}/subgraphs/id/{}",
            self.api_key, subgraph_id
        )
    }

    /// Generate a cache key for a query
    fn cache_key(subgraph_id: &str, query: &str, variables: &Value) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        subgraph_id.hash(&mut hasher);
        query.hash(&mut hasher);
        variables.to_string().hash(&mut hasher);
        format!("{:x}", hasher.finish())
    }

    /// Clean expired cache entries
    async fn clean_expired_cache(&self) {
        let now = Instant::now();
        let mut cache = self.cache.write().await;
        cache.retain(|_, entry| entry.expires_at > now);
    }
}

#[async_trait]
impl GraphGateway for BasicGraphGateway {
    async fn query_with_routing(
        &self,
        subgraph_id: &str,
        query: &str,
        variables: Value,
        routing_hints: QueryRoutingHints,
    ) -> Result<GatewayQueryResult, GatewayError> {
        // Check cache first (unless force_fresh is set)
        if !routing_hints.force_fresh {
            let cache_key = Self::cache_key(subgraph_id, query, &variables);
            let cache = self.cache.read().await;
            if let Some(entry) = cache.get(&cache_key) {
                if entry.expires_at > Instant::now() {
                    let mut cached_result = entry.result.clone();
                    cached_result.cached = true;
                    return Ok(cached_result);
                }
            }
        }

        // Execute the query
        let endpoint = self.build_endpoint(subgraph_id);
        let start = Instant::now();

        let response = self
            .client
            .post(&endpoint)
            .json(&json!({
                "query": query,
                "variables": variables
            }))
            .send()
            .await
            .map_err(|e| GatewayError::HttpError(e.to_string()))?;

        let latency_ms = start.elapsed().as_millis() as u64;

        // Check if we exceeded max latency (for metrics/logging, not failure)
        if let Some(max_latency) = routing_hints.max_latency_ms {
            if latency_ms > max_latency {
                tracing::warn!(
                    subgraph_id = subgraph_id,
                    latency_ms = latency_ms,
                    max_latency_ms = max_latency,
                    "Query exceeded maximum latency threshold"
                );
            }
        }

        let response_data: Value = response
            .json()
            .await
            .map_err(|e| GatewayError::HttpError(format!("Failed to parse response: {}", e)))?;

        // Check for GraphQL errors
        if let Some(errors) = response_data.get("errors") {
            if let Some(errors_array) = errors.as_array() {
                let error_messages: Vec<String> = errors_array
                    .iter()
                    .filter_map(|e| e.get("message").and_then(|m| m.as_str()).map(String::from))
                    .collect();
                if !error_messages.is_empty() {
                    return Err(GatewayError::GraphQLError(error_messages));
                }
            }
        }

        let data = response_data
            .get("data")
            .cloned()
            .ok_or(GatewayError::NoData)?;

        let result = GatewayQueryResult {
            data,
            indexer: None, // Basic gateway doesn't track indexers
            latency_ms,
            cached: false,
            subgraph_id: subgraph_id.to_string(),
        };

        // Cache the result
        let cache_ttl = routing_hints
            .cache_ttl_secs
            .map(Duration::from_secs)
            .unwrap_or(self.default_cache_ttl);

        let cache_key = Self::cache_key(subgraph_id, query, &variables);
        let entry = CacheEntry {
            result: result.clone(),
            expires_at: Instant::now() + cache_ttl,
        };

        {
            let mut cache = self.cache.write().await;
            cache.insert(cache_key, entry);
        }

        // Periodically clean expired entries (roughly every 10 queries based on latency)
        // Use latency as a simple pseudo-random source
        if latency_ms.is_multiple_of(10) {
            let gateway = self.clone();
            tokio::spawn(async move {
                gateway.clean_expired_cache().await;
            });
        }

        Ok(result)
    }

    async fn get_indexers(&self, _subgraph_id: &str) -> Result<Vec<IndexerInfo>, GatewayError> {
        // Basic gateway doesn't expose indexer information
        // This would require querying The Graph's network subgraph
        Ok(vec![])
    }

    fn name(&self) -> &'static str {
        "BasicGraphGateway"
    }
}

impl Clone for BasicGraphGateway {
    fn clone(&self) -> Self {
        Self {
            client: Client::new(),
            api_key: self.api_key.clone(),
            cache: Arc::clone(&self.cache),
            default_cache_ttl: self.default_cache_ttl,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_key_deterministic() {
        let key1 = BasicGraphGateway::cache_key("abc", "query { pools }", &json!({"first": 10}));
        let key2 = BasicGraphGateway::cache_key("abc", "query { pools }", &json!({"first": 10}));
        assert_eq!(key1, key2);
    }

    #[test]
    fn test_cache_key_different_for_different_inputs() {
        let key1 = BasicGraphGateway::cache_key("abc", "query { pools }", &json!({"first": 10}));
        let key2 = BasicGraphGateway::cache_key("abc", "query { pools }", &json!({"first": 20}));
        assert_ne!(key1, key2);
    }

    #[test]
    fn test_routing_hints_default() {
        let hints = QueryRoutingHints::default();
        assert!(!hints.force_fresh);
        assert!(hints.preferred_indexers.is_none());
        assert!(hints.max_latency_ms.is_none());
        assert!(hints.cache_ttl_secs.is_none());
    }
}
