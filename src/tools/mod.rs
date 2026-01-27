//! Tool implementations for the trading agent
//!
//! Tools implement the `BamlTool` trait from baml-rt and are exposed
//! to the TypeScript agent via the QuickJS bridge.

mod odos;
mod the_graph;
mod wallet;

pub use odos::OdosTool;
pub use the_graph::TheGraphTool;
pub use wallet::WalletTool;
