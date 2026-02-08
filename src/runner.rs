//! Agent runner module
//!
//! Loads and executes the trading agent in the QuickJS sandbox with
//! full tool and interceptor support.

use crate::config::{Config, PolicyDefaultMode, GRAPH_API_KEY_ENV};
use crate::interceptors::{
    AuditLogInterceptor, CooldownInterceptor, PolicyConfig, PolicyInterceptor, PolicyMode,
    SlippageGuardInterceptor, SpendLimitInterceptor,
};
use crate::paper_trading::PaperTradingState;
use crate::tools::{OdosTool, PaperTradingTool, TheGraphTool, WalletTool};
use crate::wallet::SecureWallet;
use crate::Result;
use baml_rt::quickjs_bridge::QuickJSBridge;
use baml_rt::{QuickJSConfig, Runtime, RuntimeBuilder};
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
    paper_trading: Option<PaperTradingState>,
}

fn quickjs_config_from_env() -> QuickJSConfig {
    let mut config = QuickJSConfig::new();

    if let Some(limit) = parse_u64_env("BAML_QJS_MEMORY_LIMIT_BYTES") {
        config = config.with_memory_limit(Some(limit));
        info!(
            memory_limit_bytes = limit,
            "Configured QuickJS memory limit"
        );
    }

    if let Some(size) = parse_u64_env("BAML_QJS_MAX_STACK_BYTES") {
        config = config.with_max_stack_size(Some(size));
        info!(max_stack_bytes = size, "Configured QuickJS max stack size");
    }

    if let Some(threshold) = parse_u64_env("BAML_QJS_GC_THRESHOLD") {
        config = config.with_gc_threshold(Some(threshold));
        info!(gc_threshold = threshold, "Configured QuickJS GC threshold");
    }

    if let Some(interval_secs) = parse_u64_env("BAML_QJS_GC_INTERVAL_SECS") {
        let interval = std::time::Duration::from_secs(interval_secs);
        config = config.with_gc_interval(Some(interval));
        info!(
            gc_interval_secs = interval_secs,
            "Configured QuickJS GC interval"
        );
    }

    config
}

fn parse_u64_env(name: &str) -> Option<u64> {
    match std::env::var(name) {
        Ok(value) => match value.parse::<u64>() {
            Ok(parsed) => Some(parsed),
            Err(err) => {
                warn!(env_var = name, error = %err, "Invalid QuickJS env override");
                None
            }
        },
        Err(_) => None,
    }
}

impl AgentRunner {
    /// Create a new agent runner
    pub fn new(config: Config, dry_run: bool) -> Self {
        Self {
            config,
            dry_run,
            wallet: None,
            paper_trading: None,
        }
    }

    /// Set the wallet for transaction signing
    pub fn with_wallet(mut self, wallet: SecureWallet) -> Self {
        self.wallet = Some(wallet);
        self
    }

    /// Enable paper trading mode
    pub fn with_paper_trading(mut self, state: PaperTradingState) -> Self {
        self.paper_trading = Some(state);
        self
    }

