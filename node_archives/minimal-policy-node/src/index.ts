// Minimal A2A handler for policy explainability demo.
// Runs inside the QuickJS sandbox.

// eslint-disable-next-line @typescript-eslint/no-unused-vars
(globalThis as any).handle_a2a_request = async function(request: any) {
  const ctx = request?.params?.message?.contextId || "ctx-missing";
  const text = request?.params?.message?.parts?.[0]?.text || "";

  let deny_result: { error: string } | null = null;
  if (text.toLowerCase().includes("deny")) {
    try {
      await (globalThis as any).invokeTool("odos_swap", {
        action: "quote",
        input_token: "0xEeeeeEeeeEeEeeEeEeEeeEEEeeeeEeeeeeeeEEeE",
        output_token: "0x0000000000000000000000000000000000000000",
        amount: "1"
      });
    } catch (err: any) {
      deny_result = { error: String(err) };
    }
  }
  const metrics = await (globalThis as any).invokeTool("paper_trading", {
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
