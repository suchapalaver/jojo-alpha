
// Host tool helper (agent-platform expects openToolSession for host tools).
// Note: This helper is duplicated across agent entrypoints; keep in sync.
async function invokeHostTool(toolName, args) {
  const token = globalThis.__baml_invocation_token;
  if (!token) {
    throw new Error("Missing invocation token");
  }
  const session = await globalThis.openToolSession(toolName, token);
  await session.send(args ?? {});
  let step = await session.continue();
  while (step && step.status === "streaming") {
    step = await session.continue();
  }
  await session.finish();
  if (step && step.status === "done") {
    return step.output;
  }
  if (step && step.status === "error") {
    throw new Error((step.error && step.error.message) || "Tool error");
  }
  return step;
}
const invokeTool = invokeHostTool;
 globalThis.invokeTool = invokeHostTool;

// Minimal A2A handler for policy explainability demo.
// Runs inside the QuickJS sandbox.

globalThis.handle_a2a_request = async function(request) {
  const ctx = request?.params?.message?.contextId || "ctx-missing";
  const text = request?.params?.message?.parts?.[0]?.text || "";

  let deny_result = null;
  if (text.toLowerCase().includes("deny")) {
    try {
      await globalThis.invokeTool("defi/odos_swap", {
        action: "quote",
        input_token: "0xEeeeeEeeeEeEeeEeEeEeeEEEeeeeEeeeeeeeEEeE",
        output_token: "0x0000000000000000000000000000000000000000",
        amount: "1"
      });
    } catch (err) {
      deny_result = { error: String(err) };
    }
  }
  const metrics = await globalThis.invokeTool("defi/paper_trading", {
    action: "get_metrics",
    error_class: "transient"
  });

  const artifacts = [
    {
      name: "paper_metrics",
      parts: [{ text: JSON.stringify(metrics) }]
    }
  ];

  if (deny_result) {
    artifacts.push({
      name: "policy_denied",
      parts: [{ text: JSON.stringify(deny_result) }]
    });
  }

  return {
    task: {
      id: "task-policy-demo",
      contextId: ctx,
      status: { state: "TASK_STATE_COMPLETED" },
      history: [],
      artifacts
    }
  };
};
