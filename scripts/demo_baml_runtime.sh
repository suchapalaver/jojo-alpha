#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SANDBOX_DIR="${ROOT_DIR}/../baml-ts-sandbox"
AGENT_DIR="${ROOT_DIR}/agent"
OUT_TAR="${SANDBOX_DIR}/agent.tar.gz"

if [[ ! -d "${SANDBOX_DIR}" ]]; then
  echo "Expected baml-ts-sandbox at ${SANDBOX_DIR}" >&2
  exit 1
fi

if [[ ! -d "${AGENT_DIR}" ]]; then
  echo "Expected agent source at ${AGENT_DIR}" >&2
  exit 1
fi

echo "Building agent package with upstream baml-rt-builder..."
(
  cd "${SANDBOX_DIR}"
  cargo run -p baml-rt-builder --bin baml-agent-builder -- \
    --agent-path "${AGENT_DIR}" \
    --out "${OUT_TAR}"
)

echo "Starting A2A runner (Ctrl+C to stop)..."
(
  cd "${SANDBOX_DIR}"
  cargo run -p baml-agent-runner --bin baml-agent-runner -- \
    --agent "${OUT_TAR}"
)
