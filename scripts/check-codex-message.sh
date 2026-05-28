#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPORT="${WHIPPLETREE_CODEX_SMOKE_REPORT:-$ROOT/target/codex-message-smoke-report.md}"
EXPECTED="${WHIPPLETREE_CODEX_SMOKE_EXPECTED:-WHIPPLETREE_CODEX_SMOKE_OK}"
PROMPT="${WHIPPLETREE_CODEX_SMOKE_PROMPT:-Reply with exactly: $EXPECTED}"
TIMEOUT_SECONDS="${WHIPPLETREE_CODEX_SMOKE_TIMEOUT:-180}"

if ! command -v codex >/dev/null 2>&1; then
  echo "missing required tool: codex" >&2
  exit 2
fi

WORKDIR="$(mktemp -d)"
cleanup() {
  if [[ "${WHIPPLETREE_KEEP_CODEX_SMOKE:-}" == "1" ]]; then
    echo "Kept Codex smoke workspace: $WORKDIR" >&2
  else
    rm -rf "$WORKDIR"
  fi
}
trap cleanup EXIT

LAST_MESSAGE="$WORKDIR/last-message.txt"
EVENTS_JSONL="$WORKDIR/events.jsonl"
STDERR_LOG="$WORKDIR/stderr.txt"

args=(
  exec
  --json
  --cd "$ROOT"
  --sandbox read-only
  -c 'approval_policy="never"'
  --output-last-message "$LAST_MESSAGE"
)

if [[ -n "${WHIPPLETREE_CODEX_MODEL:-}" ]]; then
  args+=(-m "$WHIPPLETREE_CODEX_MODEL")
fi

if [[ -n "${WHIPPLETREE_CODEX_PROFILE:-}" ]]; then
  args+=(-p "$WHIPPLETREE_CODEX_PROFILE")
fi

started_at="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"
set +e
timeout "$TIMEOUT_SECONDS" codex "${args[@]}" "$PROMPT" \
  >"$EVENTS_JSONL" \
  2>"$STDERR_LOG" \
  </dev/null
status=$?
set -e
finished_at="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"

response=""
if [[ -f "$LAST_MESSAGE" ]]; then
  response="$(tr -d '\r\n' <"$LAST_MESSAGE")"
fi

thread_id="$(sed -n 's/^{"type":"thread.started","thread_id":"\([^"]*\)".*/\1/p' "$EVENTS_JSONL" | head -1)"
turn_completed="no"
if grep -q '"type":"turn.completed"' "$EVENTS_JSONL"; then
  turn_completed="yes"
fi
model_override="unset"
if [[ -n "${WHIPPLETREE_CODEX_MODEL:-}" ]]; then
  model_override="set"
fi
profile_override="unset"
if [[ -n "${WHIPPLETREE_CODEX_PROFILE:-}" ]]; then
  profile_override="set"
fi

mkdir -p "$(dirname "$REPORT")"
{
  echo "# Codex Message Smoke Report"
  echo
  echo "- Started: $started_at"
  echo "- Finished: $finished_at"
  echo "- Exit code: $status"
  echo "- Codex: $(command -v codex)"
  echo "- Version: $(codex --version 2>/dev/null | head -1 || true)"
  echo "- Model override: $model_override"
  echo "- Profile override: $profile_override"
  echo "- Thread id: ${thread_id:-unavailable}"
  echo "- Turn completed: $turn_completed"
  echo "- Expected response: \`$EXPECTED\`"
  echo "- Actual response: \`$response\`"
  echo
  echo "## Events"
  echo
  echo '```jsonl'
  sed 's/```/` ` `/g' "$EVENTS_JSONL"
  echo '```'
  echo
  echo "## Stderr"
  echo
  echo '```text'
  sed 's/```/` ` `/g' "$STDERR_LOG"
  echo '```'
} >"$REPORT"

if [[ "$status" -ne 0 ]]; then
  cat "$STDERR_LOG" >&2
  echo "Codex smoke failed with exit code $status" >&2
  echo "Wrote Codex smoke report: $REPORT" >&2
  exit "$status"
fi

if [[ "$response" != "$EXPECTED" ]]; then
  echo "Codex smoke response mismatch: expected \`$EXPECTED\`, got \`$response\`" >&2
  echo "Wrote Codex smoke report: $REPORT" >&2
  exit 1
fi

if [[ "$turn_completed" != "yes" ]]; then
  echo "Codex smoke did not emit turn.completed" >&2
  echo "Wrote Codex smoke report: $REPORT" >&2
  exit 1
fi

echo "Codex message smoke passed: $response"
echo "Wrote Codex smoke report: $REPORT"
