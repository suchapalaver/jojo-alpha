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
        Commands::Run { agent, dry_run } => {
            run_agent(agent, config, dry_run).await?;
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
            println!("{}", serde_json::to_string_pretty(&config).unwrap());
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
    }

    Ok(())
}

async fn run_agent(agent_path: PathBuf, config: Config, dry_run: bool) -> Result<()> {
    use defi_trading_agent::wallet::SecureWallet;
    use defi_trading_agent::AgentRunner;

    tracing::info!(
        networks = ?config.networks,
        protocols = ?config.protocols,
        dry_run = dry_run,
        "Starting trading agent"
    );

    // Create the agent runner
    let mut runner = AgentRunner::new(config, dry_run);

    // Try to load wallet from environment if available
    if let Ok(private_key) = std::env::var("PRIVATE_KEY") {
        match SecureWallet::from_hex(&private_key) {
            Ok(wallet) => {
                tracing::info!(address = %wallet.address(), "Loaded wallet from PRIVATE_KEY");
                runner = runner.with_wallet(wallet);
            }
            Err(e) => {
                tracing::warn!(error = %e, "Failed to load wallet from PRIVATE_KEY");
            }
        }
    } else if !dry_run {
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
    use defi_trading_agent::tools::TheGraphTool;

    let tool = TheGraphTool::new();
    let params_value: serde_json::Value = params
        .map(|p| serde_json::from_str(&p).unwrap_or(serde_json::json!({})))
        .unwrap_or(serde_json::json!({}));

    let args = serde_json::json!({
        "protocol": protocol,
        "network": network,
        "query_type": query_type,
        "params": params_value
    });

    let result = tool
        .execute(args)
        .await
        .map_err(|e| defi_trading_agent::Error::GraphQL(e.to_string()))?;

    println!("{}", serde_json::to_string_pretty(&result).unwrap());
    Ok(())
}

async fn run_quote(input: String, output: String, amount: String, network: String) -> Result<()> {
    use baml_rt::tools::BamlTool;
    use defi_trading_agent::tools::OdosTool;

    // For quote, we don't need a real wallet address
    let tool = OdosTool::new("0x0000000000000000000000000000000000000000");

    let args = serde_json::json!({
        "action": "quote",
        "input_token": input,
        "output_token": output,
        "amount": amount,
        "network": network
    });

    let result = tool
        .execute(args)
        .await
        .map_err(|e| defi_trading_agent::Error::Odos(e.to_string()))?;

    println!("{}", serde_json::to_string_pretty(&result).unwrap());
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
