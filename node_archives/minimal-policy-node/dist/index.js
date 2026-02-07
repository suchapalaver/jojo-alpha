// Minimal A2A handler for policy explainability demo.
// Runs inside the QuickJS sandbox.

globalThis.handle_a2a_request = async function(request) {
  const ctx = request?.params?.message?.contextId || "ctx-missing";
  const metrics = await globalThis.invokeTool("paper_trading", {
    action: "get_metrics",
    error_class: "transient"
  });

  return {
    task: {
      id: "task-policy-demo",
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
