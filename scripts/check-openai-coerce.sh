#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
HOST="${WHIPPLESCRIPT_OPENAI_COERCE_HOST:-127.0.0.1}"
PORT="${WHIPPLESCRIPT_OPENAI_COERCE_PORT:-18765}"
ENDPOINT="http://$HOST:$PORT"
LOG="$ROOT/target/openai-coerce-server.log"

if [[ -f "$ROOT/.env" ]]; then
  set -a
  # shellcheck disable=SC1091
  source "$ROOT/.env"
  set +a
fi

if [[ -z "${OPENAI_API_KEY:-}" ]]; then
  echo "OPENAI_API_KEY is required in the environment or .env" >&2
  exit 2
fi

mkdir -p "$ROOT/target"

node "$ROOT/scripts/openai-coerce-server.mjs" >"$LOG" 2>&1 &
server_pid=$!

cleanup() {
  kill "$server_pid" >/dev/null 2>&1 || true
  wait "$server_pid" >/dev/null 2>&1 || true
}
trap cleanup EXIT

for _ in {1..50}; do
  if curl --fail --silent --show-error --max-time 1 "$ENDPOINT/health" >/dev/null 2>&1; then
    break
  fi
  if ! kill -0 "$server_pid" >/dev/null 2>&1; then
    echo "OpenAI coerce server exited early:" >&2
    sed 's/sk-[A-Za-z0-9_-]*/sk-REDACTED/g' "$LOG" >&2
    exit 2
  fi
  sleep 0.1
done

if ! curl --fail --silent --show-error --max-time 1 "$ENDPOINT/health" >/dev/null; then
  echo "OpenAI coerce server did not become healthy at $ENDPOINT" >&2
  sed 's/sk-[A-Za-z0-9_-]*/sk-REDACTED/g' "$LOG" >&2
  exit 2
fi

WHIPPLESCRIPT_E2E_REAL_PROVIDERS=1 \
WHIPPLESCRIPT_REAL_PROVIDERS=baml \
WHIPPLESCRIPT_BAML_SKIP_CLI=1 \
WHIPPLESCRIPT_BAML_TEST_ENDPOINT="$ENDPOINT" \
WHIPPLESCRIPT_BAML_HEALTH_PATH=/health \
WHIPPLESCRIPT_BAML_TEST_FUNCTION="${WHIPPLESCRIPT_BAML_TEST_FUNCTION:-classifyMessage}" \
WHIPPLESCRIPT_BAML_TEST_ARGUMENTS_JSON="${WHIPPLESCRIPT_BAML_TEST_ARGUMENTS_JSON:-{\"title\":\"Pager alert\",\"body\":\"Production checkout is down for all customers\"}}" \
WHIPPLESCRIPT_BAML_TEST_OUTPUT_TYPE="${WHIPPLESCRIPT_BAML_TEST_OUTPUT_TYPE:-MessageClassification}" \
  "$ROOT/scripts/check-real-providers-report.sh"
