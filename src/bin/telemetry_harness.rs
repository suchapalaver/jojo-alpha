//! A2A + provenance telemetry harness for the DeFi agent.
//!
//! Builds a QuickJS runtime, registers tools, wires A2A handling, and emits
//! provenance events to a JSONL file while asserting expected event types.

use baml_rt::tracing_setup;
use baml_rt::{A2aRequestHandler, QuickJSConfig, RuntimeBuilder};
use baml_rt_a2a::a2a_types::{
    JSONRPCId, JSONRPCRequest, Message, MessageRole, Part, SendMessageRequest, ROLE_USER,
};
use baml_rt_a2a::A2aAgent;
use baml_rt_core::ids::{ContextId, MessageId};
use baml_rt_provenance::{InMemoryProvenanceStore, ProvEventType, ProvenanceWriter};
use clap::Parser;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::fs::{create_dir_all, OpenOptions};
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;

use defi_trading_agent::paper_trading::{PaperModeConfig, PaperTradingState};
use defi_trading_agent::tools::PaperTradingTool;

#[derive(Parser, Debug)]
#[command(name = "telemetry-harness")]
#[command(about = "A2A + provenance telemetry harness for the agent")]
struct HarnessArgs {
    /// Path to the agent directory (expects baml_src/ and dist/index.js)
    #[arg(long, default_value = "./agent")]
    agent: PathBuf,

    /// JSONL output file for provenance events
    #[arg(long, default_value = "./telemetry/provenance.jsonl")]
    provenance_out: PathBuf,

    /// JSON output file for telemetry snapshot
    #[arg(long, default_value = "./telemetry/snapshot.json")]
    snapshot_out: PathBuf,

    /// Message text for the A2A request
    #[arg(long, default_value = "telemetry harness ping")]
    message: String,
}

#[derive(Debug, Clone, Deserialize)]
struct PolicyFile {
    mode: String,
    rules: Vec<PolicyRule>,
}

#[derive(Debug, Clone, Deserialize)]
struct PolicyRule {
    tool: String,
    allowed: bool,
    rule_id: Option<String>,
    reason: Option<String>,
}

#[derive(Debug, Clone)]
struct HarnessContextId(String);

impl HarnessContextId {
    fn new(value: &str) -> Option<Self> {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(Self(trimmed.to_string()))
        }
    }

    fn as_context_id(&self) -> ContextId {
        ContextId::from(self.0.clone())
    }
}

#[derive(Debug, Clone)]
struct HarnessMessageId(String);

impl HarnessMessageId {
    fn new(value: &str) -> Option<Self> {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(Self(trimmed.to_string()))
        }
    }

    fn as_message_id(&self) -> MessageId {
        MessageId::from(self.0.clone())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq, Hash)]
#[serde(transparent)]
struct ToolName(String);

impl ToolName {
    fn new(value: &str) -> Option<Self> {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else if is_valid_tool_name(trimmed) {
            Some(Self(trimmed.to_string()))
        } else {
            None
        }
    }

    #[cfg(test)]
    fn from_literal(value: &'static str) -> Self {
        debug_assert!(is_valid_tool_name(value), "invalid tool name literal");
        Self(value.to_string())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
enum SnapshotVersion {
    V1,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
struct SnapshotSchemaHash(String);

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq, Hash)]
#[serde(rename_all = "snake_case")]
enum ErrorClass {
    Transient,
    Permanent,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Redacted {
    redacted: bool,
    hash: String,
}

impl Redacted {
    fn from_json(value: &serde_json::Value) -> Self {
        Self {
            redacted: true,
            hash: hash_json(value),
        }
    }

