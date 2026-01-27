//! Agent runner module
//!
//! Loads and executes the trading agent in the QuickJS sandbox with
//! full tool and interceptor support.

use crate::config::Config;
use crate::interceptors::{
    AuditLogInterceptor, CooldownInterceptor, SlippageGuardInterceptor, SpendLimitInterceptor,
};
use crate::tools::{OdosTool, TheGraphTool};
use crate::wallet::SecureWallet;
use crate::Result;
use baml_rt::quickjs_bridge::QuickJSBridge;
use baml_rt::{Runtime, RuntimeBuilder};
use serde_json::json;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{error, info, warn};

/// Agent runner that manages the trading agent lifecycle
pub struct AgentRunner {
    config: Config,
    dry_run: bool,
    wallet: Option<SecureWallet>,
}

impl AgentRunner {
    /// Create a new agent runner
    pub fn new(config: Config, dry_run: bool) -> Self {
        Self {
            config,
            dry_run,
            wallet: None,
        }
    }

    /// Set the wallet for transaction signing
    pub fn with_wallet(mut self, wallet: SecureWallet) -> Self {
        self.wallet = Some(wallet);
        self
    }

    /// Load and run the agent from a directory
    pub async fn run(&self, agent_path: &Path) -> Result<()> {
        info!(
            agent_path = %agent_path.display(),
            dry_run = self.dry_run,
            "Starting agent runner"
        );

        // Determine if this is a directory or tar.gz
        let (baml_src, js_entry) = if agent_path.is_dir() {
            self.load_from_directory(agent_path)?
        } else {
            self.load_from_tarball(agent_path)?
        };

        // Build runtime with interceptors
        info!("Building runtime with interceptors");
        let runtime = self.build_runtime(&baml_src).await?;

        // Get QuickJS bridge and register tools
        let bridge = runtime
            .quickjs_bridge()
            .ok_or_else(|| crate::Error::BamlRuntime("QuickJS bridge not available".to_string()))?;

        self.register_tools(&runtime, &bridge).await?;

        // Load and execute agent JavaScript
        if let Some(js_path) = js_entry {
            self.load_agent_code(&bridge, &js_path).await?;
        }

        // Start the trading loop
        self.start_trading_loop(&bridge).await?;

        Ok(())
    }

    /// Load agent from a directory
    fn load_from_directory(
        &self,
        dir: &Path,
    ) -> Result<(std::path::PathBuf, Option<std::path::PathBuf>)> {
        let baml_src = dir.join("baml_src");
        if !baml_src.exists() {
            return Err(crate::Error::Config(format!(
                "Missing baml_src directory in {}",
                dir.display()
            )));
        }

        // Check for manifest
        let manifest_path = dir.join("manifest.json");
        let js_entry = if manifest_path.exists() {
            let manifest_content = std::fs::read_to_string(&manifest_path)
                .map_err(|e| crate::Error::Config(e.to_string()))?;
            let manifest: serde_json::Value = serde_json::from_str(&manifest_content)?;

            let entry_point = manifest
                .get("entry_point")
                .and_then(|v| v.as_str())
                .unwrap_or("dist/index.js");

            let js_path = dir.join(entry_point);
            if js_path.exists() {
                Some(js_path)
            } else {
                // Try src/index.ts for development
                let ts_path = dir.join("src/index.ts");
                if ts_path.exists() {
                    warn!("Using TypeScript source directly (dist/index.js not found)");
                    Some(ts_path)
                } else {
                    None
                }
            }
        } else {
            None
        };

        info!(
            baml_src = %baml_src.display(),
            js_entry = ?js_entry.as_ref().map(|p| p.display().to_string()),
            "Loaded agent from directory"
        );

        Ok((baml_src, js_entry))
    }

