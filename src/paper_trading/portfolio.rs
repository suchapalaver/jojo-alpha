//! Paper trading portfolio management
//!
//! Tracks hypothetical holdings, executed trades, and P&L metrics.

use alloy::primitives::{Address, U256};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::tokens::{addresses, registry};

/// A simulated portfolio for paper trading
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaperPortfolio {
    /// Initial capital in USD (for P&L calculation)
    pub initial_usd: f64,
    /// Current holdings: token_address -> amount (in wei/smallest unit)
    pub holdings: HashMap<Address, U256>,
    /// All executed paper trades
    pub trades: Vec<PaperTrade>,
    /// Current P&L metrics
    pub metrics: PnLMetrics,
    /// Last known prices for tokens (for unrealized P&L)
    #[serde(default)]
    pub prices: HashMap<Address, f64>,
    /// Timestamp of portfolio creation
    pub created_at: DateTime<Utc>,
    /// Timestamp of last update
    pub updated_at: DateTime<Utc>,
}

/// A single paper trade
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaperTrade {
    /// When the trade was executed
    pub timestamp: DateTime<Utc>,
    /// Token sold
    pub input_token: Address,
    /// Token bought
    pub output_token: Address,
    /// Amount sold (in wei)
    pub input_amount: String, // Store as string for JSON serialization
    /// Amount received (in wei)
    pub output_amount: String, // Store as string for JSON serialization
    /// USD price of input token at trade time
    pub input_price_usd: f64,
    /// USD price of output token at trade time
    pub output_price_usd: f64,
    /// USD value of the trade
    pub trade_value_usd: f64,
    /// Expected output from quote (for slippage calculation)
    pub expected_output: String,
    /// Chain ID where trade would execute
    pub chain_id: u64,
    /// Realized P&L from this trade (if closing a position)
    pub realized_pnl_usd: Option<f64>,
}

/// P&L and performance metrics
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PnLMetrics {
    /// Realized P&L from closed positions
    pub realized_pnl_usd: f64,
    /// Unrealized P&L from open positions
    pub unrealized_pnl_usd: f64,
    /// Total P&L (realized + unrealized)
    pub total_pnl_usd: f64,
    /// Total P&L as percentage of initial capital
    pub total_pnl_percent: f64,
    /// Number of winning trades
    pub winning_trades: u32,
    /// Number of losing trades
    pub losing_trades: u32,
    /// Win rate (0-1)
    pub win_rate: f64,
    /// Total volume traded in USD
    pub total_volume_usd: f64,
    /// Number of trades executed
    pub total_trades: u32,
}

impl PaperPortfolio {
    /// Create a new paper portfolio with initial USDC balance
    pub fn new(initial_usd: f64) -> Self {
        let now = Utc::now();
        let mut holdings = HashMap::new();

        // Convert initial USD to USDC (6 decimals)
        // Use Ethereum USDC as default
        let usdc_amount = U256::from((initial_usd * 1_000_000.0) as u64);
        holdings.insert(addresses::USDC_ETH, usdc_amount);

        // Set initial price for USDC
        let mut prices = HashMap::new();
        prices.insert(addresses::USDC_ETH, 1.0);

        Self {
            initial_usd,
            holdings,
            trades: Vec::new(),
            metrics: PnLMetrics::default(),
            prices,
            created_at: now,
            updated_at: now,
        }
    }

    /// Execute a paper swap
    #[allow(clippy::too_many_arguments)]
    pub fn execute_swap(
        &mut self,
        input_token: Address,
        output_token: Address,
        input_amount: U256,
        expected_output: U256,
        input_price_usd: f64,
        output_price_usd: f64,
        chain_id: u64,
    ) -> Result<PaperTrade, String> {
        // Check if we have enough balance
        let current_balance = self
            .holdings
            .get(&input_token)
            .copied()
            .unwrap_or(U256::ZERO);
        if current_balance < input_amount {
            return Err(format!(
                "Insufficient balance: have {} but need {}",
                current_balance, input_amount
            ));
        }

        // Calculate trade value in USD
        let input_decimals = get_token_decimals(&input_token);
        let trade_value_usd = calculate_usd_value(input_amount, input_decimals, input_price_usd);

        // Deduct input token
        let new_input_balance = current_balance - input_amount;
        if new_input_balance.is_zero() {
            self.holdings.remove(&input_token);
        } else {
            self.holdings.insert(input_token, new_input_balance);
        }

        // Add output token
        let current_output = self
            .holdings
            .get(&output_token)
            .copied()
            .unwrap_or(U256::ZERO);
        self.holdings
            .insert(output_token, current_output + expected_output);

        // Update prices
        self.prices.insert(input_token, input_price_usd);
        self.prices.insert(output_token, output_price_usd);

        // Create trade record
        let trade = PaperTrade {
            timestamp: Utc::now(),
            input_token,
            output_token,
            input_amount: input_amount.to_string(),
            output_amount: expected_output.to_string(),
            input_price_usd,
            output_price_usd,
            trade_value_usd,
            expected_output: expected_output.to_string(),
            chain_id,
            realized_pnl_usd: None, // TODO: Calculate if closing a position
        };

        self.trades.push(trade.clone());

        // Update metrics
        self.metrics.total_trades += 1;
        self.metrics.total_volume_usd += trade_value_usd;
        self.updated_at = Utc::now();

        // Recalculate unrealized P&L
        self.recalculate_metrics();

        Ok(trade)
    }

