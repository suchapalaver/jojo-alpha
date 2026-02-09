//! Wallet signing tools (passkey-like signing ladder).
//!
//! SECURITY NOTE:
//! - Uses SecureWallet signing only; no private key exposure.
//! - Returns signatures and hashes, never raw key material.

use crate::wallet::SecureWallet;
use alloy::primitives::{eip191_hash_message, hex, keccak256, B256};
use async_trait::async_trait;
use baml_rt::error::{BamlRtError, Result};
use baml_rt::tools::BamlTool;
use serde_json::{json, Value};
use std::sync::Arc;

const WALLET_DERIVE_ADDRESS: &str = "wallet_derive_address";
const WALLET_SIGN_MESSAGE: &str = "wallet_sign_message";
const WALLET_SIGN_TX: &str = "wallet_sign_tx";

fn b256_to_array(hash: B256) -> [u8; 32] {
    hash.0
}

fn decode_hex(input: &str) -> Result<Vec<u8>> {
    let trimmed = input.strip_prefix("0x").unwrap_or(input);
    hex::decode(trimmed)
        .map_err(|e| BamlRtError::InvalidArgument(format!("Invalid hex string: {}", e)))
}

fn encode_hex_prefixed(bytes: &[u8]) -> String {
    format!("0x{}", hex::encode(bytes))
}

pub struct WalletDeriveAddressTool {
    wallet: Arc<SecureWallet>,
}

impl WalletDeriveAddressTool {
    pub fn new(wallet: Arc<SecureWallet>) -> Self {
        Self { wallet }
    }
}

#[async_trait]
impl BamlTool for WalletDeriveAddressTool {
    const NAME: &'static str = WALLET_DERIVE_ADDRESS;

    fn description(&self) -> &'static str {
        "Derive the public wallet address (read-only)."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    }

    async fn execute(&self, _args: Value) -> Result<Value> {
        Ok(json!({
            "address": self.wallet.address_string()
        }))
    }
}

pub struct WalletSignMessageTool {
    wallet: Arc<SecureWallet>,
}

impl WalletSignMessageTool {
    pub fn new(wallet: Arc<SecureWallet>) -> Self {
        Self { wallet }
    }
}

#[async_trait]
impl BamlTool for WalletSignMessageTool {
    const NAME: &'static str = WALLET_SIGN_MESSAGE;

    fn description(&self) -> &'static str {
        "Sign an EIP-191 message (policy-gated). Returns signature and message hash."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "message": {
                    "type": "string",
                    "description": "Message to sign (UTF-8)."
                }
            },
            "required": ["message"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let message = args
            .get("message")
            .and_then(|v| v.as_str())
            .ok_or_else(|| BamlRtError::InvalidArgument("Missing message".to_string()))?;

        let hash = eip191_hash_message(message.as_bytes());
        let signature = self
            .wallet
            .sign_hash(&b256_to_array(hash))
            .await
            .map_err(|e| BamlRtError::ToolExecution(e.to_string()))?;

        Ok(json!({
            "address": self.wallet.address_string(),
            "message_hash": signature_message_hash(hash),
            "signature": signature.to_string()
        }))
    }
}

pub struct WalletSignTxTool {
    wallet: Arc<SecureWallet>,
}

impl WalletSignTxTool {
    pub fn new(wallet: Arc<SecureWallet>) -> Self {
        Self { wallet }
    }
}

#[async_trait]
impl BamlTool for WalletSignTxTool {
    const NAME: &'static str = WALLET_SIGN_TX;

    fn description(&self) -> &'static str {
        "Sign a transaction hash or raw bytes (policy-gated). Returns signature and hash."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "tx_hash": {
                    "type": "string",
                    "description": "32-byte tx hash to sign (0x...)."
                },
                "tx_bytes": {
                    "type": "string",
                    "description": "Raw transaction bytes to hash and sign (0x...)."
                }
            },
            "oneOf": [
                {"required": ["tx_hash"]},
                {"required": ["tx_bytes"]}
            ],
            "additionalProperties": false
        })
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let (hash, source) = if let Some(tx_hash) = args.get("tx_hash").and_then(|v| v.as_str()) {
            let bytes = decode_hex(tx_hash)?;
            if bytes.len() != 32 {
                return Err(BamlRtError::InvalidArgument(
                    "tx_hash must be 32 bytes".to_string(),
                ));
            }
            let mut array = [0u8; 32];
            array.copy_from_slice(&bytes);
            (B256::from(array), "tx_hash")
        } else if let Some(tx_bytes) = args.get("tx_bytes").and_then(|v| v.as_str()) {
            let bytes = decode_hex(tx_bytes)?;
            (keccak256(bytes), "tx_bytes")
        } else {
            return Err(BamlRtError::InvalidArgument(
                "Missing tx_hash or tx_bytes".to_string(),
            ));
        };

        let signature = self
            .wallet
            .sign_hash(&b256_to_array(hash))
            .await
            .map_err(|e| BamlRtError::ToolExecution(e.to_string()))?;

        Ok(json!({
            "address": self.wallet.address_string(),
            "hash_source": source,
            "tx_hash": signature_message_hash(hash),
            "signature": signature.to_string()
        }))
    }
}

fn signature_message_hash(hash: B256) -> String {
    encode_hex_prefixed(&b256_to_array(hash))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_hex_rejects_invalid() {
        let err = decode_hex("0xzz").unwrap_err();
        assert!(format!("{err}").contains("Invalid hex"));
    }
}
