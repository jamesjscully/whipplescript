# Operations Guide

This guide is for humans and coding agents inspecting a workflow that is
waiting, blocked, or producing unexpected effects.

## First Commands

Use the same contract context that was used to validate or run the workflow.
For explicit manifests:

```sh
armature overview workflow.armature \
  --adapter-manifest adapter.json \
  --policy policy.json \
  --json

armature status workflow.armature \
  --adapter-manifest adapter.json \
  --policy policy.json \
  --json
```

For built-in JSON file-backed plan/review adapters:

```sh
armature overview workflow.armature \
  --plan-file plan.json \
  --review-file reviews.json \
  --json
```

For native local agent work:

```sh
armature harness status workflow.armature --json
```

Then inspect durable queue and log records:

```sh
armature events workflow.armature --json
armature log workflow.armature --json
```

`status` and `overview` read only durable Armature state. They must not call
adapters, BAML, providers, agents, or backing JSON files.
The human `status` and `overview` outputs include a `waiting:` line that
summarizes the highest-priority visible reason: validation failure for
`overview`, blocked reason, policy blocker, current blocker, queued events,
active invocations, or idle/no work.
Failed effects that do not make the workflow stuck, such as an attempted
`start` rejected by `maxActive` while another invocation is already active, are
shown as current effect failures and retained in recent failure history.

## Common Stuck States

### Pending Events But No Progress

Symptoms:

- `status.pending_events > 0`
- `events` shows queued events
- repeated `run` commands do not reach the expected state

Checks:

- inspect the latest `outcome.status`
- inspect `recent_failures`
- validate the workflow with the same manifests and policy

Likely repairs:

- fix event payload shape
- add a reachable `on <event>` handler
- correct a guard that is always false
- fix a failed effect before reprocessing new work

### Failed Or Dead-Lettered Events

Symptoms:

- `armature events workflow.armature --status failed --json` returns records
- `armature events workflow.armature --status dead_lettered --json` returns
  records
- the event `last_error` explains an old payload or processing failure

Checks:

- inspect the failed event payload and `attempt_count`; plain
  `armature events --status failed` also shows durable `last_error` in text
  output when present
- validate the workflow with the same manifests and policy before retrying
- confirm any external cause has been repaired

Likely repairs:

- retry a specific event with
  `armature retry-event workflow.armature --event-id <event-id> --json`
  or use text mode to confirm `status=queued` and the new `pending_events`
  count
- if the selected store path does not exist, `retry-event` fails without
  creating an empty workflow store
- if the payload is wrong, emit a corrected typed event instead of retrying the
  bad one
- keep the original failed record visible; do not manually delete queue history

### Active Runs Never Retire

Symptoms:

- `active_invocations` stays non-zero
- idle observations do not start new work because `maxActive` is reached

Checks:

- inspect `armature harness status workflow.armature --json`
- confirm started agents have native invocation records
- confirm the harness claimed and completed the invocation
- confirm successful harness completion enqueued a `finished` event
- confirm `finished.name` starts with the declared agent name, for example
  `worker-17` for agent `worker`
- confirm a reachable `on finished` handler processes the event

Likely repairs:

- restart or run `armature harness run workflow.armature --config harness.json`
- fix provider command/configuration errors shown in harness events
- retry or repair failed/dead-lettered completion events
- align the completion event schema with the workflow's declared event
- avoid overlapping agent names unless the longest-prefix behavior is intended

### Workflow Is Blocked By Policy

Symptoms:

- `validate`, `run`, `status`, or `overview` reports capability diagnostics
- effect records show required capabilities

Checks:

- compare `required_capabilities` in adapter manifests with the policy document
- inspect denied capabilities first; denies always win
- in enterprise mode, unknown write-like capabilities become errors
- read the diagnostic `Fix:` hint as a policy review prompt, not as automatic
  permission to broaden authority

Likely repairs:

- add only the intended capability to `allowed_capabilities`
- remove a denied capability from the workflow or adapter manifest
- keep capability names exact; do not use fuzzy aliases

### Coerce Failed Or Repeated

Symptoms:

- `latest_coerce_failures` is non-empty
- workflow state did not advance after a `coerce`
- BAML HTTP errors are visible in status/log output

Checks:

- ensure `--baml-url` is reachable
- ensure policy allows `baml.coerce`, `allow_baml_network`, and the exact URL
- inspect the generated `baml_src/workflow.baml` from `build`
- confirm the BAML output matches the declared class/enum schema

Likely repairs:

- fix policy or URL configuration
- tighten the prompt or output type
- rerun after the failure is corrected; successful coerce outputs are replayed
  by idempotency key

### Human Review Is Open

Symptoms:

- recent effects include `askHuman`
- the review JSON file contains open review obligations
- workflow is waiting for an explicit response event

Checks:

- inspect the review id in the JSON review file
- emit `humanReview.responded` with `--review-file`

Example:

```sh
armature emit workflow.armature \
  --review-file reviews.json \
  --event humanReview.responded \
  --payload '{"reviewId":"review-1","decision":"approved","response":"continue"}' \
  --json
```

## Repair Principles

- Prefer source-level fixes over manual database edits.
- Prefer typed events over scripts that mutate workflow state.
- Do not add unbounded polling loops to repair stalled work.
- Keep external authority in adapter manifests and policy documents.
- If a workflow needs domain state, expose it through a capability such as
  `plan.snapshot()` or a typed event; do not ask agents to infer it from logs.
