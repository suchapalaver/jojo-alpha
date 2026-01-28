//! Tool implementations for the trading agent
//!
//! Tools implement the `BamlTool` trait from baml-rt and are exposed
//! to the TypeScript agent via the QuickJS bridge.

mod odos;
mod paper_trading;
mod the_graph;
mod wallet;

pub use odos::OdosTool;
pub use paper_trading::PaperTradingTool;
pub use the_graph::TheGraphTool;
pub use wallet::WalletTool;
