# RUNTIME_NOTES.md

Focused notes for the agent-platform (QuickJS) runtime and its integration.

## QuickJS Async Reality

- Promise resolution is driven by the host event loop.
- If the host loop stalls, async JS code will appear to "hang".
- Keep the event loop alive in `src/runner.rs` (the tool bridge depends on it).

## Tool Boundary Rules

- Tools are the only escape hatch from the sandbox.
- Treat tool schemas as a public API contract.
- Validate every tool input; reject malformed args early.
- Keep outputs deterministic where possible (avoid random ordering).

## Error Surfacing

- Prefer explicit error strings over silent failure.
- Fail closed for malformed tool args in safety-critical paths.
- Always include context in tool errors (which tool, which field).

## Replay and Debugging

- For non-deterministic behavior, capture:
  - Tool inputs and outputs
  - Time boundaries (UTC dates)
  - Query plans and partial data results
- A deterministic replay mode is a high-value future investment.

## Safe Defaults

- Dry-run mode must not trigger signing or on-chain sends.
- Quotes are read-only; they must not mutate spending state.
- Avoid implicit global state in JS; pass state explicitly.

## Upgrade Checklist (baml-rt)

When updating baml-rt or QuickJS:
- Re-run sandbox isolation tests (or add them if missing).
- Verify tool registration order and global exposure.
- Verify tool schemas still match JS expectations.
- Confirm event loop drive behavior is unchanged.

## References

- `src/runner.rs` - event loop and QuickJS bridge
- `src/tools/*` - tool definitions and schemas
- `agent/src/index.ts` - JS entry point and usage patterns
- `INVARIANTS.md` - sandbox and key isolation guarantees