    /// Update price for a token (for unrealized P&L calculation)
    pub fn update_price(&mut self, token: &Address, price_usd: f64) {
        self.prices.insert(*token, price_usd);
        self.recalculate_metrics();
    }

    /// Recalculate all metrics based on current holdings and prices
    fn recalculate_metrics(&mut self) {
        let mut total_value_usd = 0.0;

        for (token, amount) in &self.holdings {
            if let Some(&price) = self.prices.get(token) {
                let decimals = get_token_decimals(token);
                let value = calculate_usd_value(*amount, decimals, price);
                total_value_usd += value;
            }
        }

        self.metrics.unrealized_pnl_usd = total_value_usd - self.initial_usd;
        self.metrics.total_pnl_usd =
            self.metrics.realized_pnl_usd + self.metrics.unrealized_pnl_usd;

        if self.initial_usd > 0.0 {
            self.metrics.total_pnl_percent =
                (self.metrics.total_pnl_usd / self.initial_usd) * 100.0;
        }

        // Update win rate
        let total_result_trades = self.metrics.winning_trades + self.metrics.losing_trades;
        if total_result_trades > 0 {
            self.metrics.win_rate = self.metrics.winning_trades as f64 / total_result_trades as f64;
        }
    }

    /// Get current portfolio value in USD
    pub fn total_value_usd(&self) -> f64 {
        let mut total = 0.0;
        for (token, amount) in &self.holdings {
            if let Some(&price) = self.prices.get(token) {
                let decimals = get_token_decimals(token);
                total += calculate_usd_value(*amount, decimals, price);
            }
        }
        total
    }
}

/// Get token decimals from registry or default
fn get_token_decimals(token: &Address) -> u8 {
    registry()
        .get(token)
        .map(|info| info.decimals)
        .unwrap_or(18)
}

/// Calculate USD value from amount, decimals, and price
fn calculate_usd_value(amount: U256, decimals: u8, price_usd: f64) -> f64 {
    // Convert U256 to f64 carefully to avoid overflow
    let divisor = 10u64.pow(decimals as u32);
    let amount_str = amount.to_string();

    // Parse as f64 and divide by decimals
    if let Ok(amount_f64) = amount_str.parse::<f64>() {
        (amount_f64 / divisor as f64) * price_usd
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_portfolio() {
        let portfolio = PaperPortfolio::new(10000.0);
        assert_eq!(portfolio.initial_usd, 10000.0);

        // Should have USDC balance
        let usdc_balance = portfolio.holdings.get(&addresses::USDC_ETH);
        assert!(usdc_balance.is_some());

        // 10000 USDC = 10000 * 1e6
        let expected = U256::from(10_000_000_000u64);
        assert_eq!(*usdc_balance.unwrap(), expected);
    }

    #[test]
    fn test_execute_swap() {
        let mut portfolio = PaperPortfolio::new(10000.0);

        // Swap 1000 USDC for WETH
        let input_amount = U256::from(1_000_000_000u64); // 1000 USDC
        let expected_output = U256::from(330_000_000_000_000_000u128); // ~0.33 ETH

        let result = portfolio.execute_swap(
            addresses::USDC_ETH,
            addresses::WETH_ETH,
            input_amount,
            expected_output,
            1.0,    // USDC price
            3000.0, // ETH price
            1,      // Ethereum
        );

        assert!(result.is_ok());

        // Check balances updated
        let remaining_usdc = portfolio.holdings.get(&addresses::USDC_ETH);
        assert!(remaining_usdc.is_some());
        assert_eq!(
            *remaining_usdc.unwrap(),
            U256::from(9_000_000_000u64) // 9000 USDC
        );

        let weth_balance = portfolio.holdings.get(&addresses::WETH_ETH);
        assert!(weth_balance.is_some());
        assert_eq!(*weth_balance.unwrap(), expected_output);

        // Check trade recorded
        assert_eq!(portfolio.trades.len(), 1);
        assert_eq!(portfolio.metrics.total_trades, 1);
    }

    #[test]
    fn test_insufficient_balance() {
        let mut portfolio = PaperPortfolio::new(100.0); // Only $100

        // Try to swap 1000 USDC
        let input_amount = U256::from(1_000_000_000u64); // 1000 USDC

        let result = portfolio.execute_swap(
            addresses::USDC_ETH,
            addresses::WETH_ETH,
            input_amount,
            U256::from(330_000_000_000_000_000u128),
            1.0,
            3000.0,
            1,
        );

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Insufficient balance"));
    }

    #[test]
    fn test_calculate_usd_value() {
        // 1000 USDC (6 decimals) at $1 = $1000
        let usdc_amount = U256::from(1_000_000_000u64);
        assert!((calculate_usd_value(usdc_amount, 6, 1.0) - 1000.0).abs() < 0.01);

        // 1 ETH (18 decimals) at $3000 = $3000
        let eth_amount = U256::from(1_000_000_000_000_000_000u128);
        assert!((calculate_usd_value(eth_amount, 18, 3000.0) - 3000.0).abs() < 0.01);
    }

    #[test]
    fn test_total_value_usd() {
        let portfolio = PaperPortfolio::new(10000.0);

        // Initial value should be $10,000
        let total = portfolio.total_value_usd();
        assert!((total - 10000.0).abs() < 1.0);
    }
}
