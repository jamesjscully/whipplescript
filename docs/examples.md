# Examples

Every curated example in [`examples/`](../examples/) is checked in CI and has
a stable `.ir` snapshot. The point of this catalog is not volume: each example
exists because it demonstrates a distinct language capability or a useful
coordination pattern.

Examples tagged `@service` intentionally idle or recur instead of completing.
All examples below check with no credentials; fixture-backed `dev` runs use the
deterministic fixture provider.

## Start Here

| Example | Why it exists |
| --- | --- |
| [`minimal-noop.whip`](../examples/minimal-noop.whip) | Smallest complete workflow: `started`, `record`, `complete`, and an output contract. |
| [`human-review.whip`](../examples/human-review.whip) | Minimal human gate: `askHuman choices [...]`, inbox creation, `when human answered ...`. |
| [`triage-flow.whip`](../examples/triage-flow.whip) | Sequential `flow`: agent step, human signoff, timeout handler, branch, terminal output/failure. |

## Core Language Patterns

| Example | Why it exists |
| --- | --- |
| [`coerce-branch.whip`](../examples/coerce-branch.whip) | Named typed model decision with a success fact and human fallback on failure. |
| [`terminal-output-union.whip`](../examples/terminal-output-union.whip) | Exhaustive `case` over an effect terminal union: completed, failed, timed out, cancelled. |
| [`incident-router.whip`](../examples/incident-router.whip) | Rich guards and dynamic routing: arrays, maps, optionals, `exists`, `in`, assertions, `AgentRef`. |
| [`scheduled-escalation.whip`](../examples/scheduled-escalation.whip) | Time as effects: `timeout`, `timer until`, `cancel`, and terminal-union handling. |
| [`exec-json-ingest.whip`](../examples/exec-json-ingest.whip) | Gated local commands with typed JSON output: `exec -> Type` and `exec -> each Type`. |
| [`event-bridge.whip`](../examples/event-bridge.whip) | External event ingress and directed notification with `event`, `when fact`, and `notify`. |
| [`reusable-review-pattern.whip`](../examples/reusable-review-pattern.whip) | Compile-time reuse with `pattern` and `apply`; no hidden runtime subroutine. |

## Coordination Recipes

| Example | Why it exists |
| --- | --- |
| [`queue-worker-with-review.whip`](../examples/queue-worker-with-review.whip) | Canonical work loop: claim queue item, run agent, typed review, finish/release/escalate. |
| [`multi-agent-bounded-concurrency.whip`](../examples/multi-agent-bounded-concurrency.whip) | Two agents with different capacities and a reviewer handoff. |
| [`circuit-breaker.whip`](../examples/circuit-breaker.whip) | Resilience pattern expressed as facts, a bounded counter, and explicit failure policy. |
| [`ralph.whip`](../examples/ralph.whip) | Tiny recurring service: agent completion feeds the next turn, guarded by capacity. |

## Showcase Workflows

| Example | Why it exists |
| --- | --- |
| [`openclaw-lite.whip`](../examples/openclaw-lite.whip) | Scheduled operations composition: heartbeat observation, planning turn, queue filing, human review. |
| [`autoresearch-lite.whip`](../examples/autoresearch-lite.whip) | Objective research loop: budgeted experiment, typed metric ingestion, keep/stop decision. |
| [`gastown-lite.whip`](../examples/gastown-lite.whip) | Coding-agent coordination: queue filing, workspace lease, agent work, typed review, ledger record. |

## Runtime Operations

| Example | Why it exists |
| --- | --- |
| [`revision-ticket-v1.whip`](../examples/revision-ticket-v1.whip) / [`revision-ticket-v2.whip`](../examples/revision-ticket-v2.whip) | Paired source files for `whip revise`: compatible in-flight workflow evolution. |
| [`revision-parent-child.whip`](../examples/revision-parent-child.whip) | Parent/child workflow invocation and explicit success/failure payload mapping. |
| [`revision-validation-approval.whip`](../examples/revision-validation-approval.whip) | Operator-safe revision proposal: child drafts candidate, human reviews, activation stays outside source. |
| [`revision-running-cancel.whip`](../examples/revision-running-cancel.whip) | Revision behavior around already-running provider work. |
| [`revision-repair-planner.whip`](../examples/revision-repair-planner.whip) | Agent-drafted repair proposal that returns a dry-run command rather than self-activating. |

## Test Fixtures

These remain in `examples/` because runtime/report tests use them, but they are
not part of the learning path:

| Fixture | Purpose |
| --- | --- |
| [`provider-language-e2e.whip`](../examples/provider-language-e2e.whip) | Acceptance/report fixture for multi-provider routing, tagged assertions, and BAML evidence redaction. |
| [`provider-language-e2e.accept.json`](../examples/provider-language-e2e.accept.json), [`human-review.accept.json`](../examples/human-review.accept.json) | Machine-checked expectations for `whip accept`. |
| [`plugin-memory.whip`](../examples/plugin-memory.whip) | Plugin outbox/runtime fixture; memory is not a curated example until the memory system is real. |
| [`queue-gated-smoke.whip`](../examples/queue-gated-smoke.whip) | Narrow queue dependency smoke test; the copyable pattern is `queue-worker-with-review.whip`. |

## Running Them

```sh
whip check examples/triage-flow.whip

whip --store .whipplescript/examples.sqlite \
  dev examples/triage-flow.whip \
  --provider fixture --until idle --json
```

Useful variations:

```sh
# stream progress as NDJSON
whip --store .whipplescript/examples.sqlite \
  dev examples/openclaw-lite.whip \
  --provider fixture --until idle --stream ndjson

# run an acceptance fixture end to end
whip --store .whipplescript/accept.sqlite --json \
  accept examples/provider-language-e2e.accept.json

# inspect the lowered rules for a flow or pattern expansion
whip check examples/reusable-review-pattern.whip
```

## Reading Order

1. `minimal-noop.whip`
2. `human-review.whip`
3. `triage-flow.whip`
4. `queue-worker-with-review.whip`
5. `incident-router.whip`
6. `scheduled-escalation.whip`
7. `openclaw-lite.whip`
8. `autoresearch-lite.whip` or `gastown-lite.whip`
