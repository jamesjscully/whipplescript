#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WHIP=(cargo run --quiet --manifest-path "$ROOT/Cargo.toml" -p whipplescript --)

check_example() {
  local path="$1"
  shift || true
  "${WHIP[@]}" check "$ROOT/$path" "$@" >/dev/null
  # Examples must also be lint-clean: this dogfoods `whip lint` (any new analysis
  # that false-positives on real code fails here) and keeps the examples free of
  # dead declarations. Lint warnings exit 0, so assert on the findings text.
  local lint_out
  lint_out="$("${WHIP[@]}" lint "$ROOT/$path" "$@" 2>/dev/null || true)"
  if printf '%s' "$lint_out" | grep -q 'warning \[lint'; then
    printf 'lint findings in %s:\n%s\n' "$path" "$lint_out" >&2
    exit 1
  fi
}

check_example examples/minimal-noop.whip
check_example examples/human-review.whip
check_example examples/triage-flow.whip
check_example examples/coerce-branch.whip
check_example examples/terminal-output-union.whip
check_example examples/incident-router.whip
check_example examples/scheduled-escalation.whip
check_example examples/exec-json-ingest.whip
check_example examples/deterministic-validation.whip
check_example examples/event-bridge.whip
check_example examples/messaging-demo.whip
check_example examples/file-store-demo.whip
check_example examples/reusable-review-pattern.whip
check_example examples/reusable-action-chain.whip
check_example examples/queue-worker-with-review.whip
check_example examples/multi-agent-bounded-concurrency.whip
check_example examples/circuit-breaker.whip
check_example examples/ralph.whip
check_example examples/openclaw-lite.whip --package-lock examples/openclaw-lite.lock.json
check_example examples/autoresearch-lite.whip
check_example examples/gastown-lite.whip
check_example examples/revision-ticket-v1.whip
check_example examples/revision-ticket-v2.whip
check_example examples/revision-parent-child.whip --root ParentRevisionExample
check_example examples/revision-validation-approval.whip --root RevisionValidation
check_example examples/revision-running-cancel.whip
check_example examples/revision-repair-planner.whip

printf 'docs examples check + lint passed\n'
