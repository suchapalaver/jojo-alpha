//! Transaction simulation before signing
//!
//! Uses `eth_call` to simulate transactions before signing to:
//! - Catch reverts early with detailed error messages
//! - Estimate accurate gas usage
//! - Validate transaction will succeed on-chain
//!
//! SECURITY NOTE:
//! - This module is read-only - it never signs or submits transactions
//! - Simulation uses the wallet's public address only

use alloy::hex;
use alloy::primitives::{Address, Bytes, U256};
use alloy::providers::{Provider, ProviderBuilder};
use alloy::rpc::types::TransactionRequest;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

/// Result of simulating a transaction
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulationResult {
    /// Whether the simulation succeeded
    pub success: bool,
    /// Estimated gas used (if successful)
    pub gas_used: Option<u64>,
    /// Revert reason (if failed)
    pub revert_reason: Option<String>,
    /// Raw return data from eth_call
    pub return_data: Option<String>,
}

impl SimulationResult {
    /// Create a successful simulation result
    pub fn success(gas_used: u64, return_data: Option<String>) -> Self {
        Self {
            success: true,
            gas_used: Some(gas_used),
            revert_reason: None,
            return_data,
        }
    }

    /// Create a failed simulation result
    pub fn failed(reason: String) -> Self {
        Self {
            success: false,
            gas_used: None,
            revert_reason: Some(reason),
            return_data: None,
        }
    }
}

/// Error type for simulation failures
#[derive(Debug, thiserror::Error)]
pub enum SimulationError {
    #[error("RPC URL not configured for chain {0}")]
    NoRpcUrl(u64),

    #[error("Invalid RPC URL: {0}")]
    InvalidUrl(String),

    #[error("Simulation failed: {0}")]
    SimulationFailed(String),

    #[error("Invalid address: {0}")]
    InvalidAddress(String),

    #[error("Network error: {0}")]
    Network(String),
}

/// Transaction simulator using eth_call
pub struct TransactionSimulator {
    /// RPC URL for the chain
    rpc_url: String,
    /// Chain ID
    chain_id: u64,
}

impl TransactionSimulator {
    /// Create a new simulator for a specific chain
    pub fn new(rpc_url: String, chain_id: u64) -> Self {
        Self { rpc_url, chain_id }
    }

    /// Create a simulator from RPC config
    pub fn from_rpc_config(
        rpc_config: &crate::config::rpc::RpcConfig,
        chain_id: u64,
    ) -> Result<Self, SimulationError> {
        let rpc_url = rpc_config
            .get(chain_id)
            .ok_or(SimulationError::NoRpcUrl(chain_id))?
            .to_string();
        Ok(Self::new(rpc_url, chain_id))
    }

    /// Simulate a transaction using eth_call
    ///
    /// # Arguments
    /// * `from` - Sender address
    /// * `to` - Contract address
    /// * `data` - Calldata
    /// * `value` - ETH value to send
    pub async fn simulate(
        &self,
        from: &str,
        to: &str,
        data: &str,
        value: Option<&str>,
    ) -> Result<SimulationResult, SimulationError> {
        let from_addr = Address::from_str(from)
            .map_err(|e| SimulationError::InvalidAddress(format!("from: {}", e)))?;
        let to_addr = Address::from_str(to)
            .map_err(|e| SimulationError::InvalidAddress(format!("to: {}", e)))?;

        // Parse data - handle both with and without 0x prefix
        let data_hex = data.strip_prefix("0x").unwrap_or(data);
        let data_bytes = hex::decode(data_hex)
            .map_err(|e| SimulationError::InvalidAddress(format!("data: {}", e)))?;

        // Parse value if provided
        let value_u256 = if let Some(v) = value {
            U256::from_str(v)
                .map_err(|e| SimulationError::InvalidAddress(format!("value: {}", e)))?
        } else {
            U256::ZERO
        };

        self.simulate_request(from_addr, to_addr, Bytes::from(data_bytes), value_u256)
            .await
    }

    /// Simulate a transaction request
    pub async fn simulate_request(
        &self,
        from: Address,
        to: Address,
        data: Bytes,
        value: U256,
    ) -> Result<SimulationResult, SimulationError> {
        let url: url::Url = self
            .rpc_url
            .parse()
            .map_err(|e| SimulationError::InvalidUrl(format!("{}", e)))?;

        let provider = ProviderBuilder::new().connect_http(url);

        // Build the transaction request
        let tx = TransactionRequest::default()
            .from(from)
            .to(to)
            .input(data.into())
            .value(value);

        // First, try eth_call to check if it reverts
        match provider.call(tx.clone()).await {
            Ok(result) => {
                // Call succeeded, now estimate gas
                let gas_estimate = provider.estimate_gas(tx).await.unwrap_or(0);

                Ok(SimulationResult::success(
                    gas_estimate,
                    Some(format!("{}", result)),
                ))
            }
            Err(e) => {
                // Parse revert reason from error
                let reason = Self::parse_revert_reason(&e.to_string());
                Ok(SimulationResult::failed(reason))
            }
        }
    }

    /// Parse revert reason from RPC error message
    fn parse_revert_reason(error: &str) -> String {
        // Common patterns for revert reasons in RPC errors
        if error.contains("execution reverted") {
            // Try to extract the reason string
            if let Some(start) = error.find("revert: ") {
                let reason = &error[start + 8..];
                if let Some(end) = reason.find('"') {
                    return reason[..end].to_string();
                }
                return reason.to_string();
            }
            // Try to extract hex data
            if let Some(start) = error.find("0x") {
                let hex_data = &error[start..];
                if let Some(end) = hex_data.find(|c: char| !c.is_ascii_hexdigit() && c != 'x') {
                    let hex = &hex_data[..end];
                    // Check if it's an Error(string) selector (0x08c379a0)
                    if hex.starts_with("0x08c379a0") && hex.len() > 138 {
                        // Decode the string from ABI encoding
                        if let Ok(decoded) = hex::decode(&hex[138..]) {
                            let filtered: Vec<u8> =
                                decoded.into_iter().filter(|&b| b != 0).collect();
                            if let Ok(s) = String::from_utf8(filtered) {
                                return s;
                            }
                        }
                    }
                    return format!("Reverted with data: {}", hex);
                }
            }
            return "execution reverted".to_string();
        }

        // Return the full error if we can't parse it
        error.to_string()
    }

    /// Get chain ID
    pub fn chain_id(&self) -> u64 {
        self.chain_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simulation_result_success() {
        let result = SimulationResult::success(21000, Some("0x".to_string()));
        assert!(result.success);
        assert_eq!(result.gas_used, Some(21000));
        assert!(result.revert_reason.is_none());
    }

    #[test]
    fn test_simulation_result_failed() {
        let result = SimulationResult::failed("insufficient balance".to_string());
        assert!(!result.success);
        assert!(result.gas_used.is_none());
        assert_eq!(
            result.revert_reason,
            Some("insufficient balance".to_string())
        );
    }

    #[test]
    fn test_parse_revert_reason() {
        // Test simple revert message
        let error = "execution reverted: revert: Insufficient balance\"";
        let reason = TransactionSimulator::parse_revert_reason(error);
        assert_eq!(reason, "Insufficient balance");

        // Test execution reverted without message
        let error = "execution reverted";
        let reason = TransactionSimulator::parse_revert_reason(error);
        assert_eq!(reason, "execution reverted");

        // Test unknown error
        let error = "some other error";
        let reason = TransactionSimulator::parse_revert_reason(error);
        assert_eq!(reason, "some other error");
    }
}
