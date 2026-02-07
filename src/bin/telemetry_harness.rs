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
use std::path::{Path, PathBuf};
use std::sync::Arc;
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

    let context_id = ContextId::from("ctx-telemetry-harness");
    let request_value = build_message_request(&args.message, context_id.clone());
    let responses = agent.handle_a2a(request_value).await?;
    let responses_json = serde_json::to_string(&responses)?;
    tracing::info!(responses = %responses_json, "A2A response");

    let events = assert_provenance_events(&memory_store, &context_id).await?;
    write_snapshot(&args.snapshot_out, &events).await?;

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
            const metrics = await invokeTool("paper_trading", { action: "get_metrics" });
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

fn build_message_request(message: &str, context_id: ContextId) -> serde_json::Value {
    let params = SendMessageRequest {
        message: Message {
            message_id: MessageId::from("msg-telemetry"),
            role: MessageRole::String(ROLE_USER.to_string()),
            parts: vec![Part {
                text: Some(message.to_string()),
                ..Part::default()
            }],
            context_id: Some(context_id),
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
            let redacted = json!({
                "redacted": true,
                "hash": hash_json(&args),
            });
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
            let redacted = json!({
                "redacted": true,
                "hash": hash_json(&prompt),
            });
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

#[derive(Debug, Serialize, Deserialize)]
struct TelemetrySnapshot {
    context_id: String,
    tool_calls: Vec<ToolTelemetry>,
    totals: TelemetryTotals,
}

#[derive(Debug, Serialize, Deserialize)]
struct ToolTelemetry {
    tool: String,
    calls: u64,
    successes: u64,
    failures: u64,
    avg_duration_ms: Option<f64>,
}

#[derive(Debug, Serialize, Deserialize)]
struct TelemetryTotals {
    tool_calls: u64,
    tool_successes: u64,
    tool_failures: u64,
}

async fn write_snapshot(
    path: &Path,
    events: &[baml_rt_provenance::ProvEvent],
) -> Result<(), Box<dyn std::error::Error>> {
    use baml_rt_provenance::ProvEventData;
    use std::collections::HashMap;

    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            create_dir_all(parent).await?;
        }
    }

    let mut stats: HashMap<String, Vec<u64>> = HashMap::new();
    let mut counts: HashMap<String, (u64, u64)> = HashMap::new();

    for event in events {
        if let ProvEventData::ToolCall {
            tool_name,
            duration_ms,
            success,
            ..
        } = &event.data
        {
            let entry = counts.entry(tool_name.clone()).or_insert((0, 0));
            if let Some(true) = success {
                entry.0 += 1;
            } else if let Some(false) = success {
                entry.1 += 1;
            }
            if let Some(duration) = duration_ms {
                stats.entry(tool_name.clone()).or_default().push(*duration);
            }
        }
    }

    let mut tool_calls = Vec::new();
    let mut totals = TelemetryTotals {
        tool_calls: 0,
        tool_successes: 0,
        tool_failures: 0,
    };

    for (tool, (successes, failures)) in counts {
        let durations = stats.get(&tool).cloned().unwrap_or_default();
        let avg = if durations.is_empty() {
            None
        } else {
            let sum: u64 = durations.iter().sum();
            Some(sum as f64 / durations.len() as f64)
        };
        let calls = successes + failures;
        totals.tool_calls += calls;
        totals.tool_successes += successes;
        totals.tool_failures += failures;

        tool_calls.push(ToolTelemetry {
            tool,
            calls,
            successes,
            failures,
            avg_duration_ms: avg,
        });
    }

    let context_id = events
        .first()
        .map(|event| event.context_id.to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let snapshot = TelemetrySnapshot {
        context_id,
        tool_calls,
        totals,
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
