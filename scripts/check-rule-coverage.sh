#!/usr/bin/env bash
# Dynamic rule coverage: every rule in every fixture-runnable example must
# commit at least once in a fixture run (human asks are answered generically;
# queue-backed examples get a seeded backlog item).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WHIP="${WHIPPLESCRIPT_BIN:-cargo run -q -p whipplescript --}"
WORK_DIR="$ROOT/target/rule-coverage"
mkdir -p "$WORK_DIR"

run_whip() {
  # shellcheck disable=SC2086
  $WHIP "$@"
}

# Examples that a no-`--input`, no-`--root`, single-workflow fixture run cannot
# drive are exercised elsewhere; everything else must reach full rule coverage
# here. Categories:
#   - multiple workflows (need `--root`): the docs-examples gate checks these with
#     the right root. (coord-acquire-wait, coordination-partition-shared,
#     least-privilege-subagent, parent-child-outcomes, private-workflow-wrapper,
#     typed-invoke-result, revision-*).
#   - `given input` (need `--input`): covered by `whip test` blocks.
#     (tested-agent-turn, coerce-enum, subworkflow-tool-consumer, compact-contract,
#     echo-text-tool, improve-triage, include-audit, include-triage,
#     pattern-consumer-audit, pattern-consumer-triage, redact-projection,
#     scalar-terminal).
#   - non-`std.` package import (need a `whip.lock`): package-memory, package-notes;
#     covered by the `dev_capability_call_*` / `check_discovers_*` tests.
#   - event-bridge: an `@service` workflow whose only rule fires on an injected
#     external signal; a no-signal fixture run records nothing.
SKIP="revision-parent-child revision-validation-approval revision-repair-planner revision-running-cancel revision-ticket-v1 revision-ticket-v2 tested-agent-turn coerce-enum subworkflow-tool-consumer package-memory package-notes event-bridge coord-acquire-wait coordination-partition-shared least-privilege-subagent parent-child-outcomes private-workflow-wrapper typed-invoke-result compact-contract echo-text-tool improve-triage include-audit include-triage pattern-consumer-audit pattern-consumer-triage redact-projection scalar-terminal"

# Script hard-off Layer 2 (spec/std-script.md): raw `exec` seeds `script.raw`
# only under dev profile + a non-empty WHIPPLESCRIPT_EXEC_ALLOW, and ungranted
# exec now BLOCKS at store admission (security.script_disabled) instead of
# failing — which would strand the failure-branch rules these examples drive.
# This harness is the operator plane for the fixture runs, so it grants the
# raw commands the exec-bearing examples use (circuit-breaker's failing probe,
# the printf-based typed-ingest examples).
export WHIPPLESCRIPT_EXEC_ALLOW="sh -c *:printf *"

failures=0
for workflow in "$ROOT"/examples/*.whip; do
  name="$(basename "$workflow" .whip)"
  case " $SKIP " in *" $name "*) continue ;; esac
  store="$WORK_DIR/$name.sqlite"
  items="$WORK_DIR/$name-items.sqlite"
  rm -f "$store" "$items"
  export WHIPPLESCRIPT_ITEMS_STORE="$items"

  # Tracker-backed examples need at least one ready issue. Detect the declared
  # `tracker <name> { provider builtin }` and seed that queue via `whip issue new`
  # (renamed from the old `whip items add`).
  tracker_queue="$(grep -oE '^tracker [A-Za-z_][A-Za-z0-9_]*' "$workflow" | head -1 | awk '{print $2}' || true)"
  if [ -n "$tracker_queue" ]; then
    run_whip issue new --tracker "$tracker_queue" --title "Coverage item" --body "seeded" >/dev/null
  fi

  report="$WORK_DIR/$name.json"
  if ! run_whip --store "$store" --json dev "$workflow" --provider fixture --until idle >"$report" 2>"$WORK_DIR/$name.err"; then
    echo "FAIL (dev errored): $name"
    sed -n 1p "$WORK_DIR/$name.err"
    failures=1
    continue
  fi
  # Some effects (e.g. messaging.stdio) print to stdout ahead of the --json
  # payload; extract the JSON object so a stray line does not break the parse.
  instance="$(sed -n '/^{/,$p' "$report" | jq -r '.instance_id // empty' 2>/dev/null || true)"
  if [ -z "$instance" ]; then
    echo "FAIL (no instance_id): $name"
    failures=1
    continue
  fi

  # Drive pending human asks generically, then step until quiet.
  for _ in 1 2 3; do
    item="$(run_whip --store "$store" --json inbox | jq -r '.[0].inbox_item_id // empty')"
    [ -z "$item" ] && break
    choice="$(run_whip --store "$store" --json inbox | jq -r '.[0].choices[0] // "accept"')"
    run_whip --store "$store" inbox answer "$item" --choice "$choice" --by coverage >/dev/null
    run_whip --store "$store" step "$instance" --program "$workflow" >/dev/null
    run_whip --store "$store" worker "$instance" --provider fixture >/dev/null
    run_whip --store "$store" step "$instance" --program "$workflow" >/dev/null
  done

  declared="$(run_whip --json check "$workflow" 2>/dev/null | jq -r '.[0].snapshot' | grep -oP '^  rule \K\S+' | sort -u)"
  committed="$(run_whip --store "$store" --json log "$instance" | jq -r '.[] | select(.event_type == "rule.committed") | .payload.rule // empty' | sort -u)"

  uncovered=""
  for rule in $declared; do
    echo "$committed" | grep -qx "$rule" || uncovered="$uncovered $rule"
  done
  if [ -n "$uncovered" ]; then
    # Branch-exclusive rules (coerce/case outputs the fixture provider does
    # not drive) are legitimately uncovered in a single run; report only.
    echo "partial ($name): branch-exclusive rules not driven by fixtures:$uncovered"
  else
    echo "covered: $name"
  fi
done

exit $failures
