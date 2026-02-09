//! Paper trading tool
//!
//! Provides paper trading capabilities via the tool interface:
//! - Execute simulated swaps with real price data
//! - Query paper portfolio balances
//! - Get P&L metrics
//!
//! SECURITY NOTE:
//! - This tool never submits real transactions
//! - All operations are simulated in-memory
//! - Real price data comes from Odos quotes

use crate::paper_trading::PaperTradingState;
use crate::tokens::registry;
use crate::tools::{AnyJson, DefiBundle};
use alloy::primitives::{Address, U256};
use async_trait::async_trait;
use baml_rt::error::{BamlRtError, Result};
use baml_rt::tools::BamlTool;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::str::FromStr;
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum PaperTradingAction {
    ExecuteSwap,
    GetBalances,
    GetMetrics,
    GetTrades,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[ts(export)]
pub struct PaperTradingInput {
    pub action: PaperTradingAction,
    pub input_token: Option<String>,
    pub output_token: Option<String>,
    pub input_amount: Option<String>,
    pub expected_output: Option<String>,
    pub input_price_usd: Option<f64>,
    pub output_price_usd: Option<f64>,
    pub chain_id: Option<u64>,
    pub limit: Option<u64>,
}

/// Tool for paper trading operations
pub struct PaperTradingTool {
    state: PaperTradingState,
}

impl PaperTradingTool {
    /// Create a new PaperTradingTool with the given state
    pub fn new(state: PaperTradingState) -> Self {
        Self { state }
    }

    /// Execute a paper swap
    async fn execute_swap(&self, args: &Value) -> Result<Value> {
        let input_token = args
            .get("input_token")
            .and_then(|v| v.as_str())
            .ok_or_else(|| BamlRtError::InvalidArgument("Missing 'input_token'".to_string()))?;

        let output_token = args
            .get("output_token")
            .and_then(|v| v.as_str())
            .ok_or_else(|| BamlRtError::InvalidArgument("Missing 'output_token'".to_string()))?;

        let input_amount = args
            .get("input_amount")
            .and_then(|v| v.as_str())
            .ok_or_else(|| BamlRtError::InvalidArgument("Missing 'input_amount'".to_string()))?;

        let expected_output = args
            .get("expected_output")
            .and_then(|v| v.as_str())
            .ok_or_else(|| BamlRtError::InvalidArgument("Missing 'expected_output'".to_string()))?;

        let input_price_usd = args
            .get("input_price_usd")
            .and_then(|v| v.as_f64())
            .ok_or_else(|| BamlRtError::InvalidArgument("Missing 'input_price_usd'".to_string()))?;

        let output_price_usd = args
            .get("output_price_usd")
            .and_then(|v| v.as_f64())
            .ok_or_else(|| {
                BamlRtError::InvalidArgument("Missing 'output_price_usd'".to_string())
            })?;

        let chain_id = args.get("chain_id").and_then(|v| v.as_u64()).unwrap_or(1);

        // Parse addresses and amounts
        let input_addr = Address::from_str(input_token)
            .map_err(|e| BamlRtError::InvalidArgument(format!("Invalid input token: {}", e)))?;
        let output_addr = Address::from_str(output_token)
            .map_err(|e| BamlRtError::InvalidArgument(format!("Invalid output token: {}", e)))?;
        let input_amt = U256::from_str(input_amount)
            .map_err(|e| BamlRtError::InvalidArgument(format!("Invalid input amount: {}", e)))?;
        let expected_out = U256::from_str(expected_output)
            .map_err(|e| BamlRtError::InvalidArgument(format!("Invalid expected output: {}", e)))?;

        // Execute paper swap
        let trade = self
            .state
            .execute_swap(
                input_addr,
                output_addr,
                input_amt,
                expected_out,
                input_price_usd,
                output_price_usd,
                chain_id,
            )
            .await
            .map_err(|e| BamlRtError::ToolExecution(format!("Paper swap failed: {}", e)))?;

        // Get updated metrics
        let metrics = self.state.get_metrics().await;

        Ok(json!({
            "action": "execute_swap",
            "status": "executed_on_paper",
            "trade": {
                "timestamp": trade.timestamp.to_rfc3339(),
                "input_token": input_token,
                "output_token": output_token,
                "input_amount": trade.input_amount,
                "output_amount": trade.output_amount,
                "trade_value_usd": trade.trade_value_usd,
            },
            "portfolio_metrics": {
                "total_pnl_usd": metrics.total_pnl_usd,
                "total_pnl_percent": metrics.total_pnl_percent,
                "total_trades": metrics.total_trades,
                "total_volume_usd": metrics.total_volume_usd,
            }
        }))
    }

    /// Get paper portfolio balances
    async fn get_balances(&self, args: &Value) -> Result<Value> {
        let chain_id = args.get("chain_id").and_then(|v| v.as_u64()).unwrap_or(1);

        let balances = self.state.get_all_balances().await;

        let formatted_balances: Vec<Value> = balances
            .iter()
            .map(|(addr, amount)| {
                let token_info = registry().get(addr);
                let symbol = token_info.map(|i| i.symbol).unwrap_or("UNKNOWN");
                let decimals = token_info.map(|i| i.decimals).unwrap_or(18);

                // Format balance
                let balance_formatted = format_units(*amount, decimals as u32);

                json!({
                    "token": addr.to_string(),
                    "symbol": symbol,
                    "balance_raw": amount.to_string(),
                    "balance_formatted": balance_formatted,
                    "decimals": decimals,
                    "is_native": false
                })
            })
            .collect();

        Ok(json!({
            "action": "get_balances",
            "chain_id": chain_id,
            "balances": formatted_balances,
            "note": "Paper trading balances (simulated)"
        }))
    }

    /// Get portfolio metrics
    async fn get_metrics(&self) -> Result<Value> {
        let metrics = self.state.get_metrics().await;
        let portfolio = self.state.get_portfolio().await;

        Ok(json!({
            "action": "get_metrics",
            "initial_balance_usd": portfolio.initial_usd,
            "current_value_usd": portfolio.total_value_usd(),
            "realized_pnl_usd": metrics.realized_pnl_usd,
            "unrealized_pnl_usd": metrics.unrealized_pnl_usd,
            "total_pnl_usd": metrics.total_pnl_usd,
            "total_pnl_percent": metrics.total_pnl_percent,
            "total_trades": metrics.total_trades,
            "total_volume_usd": metrics.total_volume_usd,
            "winning_trades": metrics.winning_trades,
            "losing_trades": metrics.losing_trades,
            "win_rate": metrics.win_rate,
            "created_at": portfolio.created_at.to_rfc3339(),
            "updated_at": portfolio.updated_at.to_rfc3339(),
        }))
    }

    /// Get recent trades
    async fn get_trades(&self, args: &Value) -> Result<Value> {
        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize);
        let trades = self.state.get_trades(limit).await;

        let formatted_trades: Vec<Value> = trades
            .iter()
            .map(|t| {
                json!({
                    "timestamp": t.timestamp.to_rfc3339(),
                    "input_token": t.input_token.to_string(),
                    "output_token": t.output_token.to_string(),
                    "input_amount": t.input_amount,
                    "output_amount": t.output_amount,
                    "trade_value_usd": t.trade_value_usd,
                    "chain_id": t.chain_id,
                })
            })
            .collect();

        Ok(json!({
            "action": "get_trades",
            "trades": formatted_trades,
            "total_count": trades.len(),
        }))
    }
}

