#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WHIP="${WHIPPLESCRIPT_BIN:-cargo run -q -p whipplescript --}"
WORKFLOW="$ROOT/examples/loft-gated-smoke.whip"
STORE_DIR="$ROOT/target/loft-gated-smoke"
SUCCESS_STORE="$STORE_DIR/success.sqlite"
FAIL_STORE="$STORE_DIR/failure.sqlite"
SUCCESS_JSON="$STORE_DIR/success.json"
FAIL_JSON="$STORE_DIR/failure.json"

mkdir -p "$STORE_DIR"
rm -f "$SUCCESS_STORE" "$FAIL_STORE" "$SUCCESS_JSON" "$FAIL_JSON"

run_whip() {
  # shellcheck disable=SC2086
  $WHIP "$@"
}

run_whip check "$WORKFLOW" >/dev/null

run_whip --store "$SUCCESS_STORE" --json dev "$WORKFLOW" --provider fixture --until idle >"$SUCCESS_JSON"

jq -e '
  (.assertions | length) == 4
  and all(.assertions[]; .passed == true)
  and ([.workers[].terminal_events | length] | add) == 2
  and ([.steps[].effects_created] | add) == 2
' "$SUCCESS_JSON" >/dev/null

instance_id="$(jq -r '.instance_id' "$SUCCESS_JSON")"
run_whip --store "$SUCCESS_STORE" --json effects "$instance_id" |
  jq -e '
    ([.[] | select(.kind == "loft.claim" and .status == "completed")] | length) == 1
    and ([.[] | select(.kind == "agent.tell" and .status == "completed")] | length) == 1
    and ([.[] | select(.kind == "human.ask")] | length) == 0
  ' >/dev/null

run_whip --store "$SUCCESS_STORE" --json facts "$instance_id" |
  jq -e '
    ([.[] | select(.name == "LoftGatedResult" and (.value.status == "done"))] | length) == 1
    and ([.[] | select(.name == "loft.claim.succeeded")] | length) == 1
    and ([.[] | select(.name == "loft.claim.failed")] | length) == 0
  ' >/dev/null

set +e
run_whip --store "$FAIL_STORE" --json dev "$WORKFLOW" --provider fixture --fail --until idle >"$FAIL_JSON"
failure_status=$?
set -e

if [[ "$failure_status" -eq 0 ]]; then
  echo "expected Loft failure run to fail assertions, but it exited successfully" >&2
  exit 1
fi

jq -e '
  any(.assertions[]; .passed == false and (.expr | contains("agent.tell")))
  and any(.assertions[]; .passed == false and (.expr | contains("LoftGatedResult")))
' "$FAIL_JSON" >/dev/null

failure_instance_id="$(jq -r '.instance_id' "$FAIL_JSON")"
run_whip --store "$FAIL_STORE" --json effects "$failure_instance_id" |
  jq -e '
    ([.[] | select(.kind == "loft.claim" and .status == "failed")] | length) == 1
    and ([.[] | select(.kind == "agent.tell")] | length) == 0
    and ([.[] | select(.kind == "human.ask" and .status == "completed")] | length) == 1
  ' >/dev/null

run_whip --store "$FAIL_STORE" --json facts "$failure_instance_id" |
  jq -e '
    ([.[] | select(.name == "loft.claim.failed")] | length) == 1
    and ([.[] | select(.name == "loft.claim.succeeded")] | length) == 0
    and ([.[] | select(.name == "LoftGatedResult")] | length) == 0
  ' >/dev/null

echo "Loft-gated smoke passed"
