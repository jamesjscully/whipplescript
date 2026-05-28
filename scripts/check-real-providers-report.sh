#!/usr/bin/env bash
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPORT="${WHIPPLETREE_REAL_PROVIDER_REPORT:-$ROOT/target/real-provider-smoke-report.md}"
OUTPUT="$(mktemp)"

cleanup() {
  rm -f "$OUTPUT"
}
trap cleanup EXIT

env_state() {
  local name="$1"
  if [[ -n "${!name:-}" ]]; then
    echo "set"
  else
    echo "unset"
  fi
}

started_at="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"

set +e
"$ROOT/scripts/check-real-providers.sh" >"$OUTPUT" 2>&1
status=$?
set -e

finished_at="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"
mkdir -p "$(dirname "$REPORT")"

{
  echo "# Real Provider Smoke Report"
  echo
  echo "- Started: $started_at"
  echo "- Finished: $finished_at"
  echo "- Exit code: $status"
  echo "- Selected providers: ${WHIPPLETREE_REAL_PROVIDERS:-loft,baml}"
  echo "- Real-provider gate: $(env_state WHIPPLETREE_E2E_REAL_PROVIDERS)"
  echo "- Loft test issue: $(env_state WHIPPLETREE_LOFT_TEST_ISSUE)"
  echo "- Loft CLI override: $(env_state WHIPPLETREE_LOFT_CLI)"
  echo "- Loft repo override: $(env_state WHIPPLETREE_LOFT_REPO)"
  echo "- Loft repo preflight skip: $(env_state WHIPPLETREE_LOFT_SKIP_REPO_PREFLIGHT)"
  echo "- BAML endpoint: $(env_state WHIPPLETREE_BAML_TEST_ENDPOINT)"
  echo "- BAML function: $(env_state WHIPPLETREE_BAML_TEST_FUNCTION)"
  echo "- BAML arguments JSON: $(env_state WHIPPLETREE_BAML_TEST_ARGUMENTS_JSON)"
  echo "- BAML output type: $(env_state WHIPPLETREE_BAML_TEST_OUTPUT_TYPE)"
  echo "- BAML health path: $(env_state WHIPPLETREE_BAML_HEALTH_PATH)"
  echo "- BAML CLI skip: $(env_state WHIPPLETREE_BAML_SKIP_CLI)"
  echo
  echo "## Output"
  echo
  echo '```text'
  sed 's/```/` ` `/g' "$OUTPUT"
  echo '```'
} >"$REPORT"

cat "$OUTPUT"
echo "Wrote real-provider smoke report: $REPORT" >&2

exit "$status"
