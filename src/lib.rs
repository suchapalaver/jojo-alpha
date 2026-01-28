//! DeFi Trading Agent
//!
//! An AI-powered trading agent that uses BAML runtime to:
//! - Query DeFi protocol data from The Graph
//! - Infer trading strategies using LLMs
//! - Execute swaps via Odos DEX aggregator
//!
//! # Security Model
//!
//! - TypeScript agent runs in QuickJS sandbox (no filesystem/network access)
//! - All tool calls pass through interceptor pipeline
//! - Private keys never leave the Rust wallet module
//! - Full audit trail of all operations

pub mod config;
pub mod graphql;
pub mod interceptors;
pub mod paper_trading;
pub mod runner;
pub mod tokens;
pub mod tools;
pub mod wallet;

mod error;

// Re-export commonly used types
pub use config::{Config, RpcConfig, SpendLimitMode, GRAPH_API_KEY_ENV};
pub use error::{Error, Result};
pub use paper_trading::{PaperModeConfig, PaperTradingState};
pub use runner::AgentRunner;
