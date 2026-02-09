//! Wallet signing tools (passkey-like signing ladder).
//!
//! SECURITY NOTE:
//! - Uses SecureWallet signing only; no private key exposure.
//! - Returns signatures and hashes, never raw key material.

use crate::tools::{AnyJson, DefiBundle};
use crate::wallet::SecureWallet;
use alloy::primitives::{eip191_hash_message, hex, keccak256, B256};
use async_trait::async_trait;
use baml_rt::error::{BamlRtError, Result};
use baml_rt::tools::BamlTool;
use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[ts(export)]
pub struct EmptyArgs {}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[ts(export)]
pub struct WalletSignMessageInput {
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[ts(export)]
#[schemars(schema_with = "wallet_sign_tx_schema")]
pub struct WalletSignTxInput {
    pub tx_hash: Option<String>,
    pub tx_bytes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct WalletSignTxInputSchema {
    pub tx_hash: Option<String>,
    pub tx_bytes: Option<String>,
}

fn wallet_sign_tx_schema(gen: &mut SchemaGenerator) -> Schema {
    let schema = WalletSignTxInputSchema::json_schema(gen);
    let mut value: serde_json::Value = schema.into();
    if let serde_json::Value::Object(ref mut map) = value {
        map.insert(
            "oneOf".to_string(),
            json!([{"required": ["tx_hash"]}, {"required": ["tx_bytes"]}]),
        );
        return Schema::from(std::mem::take(map));
    }
    Schema::default()
}

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
    type Bundle = DefiBundle;
    const LOCAL_NAME: &'static str = "wallet_derive_address";
    type OpenInput = ();
    type Input = EmptyArgs;
    type Output = AnyJson;

    fn description(&self) -> &'static str {
        "Derive the public wallet address (read-only)."
    }

    async fn execute(&self, _args: Self::Input) -> Result<Self::Output> {
        Ok(AnyJson::new(json!({
            "address": self.wallet.address_string()
        })))
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
    type Bundle = DefiBundle;
    const LOCAL_NAME: &'static str = "wallet_sign_message";
    type OpenInput = ();
    type Input = WalletSignMessageInput;
    type Output = AnyJson;

    fn description(&self) -> &'static str {
        "Sign an EIP-191 message (policy-gated). Returns signature and message hash."
    }

    async fn execute(&self, args: Self::Input) -> Result<Self::Output> {
        let message = args.message;

        let hash = eip191_hash_message(message.as_bytes());
        let signature = self
            .wallet
            .sign_hash(&b256_to_array(hash))
            .await
            .map_err(|e| BamlRtError::ToolExecution(e.to_string()))?;

        Ok(AnyJson::new(json!({
            "address": self.wallet.address_string(),
            "message_hash": signature_message_hash(hash),
            "signature": signature.to_string()
        })))
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
    type Bundle = DefiBundle;
    const LOCAL_NAME: &'static str = "wallet_sign_tx";
    type OpenInput = ();
    type Input = WalletSignTxInput;
    type Output = AnyJson;

    fn description(&self) -> &'static str {
        "Sign a transaction hash or raw bytes (policy-gated). Returns signature and hash."
    }

    async fn execute(&self, args: Self::Input) -> Result<Self::Output> {
        let (hash, source) = if let Some(tx_hash) = args.tx_hash {
            let bytes = decode_hex(&tx_hash)?;
            if bytes.len() != 32 {
                return Err(BamlRtError::InvalidArgument(
                    "tx_hash must be 32 bytes".to_string(),
                ));
            }
            let mut array = [0u8; 32];
            array.copy_from_slice(&bytes);
            (B256::from(array), "tx_hash")
        } else if let Some(tx_bytes) = args.tx_bytes {
            let bytes = decode_hex(&tx_bytes)?;
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

        Ok(AnyJson::new(json!({
            "address": self.wallet.address_string(),
            "hash_source": source,
            "tx_hash": signature_message_hash(hash),
            "signature": signature.to_string()
        })))
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
