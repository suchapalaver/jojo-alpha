//! GraphQL schemas and generated types for The Graph subgraphs
//!
//! This module contains:
//! - GraphQL schema files for each protocol (uniswap_v3/, aave_v3/, etc.)
//! - Generated Rust types from graphql-client
//!
//! To regenerate types after schema changes:
//! ```bash
//! cargo build  # graphql-client generates types at compile time
//! ```

// Note: The actual GraphQL schemas are stored as .graphql files in the
// subdirectories. The graphql_client derive macro reads these at compile time.
//
// For now, TheGraphTool uses raw GraphQL strings for flexibility.
// A future enhancement could use graphql_client for compile-time type safety.
