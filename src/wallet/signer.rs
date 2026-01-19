//! Secure wallet implementation
//!
//! SECURITY: This is the ONLY place where private keys exist.
//! - Keys are held in alloy's PrivateKeySigner which handles crypto securely
//! - Keys are never serialized to JSON
//! - Keys are never passed to QuickJS/JavaScript
//! - Keys are never logged

use crate::{Error, Result};
use alloy::network::EthereumWallet;
use alloy::primitives::{Address, Bytes, U256};
use alloy::signers::local::PrivateKeySigner;
use serde::Serialize;

/// A prepared transaction ready for signing
///
/// Used when executing swaps through the interceptor pipeline.
#[derive(Debug, Clone, Serialize)]
#[allow(dead_code)] // Will be used when transaction execution is implemented
pub struct PreparedTransaction {
    pub to: Address,
    pub data: Bytes,
    pub value: U256,
    pub gas_limit: u64,
    pub chain_id: u64,
}

/// Secure wallet that protects private keys
///
/// The private key is:
/// - Stored in alloy's PrivateKeySigner (handles crypto securely)
/// - Never serialized (no Serialize impl)
/// - Only accessible via signing operations
pub struct SecureWallet {
    /// The signer
    signer: PrivateKeySigner,
    /// Public address (safe to expose)
    address: Address,
    /// Ethereum wallet for alloy integration
    wallet: EthereumWallet,
}

impl SecureWallet {
    /// Create a wallet from an environment variable
    ///
    /// # Arguments
    /// * `var_name` - Name of the environment variable containing the private key
    ///
    /// # Security
    /// The environment variable should contain a hex-encoded private key.
    /// Consider using a secrets manager in production.
    pub fn from_env(var_name: &str) -> Result<Self> {
        let key_hex = std::env::var(var_name).map_err(|_| {
            Error::Wallet(format!(
                "Environment variable {} not set. Required for wallet initialization.",
                var_name
            ))
        })?;

        Self::from_hex(&key_hex)
    }

    /// Create a wallet from a hex-encoded private key
    ///
    /// # Security
    /// After calling this, the original string should be zeroized if possible.
    pub fn from_hex(key_hex: &str) -> Result<Self> {
        // Remove 0x prefix if present
        let key_hex = key_hex.strip_prefix("0x").unwrap_or(key_hex);

        let signer: PrivateKeySigner = key_hex
            .parse()
            .map_err(|e| Error::Wallet(format!("Invalid private key: {}", e)))?;

        let address = signer.address();
        let wallet = EthereumWallet::from(signer.clone());

        Ok(Self {
            signer,
            address,
            wallet,
        })
    }

    /// Get the public address (safe to share)
    pub fn address(&self) -> Address {
        self.address
    }

    /// Get the address as a checksummed string
    pub fn address_string(&self) -> String {
        format!("{:?}", self.address)
    }

    /// Get a reference to the EthereumWallet for use with alloy providers
    ///
    /// This is safe because EthereumWallet only exposes signing operations,
    /// not the raw private key.
    pub fn wallet(&self) -> &EthereumWallet {
        &self.wallet
    }

    /// Sign a message hash
    ///
    /// This is the ONLY way to use the private key.
    pub async fn sign_hash(&self, hash: &[u8; 32]) -> Result<alloy::signers::Signature> {
        use alloy::signers::SignerSync;

        self.signer
            .sign_hash_sync(&alloy::primitives::B256::from(*hash))
            .map_err(|e| Error::Wallet(format!("Signing failed: {}", e)))
    }
}

// Implement Debug manually to avoid exposing the signer
impl std::fmt::Debug for SecureWallet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecureWallet")
            .field("address", &self.address)
            .field("signer", &"[REDACTED]")
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wallet_from_hex() {
        // Test private key (DO NOT use in production!)
        let test_key = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

        let wallet = SecureWallet::from_hex(test_key).unwrap();

        // Should derive the correct address
        assert_eq!(
            wallet.address_string().to_lowercase(),
            "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266"
        );
    }

    #[test]
    fn test_debug_redacts_key() {
        let test_key = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
        let wallet = SecureWallet::from_hex(test_key).unwrap();

        let debug_str = format!("{:?}", wallet);

        // Should not contain the private key
        assert!(!debug_str.contains("ac0974bec"));
        assert!(debug_str.contains("[REDACTED]"));
    }
}
