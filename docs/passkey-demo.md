# Passkey Demo: Policy-Gated Signing Ladder

This demo shows how an agent can *act* without ever accessing a private key. The agent asks for signing, but the host enforces policy and only returns signatures when allowed. The deny path proves that policy blocks dangerous actions and exposes the reason.

## Problem This Solves

Developers want agents to do real work (sign, transact), but do not want to hand them raw keys or unrestricted capability. This demo implements a **passkey-like signing ladder**:

1. Derive address (safe, read-only)
2. Sign a message (allowed by policy)
3. Sign a transaction hash (denied by policy)

The result is capability‑gated signing with auditability.

## Architecture Context

The demo runs inside the project’s safety pipeline:

```
TypeScript Agent (QuickJS sandbox)
         ↓
    BAML runtime
         ↓
    Interceptor Pipeline (policy first)
         ↓
    Rust Tools (signing ladder)
         ↓
    SecureWallet (keys never leave)
```

## Demo Node (Agent Handler)

The passkey demo node calls the signing ladder tools. This is the agent’s code, running inside the sandbox:

```ts
// node_archives/passkey-demo-node/src/index.ts
(globalThis as any).handle_a2a_request = async function(request: any) {
  const ctx = request?.params?.message?.contextId || "ctx-missing";
  const text = request?.params?.message?.parts?.[0]?.text || "";

  const address = await (globalThis as any).invokeTool("wallet_derive_address", {});
  const signature = await (globalThis as any).invokeTool("wallet_sign_message", {
    message: `passkey-demo:${ctx}`
  });

  let deny_result: { error: string } | null = null;
  if (text.toLowerCase().includes("deny")) {
    try {
      await (globalThis as any).invokeTool("wallet_sign_tx", {
        tx_hash: "0x" + "11".repeat(32)
      });
    } catch (err: any) {
      deny_result = { error: String(err) };
    }
  }

  const artifacts: any[] = [
    { name: "wallet_address", parts: [{ text: JSON.stringify(address) }] },
    { name: "signed_message", parts: [{ text: JSON.stringify(signature) }] }
  ];

  if (deny_result) {
    artifacts.push({ name: "policy_denied", parts: [{ text: JSON.stringify(deny_result) }] });
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
```

## Policy: Default-Deny with Explicit Allows

The demo policy allows address derivation and message signing, but denies transaction signing:

```json
// node_archives/passkey-demo-node/policy.json
{
  "mode": "default-deny",
  "rules": [
    {
      "tool": "wallet_derive_address",
      "allowed": true,
      "rule_id": "allow:wallet_derive_address",
      "reason": "read-only address derivation"
    },
    {
      "tool": "wallet_sign_message",
      "allowed": true,
      "rule_id": "allow:wallet_sign_message",
      "reason": "explicit message signing allowed"
    },
    {
      "tool": "wallet_sign_tx",
      "allowed": false,
      "rule_id": "deny:wallet_sign_tx",
      "reason": "transaction signing disabled in demo"
    }
  ]
}
```

## Signing Ladder Tools (Host-Side)

The signing tools are implemented in Rust and use `SecureWallet` for signing. The tools never expose the private key.

### Derive Address

```rust
// src/tools/wallet_signing.rs
impl BamlTool for WalletDeriveAddressTool {
    const NAME: &'static str = "wallet_derive_address";

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    }

    async fn execute(&self, _args: Value) -> Result<Value> {
        Ok(json!({
            "address": self.wallet.address_string()
        }))
    }
}
```

### Sign Message (EIP‑191)

```rust
// src/tools/wallet_signing.rs
impl BamlTool for WalletSignMessageTool {
    const NAME: &'static str = "wallet_sign_message";

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "message": {
                    "type": "string",
                    "description": "Message to sign (UTF-8)."
                }
            },
            "required": ["message"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let message = args
            .get("message")
            .and_then(|v| v.as_str())
            .ok_or_else(|| BamlRtError::InvalidArgument("Missing message".to_string()))?;

        let hash = eip191_hash_message(message.as_bytes());
        let signature = self
            .wallet
            .sign_hash(&b256_to_array(hash))
            .await
            .map_err(|e| BamlRtError::ToolExecution(e.to_string()))?;

        Ok(json!({
            "address": self.wallet.address_string(),
            "message_hash": signature_message_hash(hash),
            "signature": signature.to_string()
        }))
    }
}
```

### Sign Transaction Hash (Denied in Demo)

```rust
// src/tools/wallet_signing.rs
impl BamlTool for WalletSignTxTool {
    const NAME: &'static str = "wallet_sign_tx";

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "tx_hash": { "type": "string" },
                "tx_bytes": { "type": "string" }
            },
            "oneOf": [
                {"required": ["tx_hash"]},
                {"required": ["tx_bytes"]}
            ],
            "additionalProperties": false
        })
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        // Hash selection (tx_hash or keccak256(tx_bytes))
        // Sign using SecureWallet
        // Return address + hash + signature
        // (Policy denies this in demo)
        Ok(/* ... */)
    }
}
```

## Policy Enforcement (Harness)

The telemetry harness enforces policy before any tool executes:

```rust
// src/bin/telemetry_harness.rs
impl ToolInterceptor for HarnessPolicyInterceptor {
    async fn intercept_tool_call(
        &self,
        context: &ToolCallContext,
    ) -> baml_rt::error::Result<InterceptorDecision> {
        let Some(tool) = ToolName::new(&context.tool_name) else {
            return Ok(InterceptorDecision::Block(format!(
                "Invalid tool name '{}' for policy enforcement",
                context.tool_name
            )));
        };
        let decision = self.policy.decision_for_tool(&tool);
        if decision.allowed {
            return Ok(InterceptorDecision::Allow);
        }
        let rule_id = decision
            .rule_id
            .as_ref()
            .map(|id| format!(" rule_id={}", id))
            .unwrap_or_default();
        Ok(InterceptorDecision::Block(format!(
            "Policy denied tool {}: {}{}",
            context.tool_name, decision.reason, rule_id
        )))
    }
}
```

## How to Run (Allow + Deny)

Allow path (derive + sign message):

```bash
RUST_LOG=info nix develop -c cargo run --bin telemetry_harness -- \
  --agent ./node_archives/passkey-demo-node \
  --provenance-out ./telemetry/provenance.jsonl \
  --snapshot-out ./telemetry/snapshot.json \
  --message "passkey demo" \
  --use-agent-handler
```

Deny path (attempt sign tx → policy block):

```bash
RUST_LOG=info nix develop -c cargo run --bin telemetry_harness -- \
  --agent ./node_archives/passkey-demo-node \
  --provenance-out ./telemetry/provenance.jsonl \
  --snapshot-out ./telemetry/snapshot.json \
  --message "deny signing" \
  --use-agent-handler
```

## What Success Looks Like

- Allow path artifacts: `wallet_address`, `signed_message`
- Deny path artifacts: `wallet_address`, `signed_message`, `policy_denied`
- Provenance JSONL shows a blocked tool call for `wallet_sign_tx`
- Snapshot shows `wallet_sign_tx` with failures and the deny rule

## Why This Demonstrates Passkey‑Like Signing

- The agent never touches a private key.
- Signing is only possible via a host‑side tool.
- Policy can deny risky actions while still allowing safe ones.
- Every decision is captured in telemetry + provenance for auditability.