    /// Load agent from a tar.gz file
    fn load_from_tarball(
        &self,
        tarball: &Path,
    ) -> Result<(std::path::PathBuf, Option<std::path::PathBuf>)> {
        use flate2::read::GzDecoder;
        use tar::Archive;

        // Create temporary extraction directory
        let extract_dir = std::env::temp_dir().join(format!(
            "defi-agent-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()
        ));
        std::fs::create_dir_all(&extract_dir)
            .map_err(|e| crate::Error::Config(format!("Failed to create temp dir: {}", e)))?;

        // Extract tar.gz
        let tar_gz = std::fs::File::open(tarball)
            .map_err(|e| crate::Error::Config(format!("Failed to open tarball: {}", e)))?;
        let tar = GzDecoder::new(tar_gz);
        let mut archive = Archive::new(tar);
        archive
            .unpack(&extract_dir)
            .map_err(|e| crate::Error::Config(format!("Failed to extract tarball: {}", e)))?;

        info!(extract_dir = %extract_dir.display(), "Extracted agent tarball");

        self.load_from_directory(&extract_dir)
    }

    /// Build the BAML runtime with all interceptors
    async fn build_runtime(&self, baml_src: &Path) -> Result<baml_rt::Runtime> {
        let baml_src_str = baml_src.to_str().ok_or_else(|| {
            crate::Error::Config("BAML source path contains invalid UTF-8".to_string())
        })?;

        let mut builder = RuntimeBuilder::new()
            .with_schema_path(baml_src_str)
            .with_quickjs(true);

        // Add environment variables for LLM providers
        if let Ok(api_key) = std::env::var("OPENROUTER_API_KEY") {
            builder = builder.with_env_var("OPENROUTER_API_KEY", api_key);
        }

        // Add tool interceptors for risk management
        let risk = &self.config.risk;

        // 1. Spend limit interceptor
        let spend_limit = SpendLimitInterceptor::new(risk.max_trade_usd, risk.max_daily_usd);
        builder = builder.with_tool_interceptor(spend_limit);
        info!(
            max_trade = risk.max_trade_usd,
            max_daily = risk.max_daily_usd,
            "Added spend limit interceptor"
        );

        // 2. Slippage guard interceptor
        let slippage_guard = SlippageGuardInterceptor::new(risk.max_slippage_percent);
        builder = builder.with_tool_interceptor(slippage_guard);
        info!(
            max_slippage = risk.max_slippage_percent,
            "Added slippage guard interceptor"
        );

        // 3. Cooldown interceptor
        let cooldown = CooldownInterceptor::new(risk.cooldown_seconds);
        builder = builder.with_tool_interceptor(cooldown);
        info!(
            cooldown_seconds = risk.cooldown_seconds,
            "Added cooldown interceptor"
        );

        // 4. Audit log interceptor
        if let Some(audit_path) = &self.config.audit_log_path {
            let audit_log = AuditLogInterceptor::new(audit_path);
            builder = builder.with_tool_interceptor(audit_log);
            info!(audit_path = audit_path, "Added audit log interceptor");
        }

        // Build the runtime
        let runtime = builder
            .build()
            .await
            .map_err(|e| crate::Error::BamlRuntime(e.to_string()))?;

        info!("BAML runtime built successfully");
        Ok(runtime)
    }

    /// Register Rust tools with the QuickJS bridge and BAML manager
    async fn register_tools(
        &self,
        runtime: &Runtime,
        bridge: &Arc<Mutex<QuickJSBridge>>,
    ) -> Result<()> {
        // Get wallet address for Odos tool
        let wallet_address = self
            .wallet
            .as_ref()
            .map(|w| w.address_string())
            .unwrap_or_else(|| "0x0000000000000000000000000000000000000000".to_string());

        // Register the actual Rust tools with the BAML manager's tool registry
        // This allows __tool_invoke to dispatch to these tools
        {
            let baml_manager = runtime.baml_manager();
            let manager_guard = baml_manager.lock().await;
            let tool_registry = manager_guard.tool_registry();
            let mut registry_guard = tool_registry.lock().await;

            // Register The Graph tool
            let the_graph_tool = TheGraphTool::new();
            registry_guard.register(the_graph_tool).map_err(|e| {
                crate::Error::BamlRuntime(format!("Failed to register TheGraphTool: {}", e))
            })?;
            info!("Registered TheGraphTool with BAML manager");

            // Register Odos tool
            let odos_tool = OdosTool::try_new(&wallet_address).map_err(|e| {
                crate::Error::BamlRuntime(format!("Failed to create OdosTool: {}", e))
            })?;
            registry_guard.register(odos_tool).map_err(|e| {
                crate::Error::BamlRuntime(format!("Failed to register OdosTool: {}", e))
            })?;
            info!("Registered OdosTool with BAML manager");
        }

        // Note: JavaScript wrapper functions are NOT needed here because:
        // 1. Rust tools are registered with the BAML manager's tool registry above
        // 2. The global `invokeTool` function (registered by QuickJS bridge) automatically
        //    dispatches to Rust tools via `__tool_invoke` when a tool name doesn't exist
        //    as a JavaScript function
        // 3. The agent code calls invokeTool("query_subgraph", {...}) which correctly
        //    routes to the registered TheGraphTool
        _ = bridge; // Silence unused variable warning

        info!(
            wallet_address = wallet_address,
            "Registered tools with QuickJS and BAML manager"
        );

        Ok(())
    }

