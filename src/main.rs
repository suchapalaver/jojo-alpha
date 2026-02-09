//! DeFi Trading Agent CLI
//!
//! Command-line interface for running the AI-powered trading agent.

use clap::{Parser, Subcommand};
use defi_trading_agent::{Config, Result};
use std::path::PathBuf;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

#[derive(Parser)]
#[command(name = "defi-agent")]
#[command(about = "AI-powered DeFi trading agent")]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Path to config file
    #[arg(short, long, global = true)]
    config: Option<PathBuf>,

    /// Enable verbose logging
    #[arg(short, long, global = true)]
    verbose: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Run the trading agent
    Run {
        /// Path to the agent package (tar.gz or directory)
        #[arg(short, long)]
        agent: PathBuf,

        /// Dry run - don't execute trades, just log what would happen
        #[arg(long)]
        dry_run: bool,

        /// Paper trading mode - simulate trades without real execution
        #[arg(long)]
        paper_trading: bool,

        /// Initial balance for paper trading (USD, default: 10000)
        #[arg(long, default_value = "10000")]
        initial_balance: f64,

        /// File to persist paper trading state
        #[arg(long)]
        paper_state_file: Option<PathBuf>,
    },

    /// Query The Graph subgraphs
    Query {
        /// Protocol to query (uniswap_v3, aave_v3)
        #[arg(short, long)]
        protocol: String,

        /// Network (ethereum, arbitrum, optimism, base)
        #[arg(short, long)]
        network: String,

        /// Query type (top_pools, pool_info, token_price)
        #[arg(short = 't', long)]
        query_type: String,

        /// Additional parameters as JSON
        #[arg(short = 'P', long)]
        params: Option<String>,
    },

    /// Get a swap quote from Odos
    Quote {
        /// Input token address
        #[arg(long)]
        input: String,

        /// Output token address
        #[arg(long)]
        output: String,

        /// Amount in wei
        #[arg(long)]
        amount: String,

        /// Network (ethereum, arbitrum, optimism, base)
        #[arg(short, long, default_value = "ethereum")]
        network: String,
    },

    /// Show current configuration
    Config,

    /// Simulate a transaction using eth_call
    Simulate {
        /// Target contract address
        #[arg(long)]
        to: String,

        /// Calldata (hex encoded, with or without 0x prefix)
        #[arg(long)]
        data: String,

        /// From address (defaults to zero address)
        #[arg(long)]
        from: Option<String>,

        /// Value in wei (defaults to 0)
        #[arg(long)]
        value: Option<String>,

        /// Network (ethereum, arbitrum, optimism, base)
        #[arg(short, long, default_value = "ethereum")]
        network: String,
    },

    /// Get real-time token price in USD via Odos
    Price {
        /// Token address (or multiple comma-separated addresses)
        #[arg(long)]
        token: String,

        /// Network (ethereum, arbitrum, optimism, base)
        #[arg(short, long, default_value = "ethereum")]
        network: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    // Load .env file if present (ignore if not found)
    dotenvy::dotenv().ok();

    let cli = Cli::parse();

    // Initialize logging
    let filter = if cli.verbose {
        EnvFilter::new("debug")
    } else {
        EnvFilter::new("info")
    };

    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(filter)
        .init();

    // Load config
    let config = if let Some(config_path) = cli.config {
        let content = std::fs::read_to_string(&config_path)
            .map_err(|e| defi_trading_agent::Error::Config(e.to_string()))?;
        serde_json::from_str(&content)
            .map_err(|e| defi_trading_agent::Error::Config(e.to_string()))?
    } else {
        Config::default()
    };

    match cli.command {
        Commands::Run {
            agent,
            dry_run,
            paper_trading,
            initial_balance,
            paper_state_file,
        } => {
            run_agent(
                agent,
                config,
                dry_run,
                paper_trading,
                initial_balance,
                paper_state_file,
            )
            .await?;
        }
        Commands::Query {
            protocol,
            network,
            query_type,
            params,
        } => {
            run_query(protocol, network, query_type, params).await?;
        }
        Commands::Quote {
            input,
            output,
            amount,
            network,
        } => {
            run_quote(input, output, amount, network).await?;
        }
        Commands::Config => {
            print_pretty(&config)?;
        }
        Commands::Simulate {
            to,
            data,
            from,
            value,
            network,
        } => {
            run_simulate(to, data, from, value, network).await?;
        }
        Commands::Price { token, network } => {
            run_price(token, network).await?;
        }
    }

    Ok(())
}