    fn to_value(&self) -> serde_json::Value {
        json!({
            "redacted": self.redacted,
            "hash": self.hash,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct NonEmptyVec<T> {
    head: T,
    tail: Vec<T>,
}

impl<T> NonEmptyVec<T> {
    fn try_from_vec(mut items: Vec<T>) -> Option<Self> {
        if items.is_empty() {
            None
        } else {
            let head = items.remove(0);
            Some(Self { head, tail: items })
        }
    }
}

const fn is_valid_tool_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    let mut idx = 0;
    while idx < bytes.len() {
        let ch = bytes[idx];
        let ok = (ch >= b'a' && ch <= b'z') || (ch >= b'0' && ch <= b'9') || ch == b'_';
        if !ok {
            return false;
        }
        idx += 1;
    }
    true
}

const PAPER_TRADING_TOOL: &str = "paper_trading";
const QUERY_SUBGRAPH_TOOL: &str = "query_subgraph";
const ODOS_SWAP_TOOL: &str = "odos_swap";
const WALLET_BALANCE_TOOL: &str = "wallet_balance";
const _: () = {
    if !is_valid_tool_name(PAPER_TRADING_TOOL)
        || !is_valid_tool_name(QUERY_SUBGRAPH_TOOL)
        || !is_valid_tool_name(ODOS_SWAP_TOOL)
        || !is_valid_tool_name(WALLET_BALANCE_TOOL)
    {
        panic!("invalid tool name literal");
    }
};

struct JsonlProvenanceWriter {
    file: Mutex<tokio::fs::File>,
}

impl JsonlProvenanceWriter {
    async fn new(path: &Path) -> Result<Self, baml_rt_provenance::ProvenanceError> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                create_dir_all(parent)
                    .await
                    .map_err(|e| baml_rt_provenance::ProvenanceError::Storage(e.to_string()))?;
            }
        }
        let file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(path)
            .await
            .map_err(|e| baml_rt_provenance::ProvenanceError::Storage(e.to_string()))?;
        Ok(Self {
            file: Mutex::new(file),
        })
    }
}

#[async_trait::async_trait]
impl ProvenanceWriter for JsonlProvenanceWriter {
    async fn add_event(
        &self,
        event: baml_rt_provenance::ProvEvent,
    ) -> Result<(), baml_rt_provenance::ProvenanceError> {
        let sanitized = sanitize_event(event);
        let line = serde_json::to_string(&sanitized)
            .map_err(|e| baml_rt_provenance::ProvenanceError::Storage(e.to_string()))?;
        let mut file = self.file.lock().await;
        file.write_all(line.as_bytes())
            .await
            .map_err(|e| baml_rt_provenance::ProvenanceError::Storage(e.to_string()))?;
        file.write_all(b"\n")
            .await
            .map_err(|e| baml_rt_provenance::ProvenanceError::Storage(e.to_string()))?;
        Ok(())
    }
}

struct FanoutProvenanceWriter {
    writers: Vec<Arc<dyn ProvenanceWriter>>,
}

#[async_trait::async_trait]
impl ProvenanceWriter for FanoutProvenanceWriter {
    async fn add_event(
        &self,
        event: baml_rt_provenance::ProvEvent,
    ) -> Result<(), baml_rt_provenance::ProvenanceError> {
        for writer in &self.writers {
            writer.add_event(event.clone()).await?;
        }
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_setup::init_tracing();

    let args = HarnessArgs::parse();
    let (baml_src, js_path) = resolve_agent_paths(&args.agent)?;
    let policy = load_policy(&args.agent).await.unwrap_or_else(|err| {
        tracing::warn!(error = %err, "Failed to load policy.json; falling back to default policy");
        PolicyConfig::default()
    });

    let runtime = RuntimeBuilder::new()
        .with_schema_path(baml_src)
        .with_quickjs(true)
        .with_quickjs_config(quickjs_config_from_env())
        .build()
        .await?;

    let bridge = runtime
        .quickjs_bridge()
        .ok_or("QuickJS bridge not available")?;

    register_tools(&runtime).await?;
    load_agent_code(&bridge, &js_path).await?;
    register_a2a_handler(&bridge).await?;

    let memory_store = Arc::new(InMemoryProvenanceStore::new());
    let file_writer = Arc::new(JsonlProvenanceWriter::new(&args.provenance_out).await?);
    let writer: Arc<dyn ProvenanceWriter> = Arc::new(FanoutProvenanceWriter {
        writers: vec![
            memory_store.clone() as Arc<dyn ProvenanceWriter>,
            file_writer as Arc<dyn ProvenanceWriter>,
        ],
    });

    let agent = A2aAgent::builder()
        .with_runtime_handle(runtime.baml_manager())
        .with_bridge_handle(bridge.clone())
        .with_provenance_writer(writer)
        .build()
        .await?;

    let context_id =
        HarnessContextId::new("ctx-telemetry-harness").ok_or("Invalid harness context id")?;
    let request_value = build_message_request(&args.message, &context_id);
    let responses = agent.handle_a2a(request_value).await?;
    let responses_json = serde_json::to_string(&responses)?;
    tracing::info!(responses = %responses_json, "A2A response");

    let events = assert_provenance_events(&memory_store, &context_id.as_context_id()).await?;
    let cost_model = CostModel::from_env();
    write_snapshot(&args.snapshot_out, &events, &policy, &cost_model).await?;

    tracing::info!(
        provenance_out = %args.provenance_out.display(),
        snapshot_out = %args.snapshot_out.display(),
        "Telemetry harness completed"
    );

    Ok(())
}

fn resolve_agent_paths(agent_dir: &Path) -> Result<(String, PathBuf), Box<dyn std::error::Error>> {
    let baml_src = agent_dir.join("baml_src");
    let js_path = agent_dir.join("dist").join("index.js");
    if !baml_src.exists() {
        return Err(format!("Missing baml_src at {}", baml_src.display()).into());
    }
    if !js_path.exists() {
        return Err(format!("Missing agent JS at {}", js_path.display()).into());
    }
    Ok((baml_src.to_string_lossy().to_string(), js_path))
}

async fn register_tools(runtime: &baml_rt::Runtime) -> baml_rt::Result<()> {
    let paper_config = PaperModeConfig {
        enabled: true,
        initial_balance_usd: 10_000.0,
        state_file: None,
    };
    let paper_state = PaperTradingState::new(&paper_config);

    let manager = runtime.baml_manager();
    let manager_guard = manager.lock().await;
    let registry = manager_guard.tool_registry();
    let mut registry_guard = registry.lock().await;

    registry_guard.register(PaperTradingTool::new(paper_state))?;

    Ok(())
}

async fn load_agent_code(
    bridge: &Arc<Mutex<baml_rt::QuickJSBridge>>,
    js_path: &Path,
) -> baml_rt::Result<()> {
    let code = std::fs::read_to_string(js_path)
        .map_err(|e| baml_rt::BamlRtError::InvalidArgument(e.to_string()))?;
    let mut bridge_guard = bridge.lock().await;
    let _ = bridge_guard.evaluate(&code).await?;
    Ok(())
}

async fn register_a2a_handler(bridge: &Arc<Mutex<baml_rt::QuickJSBridge>>) -> baml_rt::Result<()> {
    let js_code = r#"
        globalThis.handle_a2a_request = async function(request) {
            const ctx = request?.params?.message?.contextId || "ctx-missing";
            const metrics = await invokeTool("paper_trading", { action: "get_metrics", error_class: "transient" });
            return {
                task: {
                    id: "task-telemetry",
                    contextId: ctx,
                    status: { state: "TASK_STATE_COMPLETED" },
                    history: [],
                    artifacts: [
                        {
                            name: "paper_metrics",
                            parts: [{ text: JSON.stringify(metrics) }]
                        }
                    ]
                }
            };
        };
    "#;
    let mut bridge_guard = bridge.lock().await;
    let _ = bridge_guard.evaluate(js_code).await?;
    Ok(())
}

fn build_message_request(message: &str, context_id: &HarnessContextId) -> serde_json::Value {
    let message_id = HarnessMessageId::new("msg-telemetry").expect("message id must be non-empty");
    let params = SendMessageRequest {
        message: Message {
            message_id: message_id.as_message_id(),
            role: MessageRole::String(ROLE_USER.to_string()),
            parts: vec![Part {
                text: Some(message.to_string()),
                ..Part::default()
            }],
            context_id: Some(context_id.as_context_id()),
            task_id: None,
            reference_task_ids: Vec::new(),
            extensions: Vec::new(),
            metadata: None,
            extra: Default::default(),
        },
        configuration: None,
        metadata: None,
        tenant: None,
        extra: Default::default(),
    };

    let request = JSONRPCRequest {
        jsonrpc: "2.0".to_string(),
        method: "message.send".to_string(),
        params: Some(serde_json::to_value(params).expect("serialize params")),
        id: Some(JSONRPCId::String("req-telemetry".to_string())),
    };

    serde_json::to_value(request).expect("serialize request")
}

async fn assert_provenance_events(
    store: &InMemoryProvenanceStore,
    context_id: &ContextId,
) -> Result<Vec<baml_rt_provenance::ProvEvent>, Box<dyn std::error::Error>> {
    let events = store.events().await;
    let has_context = events.iter().any(|event| &event.context_id == context_id);
    if !has_context {
        return Err("No provenance events for harness context_id".into());
    }

    let has_tool_started = events
        .iter()
        .any(|event| event.event_type == ProvEventType::ToolCallStarted);
    let has_tool_completed = events
        .iter()
        .any(|event| event.event_type == ProvEventType::ToolCallCompleted);

    if !has_tool_started || !has_tool_completed {
        return Err("Expected tool call provenance events were not recorded".into());
    }

    Ok(events)
}

fn quickjs_config_from_env() -> QuickJSConfig {
    let mut config = QuickJSConfig::new();

    if let Some(limit) = parse_u64_env("BAML_QJS_MEMORY_LIMIT_BYTES") {
        config = config.with_memory_limit(Some(limit));
    }

    if let Some(size) = parse_u64_env("BAML_QJS_MAX_STACK_BYTES") {
        config = config.with_max_stack_size(Some(size));
    }

    if let Some(threshold) = parse_u64_env("BAML_QJS_GC_THRESHOLD") {
        config = config.with_gc_threshold(Some(threshold));
    }

    if let Some(interval_secs) = parse_u64_env("BAML_QJS_GC_INTERVAL_SECS") {
        config = config.with_gc_interval(Some(std::time::Duration::from_secs(interval_secs)));
    }

    config
}

fn parse_u64_env(name: &str) -> Option<u64> {
    match std::env::var(name) {
        Ok(value) => value.parse::<u64>().ok(),
        Err(_) => None,
    }
}

fn sanitize_event(event: baml_rt_provenance::ProvEvent) -> baml_rt_provenance::ProvEvent {
    use baml_rt_provenance::ProvEventData;

    match event.data {
        ProvEventData::ToolCall {
            tool_name,
            function_name,
            args,
            metadata,
            duration_ms,
            success,
        } => {
            let redacted = Redacted::from_json(&args).to_value();
            let metadata = enrich_metadata_with_error_class(metadata, &args);
            baml_rt_provenance::ProvEvent {
                data: ProvEventData::ToolCall {
                    tool_name,
                    function_name,
                    args: redacted,
                    metadata,
                    duration_ms,
                    success,
                },
                ..event
            }
        }
        ProvEventData::LlmCall {
            client,
            model,
            function_name,
            prompt,
            metadata,
            duration_ms,
            success,
        } => {
            let redacted = Redacted::from_json(&prompt).to_value();
            let metadata = enrich_metadata_with_error_class(metadata, &prompt);
            baml_rt_provenance::ProvEvent {
                data: ProvEventData::LlmCall {
                    client,
                    model,
                    function_name,
                    prompt: redacted,
                    metadata,
                    duration_ms,
                    success,
                },
                ..event
            }
        }
        _ => event,
    }
}

fn hash_json(value: &serde_json::Value) -> String {
    let bytes = serde_json::to_vec(value).unwrap_or_default();
    blake3::hash(&bytes).to_hex().to_string()
}

fn enrich_metadata_with_error_class(
    mut metadata: serde_json::Value,
    args: &serde_json::Value,
) -> serde_json::Value {
    if metadata.get("error_class").is_some() {
        return metadata;
    }
    let Some(class) = args.get("error_class").and_then(|v| v.as_str()) else {
        return metadata;
    };
    match metadata.as_object_mut() {
        Some(map) => {
            map.insert("error_class".to_string(), json!(class));
        }
        None => {
            let mut map = serde_json::Map::new();
            map.insert("error_class".to_string(), json!(class));
            metadata = serde_json::Value::Object(map);
        }
    }
    metadata
}

#[derive(Debug, Serialize, Deserialize)]
struct TelemetrySnapshot {
    snapshot_version: SnapshotVersion,
    schema_hash: SnapshotSchemaHash,
    context_id: String,
    generated_at_ms: u64,
    window_ms: u64,
    tool_calls: NonEmptyVec<ToolTelemetry>,
    totals: TelemetryTotals,
    policy: PolicySummary,
    costs: CostSummary,
}

#[derive(Debug, Serialize, Deserialize)]
struct ToolTelemetry {
    tool: ToolName,
    calls: u64,
    successes: u64,
    failures: u64,
    avg_duration_ms: Option<f64>,
    error_classes: Vec<ErrorClassCount>,
    success_rate: f64,
    policy: PolicyDecision,
    costs: CostHint,
}

#[derive(Debug, Serialize, Deserialize)]
struct TelemetryTotals {
    tool_calls: u64,
    tool_successes: u64,
    tool_failures: u64,
    avg_duration_ms: Option<f64>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ErrorClassCount {
    class: ErrorClass,
    count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PolicyDecision {
    allowed: bool,
    rule_id: Option<String>,
    reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PolicyRuleSummary {
    tool: ToolName,
    allowed: bool,
    rule_id: Option<String>,
    reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PolicyViolation {
    tool: ToolName,
    calls: u64,
    rule_id: Option<String>,
    reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PolicySummary {
    mode: String,
    rules: Vec<PolicyRuleSummary>,
    decisions: Vec<PolicyDecision>,
    violations: Vec<PolicyViolation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CostHint {
    estimated_usd: f64,
    tokens: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CostSummary {
    total_estimated_usd: f64,
    total_tokens: u64,
}

#[derive(Debug, Clone)]
struct CostModel {
    odos_usd_per_call: f64,
    graph_usd_per_call: f64,
    wallet_usd_per_call: f64,
    paper_usd_per_call: f64,
    default_usd_per_call: f64,
}

impl Default for CostModel {
    fn default() -> Self {
        Self {
            odos_usd_per_call: 0.001,
            graph_usd_per_call: 0.0005,
            wallet_usd_per_call: 0.0002,
            paper_usd_per_call: 0.0,
            default_usd_per_call: 0.0001,
        }
    }
}

impl CostModel {
    fn from_env() -> Self {
        let mut model = Self::default();
        if let Some(value) = parse_env_f64("TELEMETRY_COST_ODOS_USD") {
            model.odos_usd_per_call = value;
        }
        if let Some(value) = parse_env_f64("TELEMETRY_COST_GRAPH_USD") {
            model.graph_usd_per_call = value;
        }
        if let Some(value) = parse_env_f64("TELEMETRY_COST_WALLET_USD") {
            model.wallet_usd_per_call = value;
        }
        if let Some(value) = parse_env_f64("TELEMETRY_COST_PAPER_USD") {
            model.paper_usd_per_call = value;
        }
        if let Some(value) = parse_env_f64("TELEMETRY_COST_DEFAULT_USD") {
            model.default_usd_per_call = value;
        }
        model
    }

    fn estimate_cost(&self, tool: &ToolName, calls: u64) -> CostHint {
        let per_call = match tool.0.as_str() {
            PAPER_TRADING_TOOL => self.paper_usd_per_call,
            QUERY_SUBGRAPH_TOOL => self.graph_usd_per_call,
            ODOS_SWAP_TOOL => self.odos_usd_per_call,
            WALLET_BALANCE_TOOL => self.wallet_usd_per_call,
            _ => self.default_usd_per_call,
        };
        CostHint {
            estimated_usd: per_call * calls as f64,
            tokens: 0,
        }
    }
}

#[derive(Debug, Clone)]
struct PolicyConfig {
    mode: String,
    rules: HashMap<ToolName, PolicyDecision>,
}

impl Default for PolicyConfig {
    fn default() -> Self {
        let mut rules = HashMap::new();
        let Some(tool) = ToolName::new(PAPER_TRADING_TOOL) else {
            return Self {
                mode: "default-deny".to_string(),
                rules,
            };
        };
        rules.insert(
            tool,
            PolicyDecision {
                allowed: true,
                rule_id: Some(format!("allow:{}", PAPER_TRADING_TOOL)),
                reason: "default harness allow".to_string(),
            },
        );
        Self {
            mode: "default-deny".to_string(),
            rules,
        }
    }
}

impl PolicyConfig {
    fn decision_for_tool(&self, tool: &ToolName) -> PolicyDecision {
        self.rules.get(tool).cloned().unwrap_or(PolicyDecision {
            allowed: false,
            rule_id: None,
            reason: "denied by default policy".to_string(),
        })
    }

    fn rule_summaries(&self) -> Vec<PolicyRuleSummary> {
        let mut rules = self
            .rules
            .iter()
            .map(|(tool, decision)| PolicyRuleSummary {
                tool: tool.clone(),
                allowed: decision.allowed,
                rule_id: decision.rule_id.clone(),
                reason: decision.reason.clone(),
            })
            .collect::<Vec<_>>();
        rules.sort_by(|a, b| a.tool.0.cmp(&b.tool.0));
        rules
    }
}

fn parse_env_f64(name: &str) -> Option<f64> {
    match std::env::var(name) {
        Ok(value) => match value.parse::<f64>() {
            Ok(parsed) => {
                if parsed < 0.0 {
                    tracing::warn!(
                        env_var = name,
                        value = parsed,
                        "Negative cost override ignored"
                    );
                    None
                } else {
                    Some(parsed)
                }
            }
            Err(err) => {
                tracing::warn!(env_var = name, error = %err, "Invalid cost env override");
                None
            }
        },
        Err(_) => None,
    }
}

async fn write_snapshot(
    path: &Path,
    events: &[baml_rt_provenance::ProvEvent],
    policy: &PolicyConfig,
    cost_model: &CostModel,
) -> Result<(), Box<dyn std::error::Error>> {
    use baml_rt_provenance::ProvEventData;

    let generated_at_ms = now_millis();
    let (min_ts, max_ts) = events.iter().fold((u64::MAX, 0u64), |acc, event| {
        (acc.0.min(event.timestamp_ms), acc.1.max(event.timestamp_ms))
    });
    let window_ms = if min_ts == u64::MAX {
        0
    } else {
        max_ts.saturating_sub(min_ts)
    };
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            create_dir_all(parent).await?;
        }
    }

    let mut stats: HashMap<ToolName, Vec<u64>> = HashMap::new();
    let mut counts: HashMap<ToolName, (u64, u64)> = HashMap::new();
    let mut error_classes: HashMap<ToolName, HashMap<ErrorClass, u64>> = HashMap::new();

    for event in events {
        if let ProvEventData::ToolCall {
            tool_name,
            duration_ms,
            success,
            metadata,
            ..
        } = &event.data
        {
            let Some(tool) = ToolName::new(tool_name) else {
                continue;
            };
            let entry = counts.entry(tool.clone()).or_insert((0, 0));
            if let Some(true) = success {
                entry.0 += 1;
            } else if let Some(false) = success {
                entry.1 += 1;
                let class_counts = error_classes.entry(tool.clone()).or_default();
                let class = classify_error(metadata);
                *class_counts.entry(class).or_insert(0) += 1;
            }
            if let Some(duration) = duration_ms {
                stats.entry(tool).or_default().push(*duration);
            }
        }
    }

    let mut tool_calls_vec = Vec::new();
    let mut totals = TelemetryTotals {
        tool_calls: 0,
        tool_successes: 0,
        tool_failures: 0,
        avg_duration_ms: None,
    };
    let mut total_duration: u64 = 0;
    let mut total_duration_count: u64 = 0;
    let mut policy_decisions = Vec::new();
    let policy_rules = policy.rule_summaries();
    let mut policy_violations = Vec::new();
    let mut cost_summary = CostSummary {
        total_estimated_usd: 0.0,
        total_tokens: 0,
    };

    for (tool, (successes, failures)) in counts {
        let durations = stats.get(&tool).cloned().unwrap_or_default();
        let avg = if durations.is_empty() {
            None
        } else {
            let sum: u64 = durations.iter().sum();
            total_duration += sum;
            total_duration_count += durations.len() as u64;
            Some(sum as f64 / durations.len() as f64)
        };
        let calls = successes + failures;
        totals.tool_calls += calls;
        totals.tool_successes += successes;
        totals.tool_failures += failures;

        let classes = error_classes
            .get(&tool)
            .map(|map| {
                map.iter()
                    .map(|(class, count)| ErrorClassCount {
                        class: class.clone(),
                        count: *count,
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let policy = policy.decision_for_tool(&tool);
        policy_decisions.push(policy.clone());
        if !policy.allowed && calls > 0 {
            policy_violations.push(PolicyViolation {
                tool: tool.clone(),
                calls,
                rule_id: policy.rule_id.clone(),
                reason: policy.reason.clone(),
            });
        }

        let costs = cost_model.estimate_cost(&tool, calls);
        cost_summary.total_estimated_usd += costs.estimated_usd;
        cost_summary.total_tokens += costs.tokens;

        tool_calls_vec.push(ToolTelemetry {
            tool: tool.clone(),
            calls,
            successes,
            failures,
            avg_duration_ms: avg,
            error_classes: classes,
            success_rate: if calls == 0 {
                0.0
            } else {
                successes as f64 / calls as f64
            },
            policy,
            costs,
        });
    }

    let tool_calls =
        NonEmptyVec::try_from_vec(tool_calls_vec).ok_or("No tool call telemetry available")?;
    let context_id = events
        .first()
        .map(|event| event.context_id.to_string())
        .unwrap_or_else(|| "unknown".to_string());

    totals.avg_duration_ms = if total_duration_count == 0 {
        None
    } else {
        Some(total_duration as f64 / total_duration_count as f64)
    };

    let snapshot = TelemetrySnapshot {
        snapshot_version: SnapshotVersion::V1,
        schema_hash: SnapshotSchemaHash(snapshot_schema_hash()),
        context_id,
        generated_at_ms,
        window_ms,
        tool_calls,
        totals,
        policy: PolicySummary {
            mode: policy.mode.clone(),
            rules: policy_rules,
            decisions: policy_decisions,
            violations: policy_violations,
        },
        costs: cost_summary,
    };

    let contents = serde_json::to_vec_pretty(&snapshot)?;
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(path)
        .await?;
    file.write_all(&contents).await?;
    file.write_all(b"\n").await?;
    Ok(())
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn snapshot_schema_hash() -> String {
    const SCHEMA: &str = "TelemetrySnapshot(v1):context_id,generated_at_ms,window_ms,tool_calls[tool,calls,successes,failures,avg_duration_ms,error_classes[class,count],success_rate,policy[allowed,rule_id,reason],costs[estimated_usd,tokens]],totals[tool_calls,tool_successes,tool_failures,avg_duration_ms],policy[mode,rules[tool,allowed,rule_id,reason],decisions[allowed,rule_id,reason],violations[tool,calls,rule_id,reason]],costs[total_estimated_usd,total_tokens]";
    blake3::hash(SCHEMA.as_bytes()).to_hex().to_string()
}

fn classify_error(metadata: &serde_json::Value) -> ErrorClass {
    if let Some(class) = metadata.get("error_class").and_then(|v| v.as_str()) {
        return match class {
            "transient" => ErrorClass::Transient,
            "permanent" => ErrorClass::Permanent,
            _ => ErrorClass::Unknown,
        };
    }

    let hint = metadata
        .get("error")
        .or_else(|| metadata.get("error_message"))
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    if hint.contains("timeout")
        || hint.contains("rate")
        || hint.contains("temporar")
        || hint.contains("retry")
    {
        ErrorClass::Transient
    } else if hint.contains("invalid")
        || hint.contains("unauthorized")
        || hint.contains("forbidden")
        || hint.contains("not found")
    {
        ErrorClass::Permanent
    } else {
        ErrorClass::Unknown
    }
}

async fn load_policy(agent_dir: &Path) -> Result<PolicyConfig, Box<dyn std::error::Error>> {
    let policy_path = agent_dir.join("policy.json");
    if !policy_path.exists() {
        return Ok(PolicyConfig::default());
    }
    let contents = tokio::fs::read_to_string(&policy_path).await?;
    let parsed: PolicyFile = serde_json::from_str(&contents)?;

    let mut rules = HashMap::new();
    for rule in parsed.rules {
        let Some(tool) = ToolName::new(&rule.tool) else {
            tracing::warn!(
                tool = %rule.tool,
                "Invalid tool name in policy.json; skipping rule"
            );
            continue;
        };
        rules.insert(
            tool,
            PolicyDecision {
                allowed: rule.allowed,
                rule_id: rule.rule_id.clone(),
                reason: rule
                    .reason
                    .clone()
                    .unwrap_or_else(|| "policy rule".to_string()),
            },
        );
    }

    Ok(PolicyConfig {
        mode: parsed.mode,
        rules,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use baml_rt_provenance::ProvEvent;
    use tempfile::tempdir;
    use tokio::fs;

    #[test]
    fn sanitize_event_redacts_tool_args() {
        let ctx = ContextId::from("ctx-test");
        let event = ProvEvent::tool_call_started(
            ctx,
            None,
            "paper_trading".to_string(),
            None,
            json!({"secret":"value","action":"get_metrics"}),
            json!({}),
        );

        let sanitized = sanitize_event(event);
        match sanitized.data {
            baml_rt_provenance::ProvEventData::ToolCall { args, .. } => {
                assert_eq!(args.get("redacted").and_then(|v| v.as_bool()), Some(true));
                assert!(args.get("hash").and_then(|v| v.as_str()).is_some());
            }
            _ => panic!("expected tool call event"),
        }
    }

    #[test]
    fn tool_name_validation() {
        assert!(ToolName::new("paper_trading").is_some());
        assert!(ToolName::new("odos-swap").is_none());
        assert!(ToolName::new("Tool").is_none());
        assert!(ToolName::new(" ").is_none());
    }

    #[test]
    fn classify_error_from_metadata() {
        let transient = json!({"error": "timeout while calling rpc"});
        let permanent = json!({"error_message": "invalid input"});
        let explicit = json!({"error_class": "permanent"});

        assert_eq!(classify_error(&transient), ErrorClass::Transient);
        assert_eq!(classify_error(&permanent), ErrorClass::Permanent);
        assert_eq!(classify_error(&explicit), ErrorClass::Permanent);
    }

    #[tokio::test]
    async fn snapshot_is_versioned_and_non_empty() {
        let ctx = ContextId::from("ctx-snapshot");
        let event = ProvEvent::tool_call_completed(
            ctx,
            None,
            ToolName::from_literal(PAPER_TRADING_TOOL).0,
            None,
            json!({ "action": "get_metrics" }),
            json!({}),
            3,
            true,
        );
        let events = vec![event];
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("snapshot.json");

        let policy = PolicyConfig::default();
        let cost_model = CostModel::default();
        write_snapshot(&path, &events, &policy, &cost_model)
            .await
            .expect("write snapshot");
        let contents = fs::read_to_string(&path).await.expect("read snapshot");
        let snapshot: TelemetrySnapshot = serde_json::from_str(&contents).expect("parse snapshot");

        assert_eq!(snapshot.snapshot_version, SnapshotVersion::V1);
        assert!(!snapshot.schema_hash.0.is_empty());
        let tool_calls = snapshot.tool_calls;
        assert_eq!(tool_calls.head.tool.0, "paper_trading");
        assert_eq!(tool_calls.head.calls, 1);
    }

    #[test]
    fn harness_ids_validate_non_empty() {
        assert!(HarnessContextId::new("ctx").is_some());
        assert!(HarnessContextId::new(" ").is_none());
        assert!(HarnessMessageId::new("msg").is_some());
        assert!(HarnessMessageId::new("").is_none());
    }

    #[test]
    fn tool_name_literal_validates() {
        let tool = ToolName::from_literal(PAPER_TRADING_TOOL);
        assert_eq!(tool.0, PAPER_TRADING_TOOL);
    }
}
