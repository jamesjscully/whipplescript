#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
HOST="${WHIPPLESCRIPT_OPENAI_COERCE_HOST:-127.0.0.1}"
PORT="${WHIPPLESCRIPT_OPENAI_COERCE_PORT:-18765}"
ENDPOINT="http://$HOST:$PORT"
LOG="$ROOT/target/openai-coerce-server.log"

load_openai_key_from_dotenv() {
  local file="$1"
  local line key value

  [[ -f "$file" ]] || return
  while IFS= read -r line || [[ -n "$line" ]]; do
    line="${line#"${line%%[![:space:]]*}"}"
    line="${line%"${line##*[![:space:]]}"}"
    [[ -n "$line" && "$line" != \#* ]] || continue
    key="${line%%=*}"
    value="${line#*=}"
    key="${key%"${key##*[![:space:]]}"}"
    value="${value#"${value%%[![:space:]]*}"}"
    value="${value%"${value##*[![:space:]]}"}"
    [[ "$key" == "OPENAI_API_KEY" ]] || continue
    if [[ "$value" == \"*\" && "$value" == *\" ]]; then
      value="${value:1:${#value}-2}"
    elif [[ "$value" == \'*\' && "$value" == *\' ]]; then
      value="${value:1:${#value}-2}"
    fi
    if [[ -z "${OPENAI_API_KEY:-}" ]]; then
      export OPENAI_API_KEY="$value"
    fi
  done <"$file"
}

generate_token() {
  if command -v openssl >/dev/null 2>&1; then
    openssl rand -hex 32
  else
    dd if=/dev/urandom bs=32 count=1 2>/dev/null | od -An -tx1 | tr -d ' \n'
  fi
}

if [[ -f "$ROOT/.env" ]]; then
  load_openai_key_from_dotenv "$ROOT/.env"
fi

if [[ -z "${OPENAI_API_KEY:-}" ]]; then
  echo "OPENAI_API_KEY is required in the environment or .env" >&2
  exit 2
fi

if [[ -z "${WHIPPLESCRIPT_OPENAI_COERCE_TOKEN:-}" ]]; then
  WHIPPLESCRIPT_OPENAI_COERCE_TOKEN="$(generate_token)"
  export WHIPPLESCRIPT_OPENAI_COERCE_TOKEN
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
WHIPPLESCRIPT_BAML_AUTH_TOKEN="$WHIPPLESCRIPT_OPENAI_COERCE_TOKEN" \
WHIPPLESCRIPT_BAML_SKIP_CLI=1 \
WHIPPLESCRIPT_BAML_TEST_ENDPOINT="$ENDPOINT" \
WHIPPLESCRIPT_BAML_HEALTH_PATH=/health \
WHIPPLESCRIPT_BAML_TEST_FUNCTION="${WHIPPLESCRIPT_BAML_TEST_FUNCTION:-classifyMessage}" \
WHIPPLESCRIPT_BAML_TEST_ARGUMENTS_JSON="${WHIPPLESCRIPT_BAML_TEST_ARGUMENTS_JSON:-{\"title\":\"Pager alert\",\"body\":\"Production checkout is down for all customers\"}}" \
WHIPPLESCRIPT_BAML_TEST_OUTPUT_TYPE="${WHIPPLESCRIPT_BAML_TEST_OUTPUT_TYPE:-MessageClassification}" \
  "$ROOT/scripts/check-real-providers-report.sh"
