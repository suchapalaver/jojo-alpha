#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
AGENT_PLATFORM_DIR="${ROOT_DIR}/../../semiotic-agentium/agent-platform"
AGENT_DIR="${ROOT_DIR}/agent"
OUT_TAR="${AGENT_PLATFORM_DIR}/agent.tar.gz"

if [[ ! -d "${AGENT_PLATFORM_DIR}" ]]; then
  echo "Expected agent-platform at ${AGENT_PLATFORM_DIR}" >&2
  exit 1
fi

if [[ ! -d "${AGENT_DIR}" ]]; then
  echo "Expected agent source at ${AGENT_DIR}" >&2
  exit 1
fi

echo "Building agent package with agent-platform baml-rt-builder..."
(
  cd "${AGENT_PLATFORM_DIR}"
  cargo run -p baml-rt-builder --bin baml-agent-builder -- \
    --agent-path "${AGENT_DIR}" \
    --out "${OUT_TAR}"
)

echo "Starting A2A runner (Ctrl+C to stop)..."
(
  cd "${AGENT_PLATFORM_DIR}"
  cargo run -p baml-agent-runner --bin baml-agent-runner -- \
    --agent "${OUT_TAR}"
)
