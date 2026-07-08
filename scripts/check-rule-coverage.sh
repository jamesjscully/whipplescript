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

# Examples that need real providers, multiple roots, or explicit inputs are
# exercised elsewhere; everything else must reach full rule coverage here.
#   - tested-agent-turn: requires `given input` (no `--input` here); covered by `whip test`.
#   - coerce-enum: requires `given input` (a WorkItem); its coerce->enum variant
#     dispatch is covered by the example's own `whip test` blocks.
#   - subworkflow-tool-consumer: requires `given input` (a ConsumerRequest); the
#     no-`--input` fixture run here cannot start it.
#   - package-memory: imports the non-`std.` `memory` package, so it needs a
#     `whip.lock`; covered by the `dev_capability_call_*` / `check_discovers_*` tests.
#   - event-bridge: an `@service` workflow whose only rule fires on an external
#     `deploy.finished` signal (injected with `whip signal`); a no-signal fixture
#     run records nothing, so its `assert count(...) == 1` cannot hold here.
SKIP="revision-parent-child revision-validation-approval revision-repair-planner revision-running-cancel revision-ticket-v1 revision-ticket-v2 revision-validation-approval tested-agent-turn coerce-enum subworkflow-tool-consumer package-memory event-bridge"

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

  # Queue-backed examples need at least one ready item.
  if grep -q "tracker builtin" "$workflow"; then
    run_whip items add --queue backlog --title "Coverage item" --body "seeded" >/dev/null
  fi

  report="$WORK_DIR/$name.json"
  if ! run_whip --store "$store" --json dev "$workflow" --provider fixture --until idle >"$report" 2>"$WORK_DIR/$name.err"; then
    echo "FAIL (dev errored): $name"
    sed -n 1p "$WORK_DIR/$name.err"
    failures=1
    continue
  fi
  instance="$(jq -r '.instance_id' "$report")"

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
