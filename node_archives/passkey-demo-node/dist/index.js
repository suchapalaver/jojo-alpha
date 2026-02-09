
// A2A shim (in case the builder prelude is not injected)
if (!globalThis.__baml_chat_register) {
  globalThis.__baml_chat_yield = function(chunk) {
    if (globalThis.__baml_chat_yield_buffer) {
      globalThis.__baml_chat_yield_buffer.push(chunk);
    }
  };
  globalThis.__baml_chat_register = function(agent) {
    if (agent && agent.onChatMessage) {
      globalThis.onChatMessage = agent.onChatMessage;
    }
    if (agent && agent.tools && typeof agent.tools === "object") {
      globalThis.__js_tools = globalThis.__js_tools || {};
      for (const name of Object.keys(agent.tools)) {
        globalThis.__js_tools[name] = agent.tools[name];
      }
    }
  };
}

// Host tool helper (agent-platform expects openToolSession for host tools).
// Note: This helper is duplicated across agent entrypoints; keep in sync.
async function invokeHostTool(toolName, args, token) {
  const resolvedToken = token ?? globalThis.__baml_invocation_token;
  if (!resolvedToken) {
    throw new Error("Missing invocation token");
  }
  const session = await globalThis.openToolSession(toolName, resolvedToken);
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

function extractText(message) {
  const first = message?.parts?.[0];
  if (first && typeof first.text === "string") return first.text;
  return "passkey-demo";
}

function newMessage(text) {
  return { parts: [{ text }] };
}

// Passkey-like signing demo handler.
// Runs inside the QuickJS sandbox.
async function onChatMessage(message) {
  const text = extractText(message);
  // Pass explicit token from the message for stream calls; fallback to global token if present.
  const token = message.__baml_invocation_token;

  const address = await invokeTool("defi/wallet_derive_address", {}, token);
  const signature = await invokeTool("defi/wallet_sign_message", {
    message: `passkey-demo:${text}`
  }, token);

  let deny_result = null;
  if (text.toLowerCase().includes("deny")) {
    try {
      await invokeTool("defi/wallet_sign_tx", {
        tx_hash: "0x" + "11".repeat(32)
      }, token);
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

  globalThis.__baml_chat_yield({
    message: newMessage("passkey demo complete"),
    task: {
      status: { state: "TASK_STATE_COMPLETED", message: newMessage(text) },
      artifacts
    }
  });
}

globalThis.__baml_chat_register({ onChatMessage });
