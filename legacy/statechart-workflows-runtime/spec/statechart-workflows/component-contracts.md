# Component Contracts

Status: design proposal

The workflow architecture should stay modular. Components communicate through
typed data contracts, not direct calls into each other's internals.

## Component Graph

```text
whipplescript-cli
  -> whipplescript-workflow
  -> whipplescript-engine
  -> whipplescript-adapters
  -> whipplescript-modelgen

whipplescript-engine
  -> whipplescript-workflow

whipplescript-adapters
  -> whipplescript-engine effect contracts
  -> whipplescript-workflow schema types

whipplescript-modelgen
  -> whipplescript-workflow
```

Allowed dependencies:

```text
whipplescript-workflow:
  no dependencies on engine, adapters, CLI, or modelgen

whipplescript-engine:
  depends on whipplescript-workflow IR/types
  depends on adapter traits, not adapter implementations
  owns SQLite-backed queue/log/state storage

whipplescript-adapters:
  depends on engine effect contracts and workflow schema types
  does not depend on CLI

whipplescript-modelgen:
  depends on workflow IR only
  does not depend on engine runtime state

whipplescript-cli:
  composes parser, validator, engine, adapters, and model generation behind
  product commands
```

If `whipplescript-engine` and `whipplescript-adapters` begin to pull implementation code
from each other, shared DTOs should move into a small `whipplescript-api` or
`whipplescript-contracts` crate. The important boundary is that adapters depend on
effect contracts, not interpreter internals.

## Boundary Principle

Every boundary validates typed data before accepting it:

```text
source -> IR              validate source shape and expression grammar
IR -> runtime             validate state graph, effects, schemas, policies
event queue -> runtime    validate event type and payload schema
runtime -> adapter        validate effect schema, policy, idempotency key
adapter -> runtime        validate outcome schema
runtime -> status         project durable state; no hidden adapter calls
IR -> modelgen            validate modelable subset
```

Implementation DTOs may temporarily carry `serde_json::Value` for payloads,
effect args, or durable workflow data snapshots, but those values are not "untyped" at the
boundary. Each occurrence must be paired with a schema from `WorkflowIr` or an
`AdapterManifest`, and the receiver must validate before use.

## Source Compiler Contract

Input:

```text
source_path
source_text
workspace_root
```

Output:

```json
{
  "ir": "WorkflowIr",
  "artifacts": ["BuildArtifact"],
  "diagnostics": ["Diagnostic"]
}
```

Rules:

- output IR is produced only if parsing/lowering succeeds
- diagnostics include source spans when possible
- generated BAML artifacts are build outputs, not source of truth
- compiler performs no runtime effects

## Validator Contract

Input:

```text
WorkflowIr
AdapterManifest[]
PolicyMode
PolicyDocument[]
```

Output:

```json
{
  "ok": true,
  "diagnostics": [],
  "resolved_policy": "ResolvedPolicy",
  "effect_schemas": {}
}
```

Rules:

- validation never dispatches effects
- validation resolves all known adapter effect schemas
- validation records warnings separately from errors
- validation may produce `unknown` policy outcomes in local/team modes, but
  runtime must check again before dispatch

## Event Queue Contract

Event record:

```json
{
  "event_id": "evt_01H...",
  "workflow_id": "wf_01H...",
  "event_type": "finished",
  "payload": {},
  "source": {
    "kind": "adapter",
    "name": "untie"
  },
  "occurred_at": "2026-05-22T18:00:00Z",
  "enqueued_at": "2026-05-22T18:00:01Z",
  "correlation_id": "corr_...",
  "causation_id": "transition_...",
  "dedupe_key": null,
  "status": "queued",
  "attempt_count": 0,
  "last_error": null
}
```

Rules:

- event payloads must match the declared workflow event schema
- event ids are immutable
- event status transitions are append-only or auditable
- one queued event belongs to one workflow instance

## Effect Dispatch Contract

This contract applies to effects that dispatch through an external adapter.
Native local agent `start` and `send` use the native agent harness contract
below instead of requiring an adapter manifest.

Effect request:

