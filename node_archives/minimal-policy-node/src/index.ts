
// Host tool helper (agent-platform expects openToolSession for host tools)
async function invokeHostTool(toolName: string, args: any) {
  const token = (globalThis as any).__baml_invocation_token;
  if (!token) {
    throw new Error("Missing invocation token");
  }
  const session = await (globalThis as any).openToolSession(toolName, token);
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
    throw new Error(step.error?.message || "Tool error");
  }
  return step;
}
const invokeTool = invokeHostTool;
(globalThis as any).invokeTool = invokeHostTool;

// Minimal A2A handler for policy explainability demo.
// Runs inside the QuickJS sandbox.

// eslint-disable-next-line @typescript-eslint/no-unused-vars
(globalThis as any).handle_a2a_request = async function(request: any) {
  const ctx = request?.params?.message?.contextId || "ctx-missing";
  const text = request?.params?.message?.parts?.[0]?.text || "";

  let deny_result: { error: string } | null = null;
  if (text.toLowerCase().includes("deny")) {
    try {
      await (globalThis as any).invokeTool("defi/odos_swap", {
        action: "quote",
        input_token: "0xEeeeeEeeeEeEeeEeEeEeeEEEeeeeEeeeeeeeEEeE",
        output_token: "0x0000000000000000000000000000000000000000",
        amount: "1"
      });
    } catch (err: any) {
      deny_result = { error: String(err) };
    }
  }
  const metrics = await (globalThis as any).invokeTool("defi/paper_trading", {
    action: "get_metrics",
    error_class: "transient"
  });

  const artifacts: any[] = [
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