async fn run_agent(
    agent_path: PathBuf,
    config: Config,
    dry_run: bool,
    paper_trading: bool,
    initial_balance: f64,
    paper_state_file: Option<PathBuf>,
) -> Result<()> {
    use defi_trading_agent::wallet::SecureWallet;
    use defi_trading_agent::{AgentRunner, PaperModeConfig, PaperTradingState};

    tracing::info!(
        networks = ?config.networks,
        protocols = ?config.protocols,
        dry_run = dry_run,
        paper_trading = paper_trading,
        initial_balance = initial_balance,
        "Starting trading agent"
    );

    // Create the agent runner
    let mut runner = AgentRunner::new(config, dry_run);

    // Set up paper trading if enabled
    if paper_trading {
        let paper_config = PaperModeConfig {
            enabled: true,
            initial_balance_usd: initial_balance,
            state_file: paper_state_file.map(|p| p.to_string_lossy().to_string()),
        };

        let paper_state = PaperTradingState::load_or_create(&paper_config)
            .await
            .map_err(|e| {
                defi_trading_agent::Error::Config(format!(
                    "Failed to load paper trading state: {}",
                    e
                ))
            })?;

        let portfolio = paper_state.get_portfolio().await;
        tracing::info!(
            initial_usd = portfolio.initial_usd,
            total_trades = portfolio.metrics.total_trades,
            total_pnl_usd = portfolio.metrics.total_pnl_usd,
            "Paper trading mode enabled"
        );

        runner = runner.with_paper_trading(paper_state);
    }

    // Try to load wallet from environment if available
    if let Ok(private_key) = std::env::var("PRIVATE_KEY") {
        match SecureWallet::from_hex(&private_key) {
            Ok(wallet) => {
                let wallet = wallet.with_dry_run(dry_run);
                tracing::info!(address = %wallet.address(), "Loaded wallet from PRIVATE_KEY");
                runner = runner.with_wallet(wallet);
            }
            Err(e) => {
                tracing::warn!(error = %e, "Failed to load wallet from PRIVATE_KEY");
            }
        }
    } else if !dry_run && !paper_trading {
        tracing::warn!("No PRIVATE_KEY set - running in read-only mode (quotes only)");
    }

    // Run the agent
    runner.run(&agent_path).await
}

async fn run_query(
    protocol: String,
    network: String,
    query_type: String,
    params: Option<String>,
) -> Result<()> {
    use baml_rt::tools::BamlTool;
    use defi_trading_agent::config::GRAPH_API_KEY_ENV;
    use defi_trading_agent::tools::{
        GraphQueryInput, GraphQueryParams, GraphQueryType, TheGraphTool,
    };

    // Use gateway-enabled tool if API key is available
    let tool = match std::env::var(GRAPH_API_KEY_ENV) {
        Ok(api_key) => TheGraphTool::with_gateway(api_key),
        Err(_) => TheGraphTool::new(),
    };
    let params_value: serde_json::Value = match params {
        Some(p) => serde_json::from_str(&p).map_err(|e| {
            defi_trading_agent::Error::InvalidArgument(format!("Invalid --params JSON: {}", e))
        })?,
        None => serde_json::json!({}),
    };

    let query_type = match query_type.as_str() {
        "top_pools" => GraphQueryType::TopPools,
        "pool_info" => GraphQueryType::PoolInfo,
        "token_price" => GraphQueryType::TokenPrice,
        "filtered_pools" => GraphQueryType::FilteredPools,
        "query_plan" => GraphQueryType::QueryPlan,
        other => {
            return Err(defi_trading_agent::Error::Config(format!(
                "Unknown query_type: {}",
                other
            )))
        }
    };

    let params = if params_value.is_null()
        || params_value
            .as_object()
            .map(|m| m.is_empty())
            .unwrap_or(false)
    {
        None
    } else {
        Some(
            serde_json::from_value::<GraphQueryParams>(params_value)
                .map_err(|e| defi_trading_agent::Error::Config(format!("Invalid params: {}", e)))?,
        )
    };

    let args = GraphQueryInput {
        protocol,
        network,
        query_type,
        params,
    };

    let result = tool
        .execute(args)
        .await
        .map_err(|e| defi_trading_agent::Error::GraphQL(e.to_string()))?;

    print_pretty(&result.0)?;
    Ok(())
}