```json
{
  "effect_id": "eff_01H...",
  "workflow_id": "wf_01H...",
  "transition_id": "tr_01H...",
  "effect": "start",
  "category": "async_invocation",
  "target": "external-worker",
  "args": {},
  "idempotency_key": "wf/spec/selecting/evt_123/step_0",
  "required_capabilities": ["adapter.external.start"],
  "timeout_ms": 600000
}
```

Effect outcome:

```json
{
  "effect_id": "eff_01H...",
  "status": "succeeded",
  "invocation_id": "inv_01H...",
  "output": null,
  "error": null,
  "completed_at": "2026-05-22T18:00:03Z"
}
```

Durable effect log records also retain the request `idempotency_key`. The
status projection includes that key on `recent_effects[]` so retries and adapter
reconciliation can refer to the same stable handle that dispatch used.

Rules:

- adapters must treat `idempotency_key` as stable and replay-safe
- adapter outcomes must be schema-validated before updating workflow state
- async effects may return only acceptance/outcome metadata; external work
  completes through later events
- synchronous value effects return typed output during transition prepare
- built-in event effects are runtime-owned.
- native local agent `start` and `send` are runtime-owned and persist through
  the native agent ledger, not through adapter manifests.
- adapter manifests authorize explicitly external effects such as `askHuman`,
  plan/state adapter operations, external service calls, and adapter-backed
  agents when a workflow deliberately targets such an adapter.
- `coerce` uses the separate coerce executor contract rather than the
  adapter-manifest effect path. Timer and terminal effects are reserved for
  later grammar/runtime slices.

## Native Agent Harness Contract

Local agent orchestration is a first-class runtime/harness boundary. A
statechart `start` inserts an invocation into SQLite; the harness claims that
invocation, runs a provider, records artifacts, and enqueues a typed completion
event.

Invocation record:

```json
{
  "workflow_id": "implementationLoop",
  "invocation_id": "inv_01H...",
  "agent": "worker",
  "effect_id": "eff_01H...",
  "transition_id": "tr_01H...",
  "event_id": "evt_01H...",
  "idempotency_key": "implementationLoop/evt_01H/tr_01H/handler.0/start",
  "input": {"task": "W1", "message": "Implement W1"},
  "requested_profile": "repo-writer",
  "resolved_profile": null,
  "status": "queued",
  "claimed_by": null,
  "claim_expires_at": null,
  "provider": null,
  "run_dir": null,
  "stdout_path": null,
  "stderr_path": null
}
```

Harness claim request:

```json
{
  "worker_id": "host-123:pid-456",
  "lease_seconds": 300,
  "providers": ["codex", "claude", "pi", "command"]
}
```

Harness completion:

```json
{
  "workflow_id": "implementationLoop",
  "completion_id": "cmp_01H...",
  "invocation_id": "inv_01H...",
  "agent": "worker",
  "status": "succeeded",
  "summary": "Implemented W1 and tests passed.",
  "exit_code": 0,
  "event_id": "evt_01H...",
  "payload": {
    "id": "inv_01H...",
    "name": "worker",
    "status": "succeeded",
    "summary": "Implemented W1 and tests passed.",
    "exitCode": 0
  }
}
```

Rules:

- the runtime inserts invocation/message records in the same transaction as the
  transition that produced them
- source-level `profile` names are recorded as requested profile intent for
  native harness resolution, but they do not grant provider authority by
  themselves
- the harness claims queued work through SQLite transactions and leases
- the harness resolves requested profiles through harness profile policy before
  launching providers; resolution records requested profile, resolved profile,
  provider, requested authority, enforced authority, and any best-effort gaps
- provider stdout/stderr are artifact paths, not event payloads
- the harness validates the completion payload against the workflow event
  schema before enqueueing the workflow event
- completion recording and workflow-event enqueue happen in one transaction
- harness events are operational observations; statechart handlers consume only
  typed workflow events
- the JSON agent-file bridge is not part of the product contract; if retained
  temporarily, it is a fixture/debug helper with no compatibility promise

The JSON plan file bridge supports a deliberately small read surface:
`plan.snapshot()` returns the raw JSON document as text,
`plan.unfinishedItems()` returns the count of task/status entries whose status
is not `done`, and `plan.nextReadyItem()` returns the first task whose status is
missing, `todo`, `ready`, or `ready_for_implementation`, or `null` when no such
task exists. Status writes remain idempotent task/status updates keyed by work
item id.

