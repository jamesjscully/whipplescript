#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

tmpdir="$(mktemp -d)"
trap 'rm -rf "$tmpdir"' EXIT

template="examples/templates/simple-agent-supervisor.whip"
template_policy="examples/policies/local-file-backed.policy.json"
template_store="$tmpdir/template.sqlite"
template_harness="$tmpdir/harness.json"
review_template="$tmpdir/review-template.whip"
review_store="$tmpdir/review.sqlite"
review_file="$tmpdir/reviews.json"
init_dir="$tmpdir/init-project"

cat >"$template_harness" <<'EOF'
{
  "agents": {
    "worker": {
      "provider": "command",
      "command": ["sh", "-c", "printf 'worker complete'"]
    }
  }
}
EOF

target/debug/whip init "$init_dir" --name DocsSmoke --json >/dev/null
grep -q 'machine DocsSmoke' "$init_dir/workflow.whip"
target/debug/whip validate "$init_dir/workflow.whip" --json >/dev/null
target/debug/whip validate-policy "$init_dir/.whipplescript/policy.json" --json >/dev/null
test -d "$init_dir/.whipplescript/state"
test -d "$init_dir/.whipplescript/workflows"

target/debug/whip validate \
  "$template" \
  --policy "$template_policy" \
  --json >/dev/null

target/debug/whip build \
  "$template" \
  --policy "$template_policy" \
  --out "$tmpdir/template-build" \
  --json >/dev/null

target/debug/whip run \
  "$template" \
  --store "$template_store" \
  --policy "$template_policy" \
  --event idle \
  --payload '{"activeRuns":0,"unfinishedItems":1}' \
  --json >/dev/null

target/debug/whip harness status \
  "$template" \
  --store "$template_store" \
  --json >/dev/null

overview="$(target/debug/whip overview \
  "$template" \
  --store "$template_store" \
  --policy "$template_policy")"
grep -q 'waiting: waiting for active invocation(s): worker=1' <<<"$overview"
grep -q 'data summary: {"seenRuns":0}' <<<"$overview"

target/debug/whip harness once \
  "$template" \
  --store "$template_store" \
  --config "$template_harness" \
  --json >/dev/null

target/debug/whip run \
  "$template" \
  --store "$template_store" \
  --policy "$template_policy" \
  --json >/dev/null

settled_overview="$(target/debug/whip overview \
  "$template" \
  --store "$template_store" \
  --policy "$template_policy")"
grep -q 'waiting: idle; no queued events or active invocations' <<<"$settled_overview"
grep -q 'data summary: {"seenRuns":1}' <<<"$settled_overview"

compact_status="$(target/debug/whip status \
  "$template" \
  --store "$template_store" \
  --policy "$template_policy" \
  --compact)"
grep -q 'waiting: idle; no queued events or active invocations' <<<"$compact_status"
grep -q 'current blockers: none' <<<"$compact_status"

target/debug/whip events \
  "$template" \
  --store "$template_store" \
  --policy "$template_policy" \
  --json >/dev/null

target/debug/whip log \
  "$template" \
  --store "$template_store" \
  --policy "$template_policy" \
  --json >/dev/null

cat >"$review_template" <<'EOF'
machine ReviewTemplate
initial waiting

event go {
  reason string
}

state waiting {
  on go as evt {
    askHuman(evt.reason)
    stay
  }
}
EOF

target/debug/whip run \
  "$review_template" \
  --store "$review_store" \
  --review-file "$review_file" \
  --policy "$template_policy" \
  --event go \
  --payload '{"reason":"approve release"}' \
  --json >/dev/null

