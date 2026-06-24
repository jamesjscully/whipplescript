# Examples

Every curated example in [`examples/`](https://github.com/jamesjscully/whipplescript/tree/main/examples/) is checked in CI and has
a stable `.ir` snapshot. The point of this catalog is not volume: each example
exists because it demonstrates a distinct language capability or a useful
coordination pattern.

Examples tagged `@service` intentionally idle or recur instead of completing.
All examples below check with no credentials; fixture-backed `dev` runs use the
deterministic fixture provider. The catalog commands are verified by
`scripts/check-docs-examples.sh`.

## Start Here

| Example | Check command | Why it exists |
| --- | --- | --- |
| [`minimal-noop.whip`](https://github.com/jamesjscully/whipplescript/blob/main/examples/minimal-noop.whip) | `whip check examples/minimal-noop.whip` | Smallest complete workflow: `started`, `record`, `complete`, and an output contract. |
| [`human-review.whip`](https://github.com/jamesjscully/whipplescript/blob/main/examples/human-review.whip) | `whip check examples/human-review.whip` | Minimal human gate: `askHuman choices [...]`, inbox creation, `when human answered ...`. |
| [`triage-flow.whip`](https://github.com/jamesjscully/whipplescript/blob/main/examples/triage-flow.whip) | `whip check examples/triage-flow.whip` | Sequential `flow`: agent step, human signoff, timeout handler, branch, terminal output/failure. |

## Core Language Patterns

| Example | Check command | Why it exists |
| --- | --- | --- |
| [`coerce-branch.whip`](https://github.com/jamesjscully/whipplescript/blob/main/examples/coerce-branch.whip) | `whip check examples/coerce-branch.whip` | Named typed model decision with a success fact and human fallback on failure. |
| [`terminal-output-union.whip`](https://github.com/jamesjscully/whipplescript/blob/main/examples/terminal-output-union.whip) | `whip check examples/terminal-output-union.whip` | Exhaustive `case` over an effect terminal union: completed, failed, timed out, cancelled. |
| [`incident-router.whip`](https://github.com/jamesjscully/whipplescript/blob/main/examples/incident-router.whip) | `whip check examples/incident-router.whip` | Rich guards and dynamic routing: arrays, maps, optionals, `exists`, `in`, assertions, `AgentRef`. |
| [`scheduled-escalation.whip`](https://github.com/jamesjscully/whipplescript/blob/main/examples/scheduled-escalation.whip) | `whip check examples/scheduled-escalation.whip` | Time as effects: `timeout`, `timer until`, `cancel`, and terminal-union handling. |
| [`exec-json-ingest.whip`](https://github.com/jamesjscully/whipplescript/blob/main/examples/exec-json-ingest.whip) | `whip check examples/exec-json-ingest.whip` | Gated local commands with typed JSON output: `exec -> Type` and `exec -> each Type`. |
| [`event-bridge.whip`](https://github.com/jamesjscully/whipplescript/blob/main/examples/event-bridge.whip) | `whip check examples/event-bridge.whip` | External signal ingress (`whip signal`) and directed signal injection: `emit signal ... to <instance>` relays an acknowledgement into a live peer instance, which reacts via typed `when`. A missing target fails the effect with `target instance <id> not found`. |
| [`reusable-review-pattern.whip`](https://github.com/jamesjscully/whipplescript/blob/main/examples/reusable-review-pattern.whip) | `whip check examples/reusable-review-pattern.whip` | Compile-time reuse with `pattern` and `apply`; no hidden runtime subroutine. |
| [`messaging-demo.whip`](https://github.com/jamesjscully/whipplescript/blob/main/examples/messaging-demo.whip) | `whip check examples/messaging-demo.whip` | `std.messaging`: a `channel`, inbound `when message from <channel>` (binds the generic `Message`), and outbound `send via <channel>`. Inject inbound with `whip message`. |
| [`file-store-demo.whip`](https://github.com/jamesjscully/whipplescript/blob/main/examples/file-store-demo.whip) | `whip check examples/file-store-demo.whip` | `std.files`: a `file store` policy boundary with `allow read`/`allow write` globs; durable `write text ... mode upsert` then `read text ... as f` round-trip, completing with `f.content`. |

## Coordination Recipes

| Example | Check command | Why it exists |
| --- | --- | --- |
| [`queue-worker-with-review.whip`](https://github.com/jamesjscully/whipplescript/blob/main/examples/queue-worker-with-review.whip) | `whip check examples/queue-worker-with-review.whip` | Canonical work loop: claim queue item, run agent, typed review, finish/release/escalate. |
| [`multi-agent-bounded-concurrency.whip`](https://github.com/jamesjscully/whipplescript/blob/main/examples/multi-agent-bounded-concurrency.whip) | `whip check examples/multi-agent-bounded-concurrency.whip` | Two agents with different capacities and a reviewer handoff. |
| [`circuit-breaker.whip`](https://github.com/jamesjscully/whipplescript/blob/main/examples/circuit-breaker.whip) | `whip check examples/circuit-breaker.whip` | Resilience pattern expressed as facts, a bounded counter, and explicit failure policy. |
| [`ralph.whip`](https://github.com/jamesjscully/whipplescript/blob/main/examples/ralph.whip) | `whip check examples/ralph.whip` | Tiny recurring service: agent completion feeds the next turn, guarded by capacity. |

## Showcase Workflows

| Example | Check command | Why it exists |
| --- | --- | --- |
| [`openclaw-lite.whip`](https://github.com/jamesjscully/whipplescript/blob/main/examples/openclaw-lite.whip) | `whip check examples/openclaw-lite.whip` | Scheduled operations composition: heartbeat observation, planning turn, queue filing, human review. |
| [`autoresearch-lite.whip`](https://github.com/jamesjscully/whipplescript/blob/main/examples/autoresearch-lite.whip) | `whip check examples/autoresearch-lite.whip` | Objective research loop: budgeted experiment, typed metric ingestion, keep/stop decision. |
| [`gastown-lite.whip`](https://github.com/jamesjscully/whipplescript/blob/main/examples/gastown-lite.whip) | `whip check examples/gastown-lite.whip` | Coding-agent coordination: queue filing, workspace lease, agent work, typed review, ledger record. |

## Runtime Operations

| Example | Check command | Why it exists |
| --- | --- | --- |
| [`revision-ticket-v1.whip`](https://github.com/jamesjscully/whipplescript/blob/main/examples/revision-ticket-v1.whip) / [`revision-ticket-v2.whip`](https://github.com/jamesjscully/whipplescript/blob/main/examples/revision-ticket-v2.whip) | `whip check examples/revision-ticket-v1.whip` / `whip check examples/revision-ticket-v2.whip` | Paired source files for `whip revise`: compatible in-flight workflow evolution. |
| [`revision-parent-child.whip`](https://github.com/jamesjscully/whipplescript/blob/main/examples/revision-parent-child.whip) | `whip check examples/revision-parent-child.whip --root ParentRevisionExample` | Parent/child workflow invocation and explicit success/failure payload mapping. |
| [`revision-validation-approval.whip`](https://github.com/jamesjscully/whipplescript/blob/main/examples/revision-validation-approval.whip) | `whip check examples/revision-validation-approval.whip --root RevisionValidation` | Operator-safe revision proposal: child drafts candidate, human reviews, activation stays outside source. |
| [`revision-running-cancel.whip`](https://github.com/jamesjscully/whipplescript/blob/main/examples/revision-running-cancel.whip) | `whip check examples/revision-running-cancel.whip` | Revision behavior around already-running provider work. |
| [`revision-repair-planner.whip`](https://github.com/jamesjscully/whipplescript/blob/main/examples/revision-repair-planner.whip) | `whip check examples/revision-repair-planner.whip` | Agent-drafted repair proposal that returns a dry-run command rather than self-activating. |

## Test Fixtures

These remain in `examples/` because runtime/report tests use them, but they are
not part of the learning path:

| Fixture | Purpose |
| --- | --- |
| [`provider-language-e2e.whip`](https://github.com/jamesjscully/whipplescript/blob/main/examples/provider-language-e2e.whip) | Acceptance/report fixture for multi-provider routing, tagged assertions, and coerce evidence redaction. |
| [`provider-language-e2e.accept.json`](https://github.com/jamesjscully/whipplescript/blob/main/examples/provider-language-e2e.accept.json), [`human-review.accept.json`](https://github.com/jamesjscully/whipplescript/blob/main/examples/human-review.accept.json) | Machine-checked expectations for `whip accept`. |
| Package capability fixture | Exercises package-locked `capability.call` lowering and runtime-boundary output validation. |
| [`queue-gated-smoke.whip`](https://github.com/jamesjscully/whipplescript/blob/main/examples/queue-gated-smoke.whip) | Narrow queue dependency smoke test; the copyable pattern is `queue-worker-with-review.whip`. |

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