    /// Load and execute the agent's JavaScript code
    async fn load_agent_code(
        &self,
        bridge: &Arc<Mutex<QuickJSBridge>>,
        js_path: &Path,
    ) -> Result<()> {
        let code = std::fs::read_to_string(js_path)
            .map_err(|e| crate::Error::Config(format!("Failed to read agent code: {}", e)))?;

        info!(js_path = %js_path.display(), "Loading agent JavaScript code");

        let mut bridge_guard = bridge.lock().await;

        // For TypeScript files, we need to transpile or use a compatible version
        // For now, assume the code is JavaScript or has been transpiled
        let result = bridge_guard.evaluate(&code).await;

        match result {
            Ok(_) => {
                info!("Agent code loaded and initialized");
                Ok(())
            }
            Err(e) => {
                // Some initialization code doesn't return a value, which is OK
                warn!(error = %e, "Agent code evaluation warning (may be expected)");
                Ok(())
            }
        }
    }

    /// Start the trading loop
    async fn start_trading_loop(&self, bridge: &Arc<Mutex<QuickJSBridge>>) -> Result<()> {
        // Build trading config for JavaScript
        let trading_config = json!({
            "networks": self.config.networks.iter().map(|n| n.name()).collect::<Vec<_>>(),
            "protocols": self.config.protocols.iter().map(|p| p.name()).collect::<Vec<_>>(),
            "check_interval_ms": self.config.check_interval_ms,
            "risk": {
                "max_trade_usd": self.config.risk.max_trade_usd,
                "max_slippage_percent": self.config.risk.max_slippage_percent,
                "preferred_networks": self.config.networks.iter().map(|n| n.name()).collect::<Vec<_>>()
            }
        });

        if self.dry_run {
            info!("Dry run mode - showing config that would be used:");
            info!("{}", serde_json::to_string_pretty(&trading_config).unwrap());
            return Ok(());
        }

        info!("Starting trading loop with config:");
        info!("{}", serde_json::to_string_pretty(&trading_config).unwrap());

        let mut bridge_guard = bridge.lock().await;

        // Start the trading loop without waiting for it to complete.
        // runTradingLoop runs forever (infinite while loop), so we:
        // 1. Start the loop (it returns a promise immediately)
        // 2. Continuously drive the QuickJS event loop to allow async code to run
        let js_code = format!(
            r#"
            (function() {{
                const config = {};
                if (typeof runTradingLoop === 'function') {{
                    // Start the trading loop - don't await, let it run in background
                    runTradingLoop(config).catch(function(err) {{
                        console.error("Trading loop fatal error:", err);
                    }});
                    return JSON.stringify({{ status: "started" }});
                }} else {{
                    return JSON.stringify({{ error: "runTradingLoop not found" }});
                }}
            }})()
            "#,
            serde_json::to_string(&trading_config).unwrap()
        );

        // Start the trading loop
        let result = bridge_guard.evaluate(&js_code).await;

        match result {
            Ok(value) => {
                info!(result = %value, "Trading loop started");
            }
            Err(e) => {
                error!(error = %e, "Failed to start trading loop");
                return Err(crate::Error::BamlRuntime(format!(
                    "Failed to start trading loop: {}",
                    e
                )));
            }
        }

        // Keep the QuickJS event loop running to drive async operations
        // This loop will run forever, driving the trading agent
        info!("Entering main event loop to drive trading agent...");
        loop {
            // Drive the QuickJS event loop
            bridge_guard.drive_event_loop().await;

            // Small delay to prevent busy-waiting
            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        }
    }
}
