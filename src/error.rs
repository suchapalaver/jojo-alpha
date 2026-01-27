//! Error types for the DeFi trading agent

use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("GraphQL query failed: {0}")]
    GraphQL(String),

    #[error("Odos API error: {0}")]
    Odos(String),

    #[error("Wallet error: {0}")]
    Wallet(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Interceptor blocked: {0}")]
    Blocked(String),

    #[error("Invalid argument: {0}")]
    InvalidArgument(String),

    #[error("Network error: {0}")]
    Network(#[from] reqwest::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("BAML runtime error: {0}")]
    BamlRuntime(String),

    #[error("Transaction simulation failed: {0}")]
    Simulation(String),
}

pub type Result<T> = std::result::Result<T, Error>;