async fn run_quote(input: String, output: String, amount: String, network: String) -> Result<()> {
    use baml_rt::tools::BamlTool;
    use defi_trading_agent::tools::{OdosAction, OdosInput, OdosTool};

    // For quote, we don't need a real wallet address
    let tool = OdosTool::new("0x0000000000000000000000000000000000000000");

    let args = OdosInput {
        action: OdosAction::Quote,
        input_token: Some(input),
        output_token: Some(output),
        amount: Some(amount),
        token: None,
        tokens: None,
        slippage_percent: None,
        chain_id: None,
        network: Some(network),
    };

    let result = tool
        .execute(args)
        .await
        .map_err(|e| defi_trading_agent::Error::Odos(e.to_string()))?;

    print_pretty(&result.0)?;
    Ok(())
}

async fn run_price(token: String, network: String) -> Result<()> {
    use baml_rt::tools::BamlTool;
    use defi_trading_agent::tools::{OdosAction, OdosInput, OdosTool};

    // For price lookup, we don't need a real wallet address
    let tool = OdosTool::new("0x0000000000000000000000000000000000000000");

    // Check if multiple tokens (comma-separated)
    let tokens: Vec<&str> = token.split(',').map(|s| s.trim()).collect();

    if tokens.len() > 1 {
        // Batch price lookup
        let args = OdosInput {
            action: OdosAction::GetPrices,
            input_token: None,
            output_token: None,
            amount: None,
            token: None,
            tokens: Some(tokens.iter().map(|s| s.to_string()).collect()),
            slippage_percent: None,
            chain_id: None,
            network: Some(network.clone()),
        };

        let result = tool
            .execute(args)
            .await
            .map_err(|e| defi_trading_agent::Error::Odos(e.to_string()))?;

        print_pretty(&result.0)?;
    } else {
        // Single token price
        let args = OdosInput {
            action: OdosAction::GetPrice,
            input_token: None,
            output_token: None,
            amount: None,
            token: Some(tokens[0].to_string()),
            tokens: None,
            slippage_percent: None,
            chain_id: None,
            network: Some(network),
        };

        let result = tool
            .execute(args)
            .await
            .map_err(|e| defi_trading_agent::Error::Odos(e.to_string()))?;

        print_pretty(&result.0)?;
    }

    Ok(())
}

async fn run_simulate(
    to: String,
    data: String,
    from: Option<String>,
    value: Option<String>,
    network: String,
) -> Result<()> {
    use defi_trading_agent::config::RpcConfig;
    use defi_trading_agent::wallet::TransactionSimulator;

    let rpc_config = RpcConfig::from_env();

    // Parse network to chain_id
    let chain_id = match network.to_lowercase().as_str() {
        "ethereum" | "mainnet" => 1,
        "arbitrum" => 42161,
        "optimism" => 10,
        "base" => 8453,
        _ => {
            return Err(defi_trading_agent::Error::InvalidArgument(format!(
                "Unknown network: {}",
                network
            )));
        }
    };

    let simulator = TransactionSimulator::from_rpc_config(&rpc_config, chain_id)
        .map_err(|e| defi_trading_agent::Error::Simulation(e.to_string()))?;

    let from_addr =
        from.unwrap_or_else(|| "0x0000000000000000000000000000000000000000".to_string());

    tracing::info!(
        from = %from_addr,
        to = %to,
        data_len = data.len(),
        network = %network,
        "Simulating transaction"
    );

    let result = simulator
        .simulate(&from_addr, &to, &data, value.as_deref())
        .await
        .map_err(|e| defi_trading_agent::Error::Simulation(e.to_string()))?;

    if result.success {
        println!("Simulation SUCCEEDED");
        if let Some(gas) = result.gas_used {
            println!("  Gas used: {}", gas);
        }
        if let Some(data) = result.return_data {
            if data != "0x" && !data.is_empty() {
                println!("  Return data: {}", data);
            }
        }
    } else {
        println!("Simulation FAILED");
        if let Some(reason) = result.revert_reason {
            println!("  Revert reason: {}", reason);
        }
    }

    Ok(())
}

fn print_pretty<T: serde::Serialize>(value: &T) -> Result<()> {
    let rendered = serde_json::to_string_pretty(value).map_err(defi_trading_agent::Error::Json)?;
    println!("{}", rendered);
    Ok(())
}