grep -q '"status": "open"' "$review_file"
review_id="$(sed -n 's/.*"id": "\(review-[^"]*\)".*/\1/p' "$review_file" | head -n 1)"
test -n "$review_id"

target/debug/whip emit \
  "$review_template" \
  --store "$review_store" \
  --review-file "$review_file" \
  --event humanReview.responded \
  --payload "{\"reviewId\":\"$review_id\",\"decision\":\"approved\",\"response\":\"continue\"}" \
  --json >/dev/null

grep -q '"status": "responded"' "$review_file"
grep -q '"responses"' "$review_file"
grep -q '"decision": "approved"' "$review_file"

grep -q 'whip events workflow.whip --status failed --json' \
  spec/statechart-workflows/operations.md
grep -q 'whip events workflow.whip --status dead_lettered --json' \
  spec/statechart-workflows/operations.md
grep -q 'whip retry-event workflow.whip --event-id <event-id> --json' \
  spec/statechart-workflows/operations.md
grep -q 'scripts/check-docs.sh' README.md
grep -q 'retry-event --event-id evt_cli_...' README.md
grep -q 'git diff --check' .github/workflows/ci.yml
grep -q 'git diff --check' spec/statechart-workflows/release-checklist.md
grep -q 'whip events workflow.whip --status dead_lettered --json' \
  skills/whipplescript-statechart/SKILL.md
grep -q 'whip retry-event workflow.whip --event-id evt_cli_... --json' \
  skills/whipplescript-statechart/SKILL.md
grep -q 'events --limit' skills/whipplescript-statechart/SKILL.md
grep -q 'last_error' skills/whipplescript-statechart/SKILL.md
grep -q 'last_error' spec/statechart-workflows/product-surface.md
grep -q 'last_error' spec/statechart-workflows/operations.md
grep -q 'pending_events' skills/whipplescript-statechart/SKILL.md
grep -q 'pending event count' spec/statechart-workflows/product-surface.md
grep -q 'whip status \[file\] --compact' spec/statechart-workflows/product-surface.md
grep -q 'status --compact' README.md
grep -q 'status --compact' skills/whipplescript-statechart/SKILL.md
grep -q 'current_effect_failures' README.md
grep -q 'current_effect_failures' skills/whipplescript-statechart/SKILL.md
grep -q 'current_coerce_failure' README.md
grep -q 'current_coerce_failure' skills/whipplescript-statechart/SKILL.md
grep -q 'current effect failures' spec/statechart-workflows/runtime-semantics.md
grep -q 'current blockers' spec/statechart-workflows/runtime-semantics.md
grep -q 'agent worker = codingAgent()' skills/whipplescript-statechart/SKILL.md
grep -q 'payload does not match schema for event' skills/whipplescript-statechart/SKILL.md
grep -q 'creating an empty workflow store' spec/statechart-workflows/operations.md
grep -Fq 'recent_effects[].idempotency_key' skills/whipplescript-statechart/SKILL.md
grep -q 'effect idempotency keys' README.md
grep -q 'status JSON' spec/statechart-workflows/effects.md
grep -Fq 'recent_effects[]' spec/statechart-workflows/component-contracts.md
grep -q 'hidden built-in blocked state in v0' spec/statechart-workflows/effects.md
grep -q 'v0 does not create a hidden built-in blocked state' \
  spec/statechart-workflows/runtime-semantics.md
grep -q 'workflow_events' spec/statechart-workflows/storage.md
grep -q 'event_json TEXT NOT NULL' spec/statechart-workflows/storage.md
grep -q "schema version is \`4\`" spec/statechart-workflows/storage.md
grep -q 'unique within its `workflow_id`' spec/statechart-workflows/event-queue.md
grep -q 'Ignored events are durable records with reasons stored in `last_error`' \
  spec/statechart-workflows/event-queue.md
grep -q 'schema version `4`' spec/statechart-workflows/database-migrations.md
grep -q 'UNIQUE(workflow_id, event_id)' spec/statechart-workflows/database-migrations.md

target/debug/whip validate-policy \
  examples/policies/local-file-backed.policy.json \
  examples/policies/enterprise-baml-http.policy.json \
  examples/policies/spec-implementation.enterprise-policy.json \
  --json >/dev/null

for path in \
  spec/statechart-workflows/operations.md \
  spec/statechart-workflows/migration.md \
  spec/statechart-workflows/database-migrations.md \
  spec/statechart-workflows/release-checklist.md \
  examples/templates/README.md \
  skills/whipplescript-statechart/SKILL.md
do
  test -s "$path"
done
