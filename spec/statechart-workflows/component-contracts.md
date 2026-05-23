# Component Contracts

Status: design proposal

The workflow architecture should stay modular. Components communicate through
typed data contracts, not direct calls into each other's internals.

## Component Graph

```text
armature-cli
  -> armature-workflow
  -> armature-engine
  -> armature-adapters
  -> armature-modelgen

armature-engine
  -> armature-workflow

armature-adapters
  -> armature-engine effect contracts
  -> armature-workflow schema types

armature-modelgen
  -> armature-workflow
```

Allowed dependencies:

```text
armature-workflow:
  no dependencies on engine, adapters, CLI, or modelgen

armature-engine:
  depends on armature-workflow IR/types
  depends on adapter traits, not adapter implementations
  owns SQLite-backed queue/log/state storage

armature-adapters:
  depends on engine effect contracts and workflow schema types
  does not depend on CLI

armature-modelgen:
  depends on workflow IR only
  does not depend on engine runtime state

armature-cli:
  composes parser, validator, engine, adapters, and model generation behind
  product commands
```

If `armature-engine` and `armature-adapters` begin to pull implementation code
from each other, shared DTOs should move into a small `armature-api` or
`armature-contracts` crate. The important boundary is that adapters depend on
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
effect args, or context patches, but those values are not "untyped" at the
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

Effect request:

```json
{
  "effect_id": "eff_01H...",
  "workflow_id": "wf_01H...",
  "transition_id": "tr_01H...",
  "effect": "start",
  "category": "async_invocation",
  "target": "worker",
  "args": {},
  "idempotency_key": "wf/spec/selecting/evt_123/step_0",
  "required_capabilities": ["adapter.untie.start"],
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

Rules:

- adapters must treat `idempotency_key` as stable and replay-safe
- adapter outcomes must be schema-validated before updating workflow state
- async effects may return only acceptance/outcome metadata; external work
  completes through later events
- synchronous value effects return typed output during transition prepare
- built-in event/timer/terminal effects are runtime-owned; adapter manifests
  authorize adapter-backed effects such as `start`, `send`, `askHuman`,
  `coerce`, and capability operations

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
      "required_capabilities": ["adapter.untie.start"],
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
  the user-authored expression argument. For example, `send director "hello"`
  dispatches args shaped like `{agent: string, message: string}`, `start worker
  input` dispatches `{agent: string, input: json}`, `askHuman reason`
  dispatches `{reason: string}`, and statement-style capability calls dispatch
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

Manifests can be checked without a workflow:

```text
armature validate-adapter adapter.json --json
armature validate-adapter adapter-a.json adapter-b.json --json
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
- denied policy decisions are durable blocked records
- every policy decision is visible in diagnostics when it blocks execution

## Transition Log Contract

Transition record:

```json
{
  "transition_id": "tr_01H...",
  "workflow_id": "wf_01H...",
  "from_state": "selecting",
  "to_state": "supervising",
  "event_id": "evt_01H...",
  "guard": "next.kind == 'StartWorker'",
  "context_patch": [],
  "sync_effects": [],
  "async_effects": ["eff_01H..."],
  "diagnostics": [],
  "created_at": "2026-05-22T18:00:02Z"
}
```

Rules:

- transition records are append-only
- materialized current state must be reconstructable from logs plus snapshots
- context patches are typed and target declared context paths
- records are persisted in SQLite from the first runtime implementation

## Status Projection Contract

Status is a read-only projection:

```json
{
  "workflow_id": "wf_01H...",
  "workflow_name": "spec-implementation",
  "state": "supervising",
  "blocked": null,
  "pending_events": 2,
  "active_invocations": [
    {"agent": "worker", "count": 3, "max": 4}
  ],
  "latest_transition": "tr_01H...",
  "latest_decisions": [],
  "recent_failures": [],
  "next_timers": []
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
