// Passkey-like signing demo handler.
// Runs inside the QuickJS sandbox.

globalThis.handle_a2a_request = async function(request) {
  const ctx = request?.params?.message?.contextId || "ctx-missing";
  const text = request?.params?.message?.parts?.[0]?.text || "";

  const address = await globalThis.invokeTool("wallet_derive_address", {});
  const signature = await globalThis.invokeTool("wallet_sign_message", {
    message: `passkey-demo:${ctx}`
  });

  let deny_result = null;
  if (text.toLowerCase().includes("deny")) {
    try {
      await globalThis.invokeTool("wallet_sign_tx", {
        tx_hash: "0x" + "11".repeat(32)
      });
    } catch (err) {
      deny_result = { error: String(err) };
    }
  }

  const artifacts = [
    {
      name: "wallet_address",
      parts: [{ text: JSON.stringify(address) }]
    },
    {
      name: "signed_message",
      parts: [{ text: JSON.stringify(signature) }]
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
      id: "task-passkey-demo",
      contextId: ctx,
      status: { state: "TASK_STATE_COMPLETED" },
      history: [],
      artifacts
    }
  };
};
