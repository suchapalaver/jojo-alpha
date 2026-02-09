
// A2A shim (in case the builder prelude is not injected)
if (!(globalThis as any).__baml_chat_register) {
  (globalThis as any).__baml_chat_yield = function (chunk: any) {
    if ((globalThis as any).__baml_chat_yield_buffer) {
      (globalThis as any).__baml_chat_yield_buffer.push(chunk);
    }
  };
  (globalThis as any).__baml_chat_register = function (agent: any) {
    if (agent?.onChatMessage) {
      (globalThis as any).onChatMessage = agent.onChatMessage;
    }
    if (agent?.tools && typeof agent.tools === "object") {
      (globalThis as any).__js_tools = (globalThis as any).__js_tools || {};
      for (const name of Object.keys(agent.tools)) {
        (globalThis as any).__js_tools[name] = agent.tools[name];
      }
    }
  };
}

// Host tool helper (agent-platform expects openToolSession for host tools).
// Note: This helper is duplicated across agent entrypoints; keep in sync.
async function invokeHostTool(toolName: string, args: any, token?: string) {
  const resolvedToken = token ?? (globalThis as any).__baml_invocation_token;
  if (!resolvedToken) {
    throw new Error("Missing invocation token");
  }
  const session = await (globalThis as any).openToolSession(toolName, resolvedToken);
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

function extractText(message: { parts?: { text?: string }[] } | null | undefined): string {
  const first = message?.parts?.[0];
  if (first && typeof first.text === "string") return first.text;
  return "passkey-demo";
}

function newMessage(text: string) {
  return { parts: [{ text }] };
}

// Passkey-like signing demo handler.
// Runs inside the QuickJS sandbox.
async function onChatMessage(message: { parts: { text?: string }[]; __baml_invocation_token?: string }) {
  const text = extractText(message);
  // Pass explicit token from the message for stream calls; fallback to global token if present.
  const token = message.__baml_invocation_token;

  const address = await invokeTool("defi/wallet_derive_address", {}, token);
  const signature = await invokeTool("defi/wallet_sign_message", {
    message: `passkey-demo:${text}`
  }, token);

  let deny_result: { error: string } | null = null;
  if (text.toLowerCase().includes("deny")) {
    try {
      await invokeTool("defi/wallet_sign_tx", {
        tx_hash: "0x" + "11".repeat(32)
      }, token);
    } catch (err: any) {
      deny_result = { error: String(err) };
    }
  }

  const artifacts: any[] = [
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

  (globalThis as any).__baml_chat_yield({
    message: newMessage("passkey demo complete"),
    task: {
      status: { state: "TASK_STATE_COMPLETED", message: newMessage(text) },
      artifacts
    }
  });
}

(globalThis as any).__baml_chat_register({ onChatMessage });