    /// Check if paper trading is enabled
    pub fn is_paper_trading(&self) -> bool {
        self.paper_trading
            .as_ref()
            .map(|s| s.is_enabled())
            .unwrap_or(false)
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
            .with_quickjs(true)
            .with_quickjs_config(quickjs_config_from_env());

        // Add environment variables for LLM providers
        if let Ok(api_key) = std::env::var("OPENROUTER_API_KEY") {
            builder = builder.with_env_var("OPENROUTER_API_KEY", api_key);
        }

        // Add tool interceptors for policy + risk management
        let agent_root = baml_src
            .parent()
            .ok_or_else(|| crate::Error::Config("Missing agent directory".to_string()))?;
        let policy_path = agent_root.join("policy.json");
        let policy_settings = &self.config.policy;
        let fallback_mode = match policy_settings.default_mode {
            PolicyDefaultMode::AllowAll => PolicyMode::AllowAll,
            PolicyDefaultMode::DefaultDeny => PolicyMode::DefaultDeny,
        };

        let policy = if !policy_path.exists() {
            if policy_settings.require_file {
                return Err(crate::Error::Config(format!(
                    "policy.json required but missing at {}",
                    policy_path.display()
                )));
            }
            warn!(
                policy_path = %policy_path.display(),
                default_mode = ?policy_settings.default_mode,
                "policy.json missing; falling back to default policy"
            );
            PolicyConfig::from_mode(fallback_mode)
        } else {
            PolicyConfig::load_from_dir(agent_root, fallback_mode)
                .await
                .unwrap_or_else(|err| {
                    warn!(error = %err, "Failed to load policy.json; defaulting to fallback mode");
                    PolicyConfig::from_mode(fallback_mode)
                })
        };

        let policy_interceptor = PolicyInterceptor::new(policy);
        builder = builder.with_tool_interceptor(policy_interceptor);
        info!("Added policy interceptor");

        // Add tool interceptors for risk management
        let risk = &self.config.risk;

        // 2. Spend limit interceptor (with configurable mode)
        let spend_limit = SpendLimitInterceptor::with_mode(
            risk.max_trade_usd,
            risk.max_daily_usd,
            risk.spend_limit_mode,
        );
        builder = builder.with_tool_interceptor(spend_limit);
        info!(
            max_trade = risk.max_trade_usd,
            max_daily = risk.max_daily_usd,
            mode = ?risk.spend_limit_mode,
            "Added spend limit interceptor"
        );

        // 3. Slippage guard interceptor
        let slippage_guard = SlippageGuardInterceptor::new(risk.max_slippage_percent);
        builder = builder.with_tool_interceptor(slippage_guard);
        info!(
            max_slippage = risk.max_slippage_percent,
            "Added slippage guard interceptor"
        );

        // 4. Cooldown interceptor
        let cooldown = CooldownInterceptor::new(risk.cooldown_seconds);
        builder = builder.with_tool_interceptor(cooldown);
        info!(
            cooldown_seconds = risk.cooldown_seconds,
            "Added cooldown interceptor"
        );

        // 5. Audit log interceptor
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
            // Use gateway-enabled version if GRAPH_API_KEY is set (enables caching)
            let the_graph_tool = match std::env::var(GRAPH_API_KEY_ENV) {
                Ok(api_key) => {
                    info!("Creating TheGraphTool with gateway caching enabled");
                    TheGraphTool::with_gateway(api_key)
                }
                Err(_) => {
                    warn!(
                        "GRAPH_API_KEY not set, TheGraphTool will use direct queries (no caching)"
                    );
                    TheGraphTool::new()
                }
            };
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

            // Register Wallet tool
            let wallet_tool = WalletTool::new(&wallet_address).map_err(|e| {
                crate::Error::BamlRuntime(format!("Failed to create WalletTool: {}", e))
            })?;
            registry_guard.register(wallet_tool).map_err(|e| {
                crate::Error::BamlRuntime(format!("Failed to register WalletTool: {}", e))
            })?;
            info!("Registered WalletTool with BAML manager");

            // Register Paper Trading tool if enabled
            if let Some(ref paper_state) = self.paper_trading {
                if paper_state.is_enabled() {
                    let paper_tool = PaperTradingTool::new(paper_state.clone());
                    registry_guard.register(paper_tool).map_err(|e| {
                        crate::Error::BamlRuntime(format!(
                            "Failed to register PaperTradingTool: {}",
                            e
                        ))
                    })?;
                    info!("Registered PaperTradingTool with BAML manager");
                }
            }
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
        let mut trading_config = json!({
            "networks": self.config.networks.iter().map(|n| n.name()).collect::<Vec<_>>(),
            "protocols": self.config.protocols.iter().map(|p| p.name()).collect::<Vec<_>>(),
            "check_interval_ms": self.config.check_interval_ms,
            "risk": {
                "max_trade_usd": self.config.risk.max_trade_usd,
                "max_slippage_percent": self.config.risk.max_slippage_percent,
                "preferred_networks": self.config.networks.iter().map(|n| n.name()).collect::<Vec<_>>()
            }
        });

        // Add paper trading config if enabled
        if let Some(ref paper_state) = self.paper_trading {
            if paper_state.is_enabled() {
                trading_config["paper_trading"] = json!({
                    "enabled": true
                });
                info!("Paper trading mode enabled in trading config");
            }
        }

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

        // Keep the process alive and explicitly poll the QuickJS event loop
        // so timers/promises progress even without additional evaluate() calls.
        info!("Agent running. Press Ctrl+C to stop.");
        loop {
            bridge_guard.poll_event_loop();
            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        }
    }
}