## Coerce Executor Contract

`coerce` is a synchronous value effect backed by BAML HTTP in v1.

Request:

```json
{
  "coerce_call_id": "coerce_01H...",
  "workflow_id": "implementationLoop",
  "workflow_version": "statechart-workflow-ir/v0",
  "transition_id": "tr_01H...",
  "event_id": "evt_01H...",
  "step_path": "state:choosing/entry/0",
  "function_name": "chooseNextStep",
  "idempotency_key": "implementationLoop/statechart-workflow-ir/v0/evt_01H/0/state:choosing/entry/0/chooseNextStep",
  "args": {
    "planText": "W1 ready"
  },
  "output_schema": {"type": "ref", "name": "NextStep"},
  "backend": {
    "kind": "baml_http",
    "url": "http://127.0.0.1:2024",
    "baml_src_hash": "sha256:..."
  },
  "timeout_ms": 60000
}
```

Outcome:

```json
{
  "coerce_call_id": "coerce_01H...",
  "status": "succeeded",
  "http_status": 200,
  "parsed_output": {
    "action": "StartWorker",
    "workItemId": "W1",
    "reason": "ready",
    "message": "Implement W1"
  },
  "raw_response": {"redacted": true},
  "error": null,
  "duration_ms": 1350
}
```

Rules:

- arguments are named by `coerce` parameter names, even if the source call uses
  positional syntax
- argument values are schema-validated before the HTTP call
- parsed output is schema-validated after the HTTP call
- successful outputs are reused by idempotency key during replay
- failed attempts are durable records and may be retried only through explicit
  runtime retry policy
- the executor cannot dispatch workflow effects or mutate workflow data
- BAML HTTP internals are not modeled by formal backends

## Adapter Manifest Contract

Adapters advertise capabilities through manifests:

```json
{
  "name": "untie",
  "version": "0.1.0",
  "types": {
    "AgentFinished": {
      "type": "record",
      "fields": [
        {"name": "name", "schema": {"type": "string"}},
        {"name": "exitCode", "schema": {"type": "int"}}
      ]
    }
  },
  "effects": {
    "start": {
      "category": "async_invocation",
      "required_capabilities": ["adapter.external.start"],
      "input": {"type": "json"},
      "output": {"type": "json"},
      "idempotent": true,
      "failure_categories": ["adapter_failure", "timeout"],
      "model": {
        "kind": "nondeterministic_outcome",
        "values": ["accepted", "rejected", "failed"]
      }
    }
  },
  "events": {
    "finished": {
      "payload": {"type": "ref", "name": "AgentFinished"}
    }
  }
}
```

Rules:

- effect names are global, but adapter manifests define which adapter supports
  which effect
- policy references adapter capability names
- `required_capabilities`, `failure_categories`, and nondeterministic model
  `values` are exact non-empty tokens with no whitespace/control characters and
  no duplicates in their local list
- model generation consumes the adapter's model abstraction, not implementation
  code
- `types` are manifest-local; every schema ref in effect inputs, effect
  outputs, and event payloads must resolve inside the same manifest, and type
  refs must not form cycles
- effect `input` schemas describe the runtime request `args` envelope, not just
  the user-authored expression argument. For adapter-backed effects, `send
  director "hello"` dispatches args shaped like
  `{agent: string, message: string}`, `start worker input` dispatches
  `{agent: string, input: json}`, `askHuman reason` dispatches
  `{reason: string}`, and statement-style capability calls dispatch
  `{capability: string, operation: string, call_args: json[]}`.
  Optional authored arguments are omitted from the envelope when absent.
- workflow validation checks adapter effect categories and the static request
  args envelope before runtime dispatch. Runtime dispatch revalidates the
  concrete JSON values against the same manifest schema.
- when an authored effect argument has a known schema, including a
  manifest-declared capability output schema, that schema is used for the
  corresponding request envelope field during static validation
- post-commit effects such as `start`, `send`, and `askHuman` must be
  idempotent because they may be replayed during recovery
- duplicate effect or event names across loaded manifests are validation errors
- CLI event intake validates first against workflow-declared event schemas, then
  against loaded adapter event schemas. This lets adapter-originated events use
  manifest-local types at the boundary.