/// Format a U256 value with decimals
fn format_units(value: U256, decimals: u32) -> String {
    if value.is_zero() {
        return "0".to_string();
    }

    let divisor = U256::from(10).pow(U256::from(decimals));
    let whole = value / divisor;
    let remainder = value % divisor;

    if remainder.is_zero() {
        whole.to_string()
    } else {
        let remainder_str = format!("{:0>width$}", remainder, width = decimals as usize);
        let trimmed = remainder_str.trim_end_matches('0');
        if trimmed.is_empty() {
            whole.to_string()
        } else {
            format!("{}.{}", whole, trimmed)
        }
    }
}

#[async_trait]
impl BamlTool for PaperTradingTool {
    type Bundle = DefiBundle;
    const LOCAL_NAME: &'static str = "paper_trading";
    type OpenInput = ();
    type Input = PaperTradingInput;
    type Output = AnyJson;

    fn description(&self) -> &'static str {
        "Paper trading tool for simulated trading. Execute hypothetical swaps, \
         query paper balances, and track P&L metrics. All operations are simulated \
         and no real transactions are submitted."
    }

    async fn execute(&self, args: Self::Input) -> Result<Self::Output> {
        let args_value = serde_json::to_value(&args)
            .map_err(|e| BamlRtError::InvalidArgument(format!("Invalid args: {}", e)))?;

        let result = match args.action {
            PaperTradingAction::ExecuteSwap => self.execute_swap(&args_value).await?,
            PaperTradingAction::GetBalances => self.get_balances(&args_value).await?,
            PaperTradingAction::GetMetrics => self.get_metrics().await?,
            PaperTradingAction::GetTrades => self.get_trades(&args_value).await?,
        };

        Ok(AnyJson::new(result))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paper_trading::PaperModeConfig;

    #[tokio::test]
    async fn test_paper_trading_tool_creation() {
        let config = PaperModeConfig {
            enabled: true,
            initial_balance_usd: 10000.0,
            state_file: None,
        };
        let state = PaperTradingState::new(&config);
        let tool = PaperTradingTool::new(state);

        assert_eq!(PaperTradingTool::name(), "defi/paper_trading");
        assert!(tool.description().contains("Paper trading"));
    }

    #[tokio::test]
    async fn test_get_metrics() {
        let config = PaperModeConfig {
            enabled: true,
            initial_balance_usd: 5000.0,
            state_file: None,
        };
        let state = PaperTradingState::new(&config);
        let tool = PaperTradingTool::new(state);

        let args = PaperTradingInput {
            action: PaperTradingAction::GetMetrics,
            input_token: None,
            output_token: None,
            input_amount: None,
            expected_output: None,
            input_price_usd: None,
            output_price_usd: None,
            chain_id: None,
            limit: None,
        };
        let result = tool.execute(args).await.unwrap().0;

        assert_eq!(result["action"], "get_metrics");
        assert_eq!(result["initial_balance_usd"], 5000.0);
    }

    #[tokio::test]
    async fn test_get_balances() {
        let config = PaperModeConfig {
            enabled: true,
            initial_balance_usd: 10000.0,
            state_file: None,
        };
        let state = PaperTradingState::new(&config);
        let tool = PaperTradingTool::new(state);

        let args = PaperTradingInput {
            action: PaperTradingAction::GetBalances,
            input_token: None,
            output_token: None,
            input_amount: None,
            expected_output: None,
            input_price_usd: None,
            output_price_usd: None,
            chain_id: Some(1),
            limit: None,
        };
        let result = tool.execute(args).await.unwrap().0;

        assert_eq!(result["action"], "get_balances");
        assert!(result["balances"].is_array());
        // Should have USDC balance
        let balances = result["balances"].as_array().unwrap();
        assert!(!balances.is_empty());
    }
}
