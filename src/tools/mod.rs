//! Tool implementations for the trading agent
//!
//! Tools implement the `BamlTool` trait from baml-rt and are exposed
//! to the TypeScript agent via the QuickJS bridge.

pub mod graph_gateway;
mod odos;
mod paper_trading;
mod the_graph;
mod types;
mod wallet;
mod wallet_signing;

use baml_rt_tools::BundleType;

pub use graph_gateway::{BasicGraphGateway, GatewayError, GraphGateway, QueryRoutingHints};
pub use odos::{OdosAction, OdosInput, OdosTool};
pub use paper_trading::PaperTradingTool;
pub use the_graph::{
    GraphQueryInput, GraphQueryParams, GraphQueryType, QueryFilters, QueryPlan, TheGraphTool,
};
pub use types::AnyJson;
pub use wallet::WalletTool;
pub use wallet_signing::{WalletDeriveAddressTool, WalletSignMessageTool, WalletSignTxTool};

/// Bundle for all agent tools in this repo.
pub struct DefiBundle;

impl BundleType for DefiBundle {
    const NAME: &'static str = "defi";

    fn description() -> &'static str {
        "DeFi trading agent tools (graph, swaps, wallet, and paper trading)"
    }
}

pub const TOOL_PAPER_TRADING: &str = "defi/paper_trading";
pub const TOOL_QUERY_SUBGRAPH: &str = "defi/query_subgraph";
pub const TOOL_ODOS_SWAP: &str = "defi/odos_swap";
pub const TOOL_WALLET_BALANCE: &str = "defi/wallet_balance";
pub const TOOL_WALLET_DERIVE_ADDRESS: &str = "defi/wallet_derive_address";
pub const TOOL_WALLET_SIGN_MESSAGE: &str = "defi/wallet_sign_message";
pub const TOOL_WALLET_SIGN_TX: &str = "defi/wallet_sign_tx";
