//! Tool implementations for the trading agent
//!
//! Tools implement the `BamlTool` trait from baml-rt and are exposed
//! to the TypeScript agent via the QuickJS bridge.

pub mod graph_gateway;
mod odos;
mod paper_trading;
mod the_graph;
mod wallet;
mod wallet_signing;

pub use graph_gateway::{BasicGraphGateway, GatewayError, GraphGateway, QueryRoutingHints};
pub use odos::OdosTool;
pub use paper_trading::PaperTradingTool;
pub use the_graph::{QueryFilters, QueryPlan, TheGraphTool};
pub use wallet::WalletTool;
pub use wallet_signing::{WalletDeriveAddressTool, WalletSignMessageTool, WalletSignTxTool};
