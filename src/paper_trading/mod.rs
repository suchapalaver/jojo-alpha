//! Paper trading module
//!
//! Provides simulated trading capabilities that:
//! - Track hypothetical positions starting from configurable initial balances
//! - Record trades that WOULD have executed (using real quotes from Odos)
//! - Calculate P&L over time
//! - Persist state across restarts (optional)
//!
//! SECURITY NOTE:
//! - Paper trading never signs or submits actual transactions
//! - All prices come from real Odos quotes for accurate simulation
//! - State is stored locally and can be reset at any time

mod portfolio;

pub use portfolio::{PaperPortfolio, PaperTrade, PnLMetrics};

use alloy::primitives::{Address, U256};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Paper trading configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaperModeConfig {
    /// Enable paper trading mode
    pub enabled: bool,
    /// Initial balance in USD (default 10,000)
    pub initial_balance_usd: f64,
    /// Path to persist state (optional)
    pub state_file: Option<String>,
}

impl Default for PaperModeConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            initial_balance_usd: 10_000.0,
            state_file: None,
        }
    }
}

/// Thread-safe paper trading state manager
#[derive(Clone)]
pub struct PaperTradingState {
    portfolio: Arc<RwLock<PaperPortfolio>>,
    enabled: bool,
    state_file: Option<String>,
}

impl PaperTradingState {
    /// Create a new paper trading state with initial USD balance
    ///
    /// The initial balance is converted to USDC in the portfolio
    pub fn new(config: &PaperModeConfig) -> Self {
        let portfolio = PaperPortfolio::new(config.initial_balance_usd);
        Self {
            portfolio: Arc::new(RwLock::new(portfolio)),
            enabled: config.enabled,
            state_file: config.state_file.clone(),
        }
    }

    /// Load state from a file, or create new if file doesn't exist
    pub async fn load_or_create(config: &PaperModeConfig) -> std::io::Result<Self> {
        if let Some(ref path) = config.state_file {
            if Path::new(path).exists() {
                let content = tokio::fs::read_to_string(path).await?;
                let portfolio: PaperPortfolio = serde_json::from_str(&content)
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
                return Ok(Self {
                    portfolio: Arc::new(RwLock::new(portfolio)),
                    enabled: config.enabled,
                    state_file: config.state_file.clone(),
                });
            }
        }
        Ok(Self::new(config))
    }

    /// Check if paper trading is enabled
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Execute a hypothetical swap
    ///
    /// This updates the paper portfolio as if the trade executed
    #[allow(clippy::too_many_arguments)]
    pub async fn execute_swap(
        &self,
        input_token: Address,
        output_token: Address,
        input_amount: U256,
        expected_output: U256,
        input_price_usd: f64,
        output_price_usd: f64,
        chain_id: u64,
    ) -> Result<PaperTrade, String> {
        let mut portfolio = self.portfolio.write().await;
        let trade = portfolio.execute_swap(
            input_token,
            output_token,
            input_amount,
            expected_output,
            input_price_usd,
            output_price_usd,
            chain_id,
        )?;

        // Auto-save if state file is configured
        if let Some(ref path) = self.state_file {
            if let Err(e) = self.save_to_file_internal(&portfolio, path).await {
                tracing::warn!("Failed to auto-save paper trading state: {}", e);
            }
        }

        Ok(trade)
    }

    /// Get current portfolio state (snapshot)
    pub async fn get_portfolio(&self) -> PaperPortfolio {
        self.portfolio.read().await.clone()
    }

    /// Get current P&L metrics
    pub async fn get_metrics(&self) -> PnLMetrics {
        self.portfolio.read().await.metrics.clone()
    }

    /// Get balance for a specific token
    pub async fn get_balance(&self, token: &Address) -> U256 {
        self.portfolio
            .read()
            .await
            .holdings
            .get(token)
            .copied()
            .unwrap_or(U256::ZERO)
    }

    /// Get all non-zero balances
    pub async fn get_all_balances(&self) -> Vec<(Address, U256)> {
        self.portfolio
            .read()
            .await
            .holdings
            .iter()
            .filter(|(_, &amount)| !amount.is_zero())
            .map(|(addr, amount)| (*addr, *amount))
            .collect()
    }

    /// Save state to the configured file
    pub async fn save(&self) -> std::io::Result<()> {
        if let Some(ref path) = self.state_file {
            let portfolio = self.portfolio.read().await;
            self.save_to_file_internal(&portfolio, path).await
        } else {
            Ok(())
        }
    }

    /// Internal save helper
    async fn save_to_file_internal(
        &self,
        portfolio: &PaperPortfolio,
        path: &str,
    ) -> std::io::Result<()> {
        let content = serde_json::to_string_pretty(portfolio)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        tokio::fs::write(path, content).await
    }

    /// Update token price for unrealized P&L calculation
    pub async fn update_price(&self, token: &Address, price_usd: f64) {
        let mut portfolio = self.portfolio.write().await;
        portfolio.update_price(token, price_usd);
    }

    /// Get recent trades
    pub async fn get_trades(&self, limit: Option<usize>) -> Vec<PaperTrade> {
        let portfolio = self.portfolio.read().await;
        let trades = &portfolio.trades;
        match limit {
            Some(n) => trades.iter().rev().take(n).cloned().collect(),
            None => trades.clone(),
        }
    }

    /// Reset the portfolio to initial state
    pub async fn reset(&self, initial_balance_usd: f64) {
        let mut portfolio = self.portfolio.write().await;
        *portfolio = PaperPortfolio::new(initial_balance_usd);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_paper_mode_config_default() {
        let config = PaperModeConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.initial_balance_usd, 10_000.0);
        assert!(config.state_file.is_none());
    }

    #[tokio::test]
    async fn test_paper_trading_state_creation() {
        let config = PaperModeConfig {
            enabled: true,
            initial_balance_usd: 5000.0,
            state_file: None,
        };
        let state = PaperTradingState::new(&config);
        assert!(state.is_enabled());

        let portfolio = state.get_portfolio().await;
        assert_eq!(portfolio.initial_usd, 5000.0);
    }
}
