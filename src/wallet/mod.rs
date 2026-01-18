//! Secure wallet management
//!
//! This module handles private key storage and transaction signing.
//! The private key NEVER leaves this module and is NEVER exposed to JavaScript.

mod signer;

pub use signer::SecureWallet;