- when a workflow and loaded adapter both declare the same event, validation
  requires the schemas to match after resolving refs in their respective type
  maps

Built-in JSON human-review response event:

```json
{
  "humanReview.responded": {
    "type": "record",
    "fields": {
      "reviewId": "string",
      "decision": "string",
      "response": {"type": "optional", "inner": "string"}
    }
  }
}
```

`emit --review-file <json>` loads this event schema without also loading the
`askHuman` effect, so response intake can coexist with explicit review effect
manifests. It also appends a `responses` record to the JSON review file and
marks the matching open review as `responded` when `reviewId` matches a stored
review id.

Native agent completion event convention:

```json
{
  "finished": {
    "type": "record",
    "fields": {
      "id": "string",
      "name": "string",
      "status": "string",
      "summary": "string",
      "exitCode": {"type": "optional", "inner": "int"}
    }
  }
}
```

Bounded-start workflows still declare a compatible `finished` event in source
so active invocation accounting is statically visible. The native harness writes
provider stdout/stderr as artifacts, records a durable completion row, validates
the completion payload against the workflow event schema, and enqueues the typed
workflow event. Raw logs do not belong in the event payload.

Manifests can be checked without a workflow:

```text
whip validate-adapter adapter.json --json
whip validate-adapter adapter-a.json adapter-b.json --json
```

The multi-file form validates cross-manifest uniqueness for effect and event
names.

## Policy Contract

Policy decision:

```json
{
  "effect_id": "eff_01H...",
  "outcome": "allow_with_warning",
  "required_capabilities": ["resource.plan.write"],
  "layers": [
    {"layer": "workflow", "outcome": "allow"},
    {"layer": "workspace", "outcome": "allow"},
    {"layer": "resource", "outcome": "allow"},
    {"layer": "adapter", "outcome": "allow"},
    {"layer": "target", "outcome": "unknown"}
  ],
  "diagnostics": []
}
```

Rules:

- runtime dispatch requires an `allow` or mode-accepted `allow_with_warning`
- denied policy decisions are durable denial/failure records and appear in
  policy diagnostics or status blockers
- every policy decision is visible in diagnostics when it blocks execution

## Transition Log Contract

Transition record:

```json
{
  "type": "transition",
  "transition_id": "tr_01H...",
  "workflow_id": "wf_01H...",
  "from_state": "selecting",
  "to_state": "supervising",
  "event_type": "idle",
  "event_id": "evt_01H..."
}
```

Rules:

- transition records are append-only
- materialized current state must be reconstructable from logs plus snapshots
- data writes are type-checked before the transition is committed
- intended and terminal effect records are stored separately and linked by
  `transition_id`
- records are persisted in SQLite from the first runtime implementation

## Status Projection Contract

Status is a read-only projection:

```json
{
  "workflow_id": "wf_01H...",
  "workflow_name": "spec-implementation",
  "current_state": "supervising",
  "blocked_reason": null,
  "pending_events": 2,
  "active_invocations": [
    {"agent": "worker", "count": 3, "max": 4}
  ],
  "data_summary": {
    "seenRuns": 12,
    "lastIdleNudgeAt": "2026-05-23T10:00:00Z"
  },
  "latest_transition": "tr_01H...",
  "latest_coerce_calls": [
    {
      "function_name": "chooseNextStep",
      "status": "succeeded",
      "parsed_output": {"action": "StartWorker"}
    }
  ],
  "policy_blockers": [],
  "recent_failures": []
}
```

Rules:

- status must not call adapters for hidden live data
- status reads durable state/logs only
- if an adapter has relevant live state, it must publish it as events/effects
  first

## Modelgen Contract

Input:

```text
WorkflowIr
AdapterManifest[]
ResolvedPolicy
ModelOptions
```

Output:

```json
{
  "target": "apalache",
  "files": ["SpecImplementation.tla", "SpecImplementation.cfg"],
  "diagnostics": []
}
```

Rules:

- model generation never depends on runtime logs
- coerce functions lower to nondeterministic outputs constrained by schema
- adapter effects lower through adapter model abstractions
- unsupported constructs fail model generation explicitly
