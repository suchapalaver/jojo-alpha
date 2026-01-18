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

pub mod graphql;
pub mod interceptors;
pub mod tools;
pub mod wallet;

mod config;
mod error;

pub use config::Config;
pub use error::{Error, Result};
