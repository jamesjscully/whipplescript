# Workflow Effects

Status: design proposal

Effects are the only way a workflow changes durable data or the outside
world. They are typed, validated, logged, and capability-checked.

This document separates effect categories so the runtime transaction model is
not ambiguous.

## Effect Categories

Every effect belongs to exactly one category:

```text
data
sync_value
async_invocation
message
human_obligation
event
```

The category determines transaction timing, output shape, and model-generation
semantics.

### 1. Data Effects

Data effects mutate workflow-local durable `data`.

Initial statement:

```text
assign
```

Data effects are deterministic and transaction-local. They are applied before
the transition commit and are replayable from the transition log.

Examples:

```armature
assign data.classification = classification
assign data.seenRuns = data.seenRuns.append(run.id)
assign data.activeItems = data.activeItems.remove(itemId)
```

Supported operations for the first implementation:

```text
assignment
list append
list remove
map insert/remove, if declared by schema
```

All target paths must be declared in the workflow `data` schema.

### 2. Synchronous Value Effects

Synchronous value effects produce a value needed by the same transition.

Initial effects:

```text
coerce
capability value calls such as plan.snapshot()
adapter summaries, if declared
```

These effects run before the transition commits. The runtime records their
inputs and outputs durably. `coerce` calls additionally write `coerce_calls`
records so model decisions are replay-safe and visible in status.

If a synchronous value effect fails, no later steps in the transition run. The
implemented v0 runtime marks the triggering event failed, discards tentative
state changes, writes durable diagnostics, and surfaces the failure through
`status`/`overview` current blockers while retaining it in historical recent
failures. Explicit failure transitions are future workflow syntax; there is no
hidden built-in blocked state in v0.

`coerce` is backed by BAML HTTP over generated `baml_src` artifacts. Adapter
value calls are backed by declared capabilities.

### 3. Native Agent Invocation Effects

Native agent invocation effects start local harness-managed agent work and
return an invocation id.

Initial effect:

```text
start
```

For local agents, `start` commits the workflow transition and inserts an
`agent_invocations` row in the same SQLite transaction. The harness later claims
that row, runs the configured provider, records artifacts, and enqueues a typed
completion event. For explicitly adapter-backed agents, `start` may dispatch
through a manifest-backed adapter after the transition commit.

The runtime must record:

```text
target
input
idempotency key
invocation id, if accepted
status
claim/provider metadata, when available
capability decision, for adapter-backed starts
```

### 4. Message Effects

Message effects deliver information to an existing target.

Initial effect:

```text
send
```

For local agents, `send` inserts an `agent_messages` row in the same transaction
as the transition that produced it. The harness or provider adapter later
delivers the message or records a visible failure. Adapter-backed sends may
dispatch through a manifest-backed adapter after the transition commit.

### 5. Human Obligation Effects

Human obligation effects create visible work for a person.

Initial effect:

```text
askHuman
```

`askHuman` is asynchronous. It creates or updates a durable human-review item
with an idempotency key. Human response arrives later as a typed
`humanReview.responded` event. The built-in JSON review-file bridge uses:

```text
{ reviewId string, decision string, response string? }
```

### 6. Event Effects

Event effects publish a typed event to a workflow event queue.

Initial effect:

```text
raise
```

`raise` appends to the durable event queue after transition commit. Raised events
must conform to a declared event schema.

### 7. Timer Effects

Timer effects are reserved for a later grammar/runtime slice. The implemented
v0 source surface does not include `sleep` or `after`; use explicit external
observations such as an `idle` event for recurring supervisor loops.

### 8. Terminal Effects

Terminal effects are reserved for a later grammar/runtime slice. Implemented v0
workflows enter terminal states with `goto` to a state that contains `final`.

## Built-In Versus Adapter Effects

Built-in effects:

```text
assign
raise
start, for local harness agents
send, for local harness agents
```

Adapter-backed effects:

```text
capability value calls such as plan.snapshot
capability mutation calls such as plan.markDone
askHuman
start/send only for explicitly adapter-backed agents
```

Executor-backed effects:

```text
coerce
```

Adapter-backed effects still have schemas and model-generation semantics. A
workflow may use them only when an adapter declares support and policy permits
the requested capability.

Executor-backed `coerce` has the same schema, policy, idempotency, and logging
requirements, but it is executed through the BAML HTTP coerce executor rather
than an adapter manifest effect.

## Effect Schema Shape

Every effect schema is described by:

```json
{
  "name": "start",
  "category": "async_invocation",
  "input": {"type": "json"},
  "output": {"type": "json"},
  "required_capabilities": ["agent.worker.start"],
  "idempotent": true,
  "timeout_ms_default": 600000,
  "failure_categories": ["blocked_by_capability", "adapter_failure", "timeout"]
}
```

For adapter-dispatched effects, the manifest `input` schema describes the effect
request's `args` envelope. This includes routing fields inserted by the
language:

- `send director "hello"` dispatches `{agent: "director", message: "hello"}`
- `start worker input` dispatches `{agent: "worker", input: <value>}`
- `askHuman reason` dispatches `{reason: <string>}`
- statement-style capability calls such as `plan.markDone(id)` dispatch
  `{capability: "plan", operation: "markDone", call_args: [<value>]}`

If an optional authored argument is absent, its key is omitted from the request
envelope.
Static validation rejects adapter-backed step effects whose request envelope
cannot satisfy the manifest input schema. Native local `start` and `send`
validate against the declared agent target and the built-in agent ledger
contract instead of requiring an adapter manifest. When the workflow expression
has a known schema, including a manifest-declared capability output schema,
validation uses that schema for the corresponding envelope field instead of
treating it as untyped JSON. The runtime revalidates the concrete JSON values
before dispatch or native ledger insertion.
When an adapter dispatches an effect, the accepted outcome records the
manifest's `required_capabilities`. Status projections prefer the outcome's
capability list and fall back to the intended effect request, so operators can
see which authority each recent effect required.
Adapter manifests are validated before use. Manifest-local type refs must
resolve through the manifest's `types` map. Required capability names, failure
categories, and nondeterministic model values must be non-empty exact tokens
with no whitespace/control characters and no duplicates in their local list.
Post-commit effects must be idempotent.
Loaded adapter event schemas are accepted at event intake after
workflow-declared schemas, so adapters can declare the payload shape for events
they deliver into the durable queue. If both the workflow and adapter manifest
declare the same event, static validation requires the schemas to match after
resolving local type refs.

## Capability Effects

External state, filesystem edits, database updates, and approved scripts are not
built into the language as ambient APIs. They are exposed through declared
capabilities.

Example source:

```armature
capability plan = adapter("implementationPlan")

let planText = plan.snapshot()
plan.markCompleted(classification.workItemId)
```

The adapter owns operation semantics, but it must advertise:

- operation names
- input schema
- output schema
- required capabilities
- idempotency behavior
- conflict behavior
- model abstraction

When manifests are supplied, static validation uses capability value-call output
schemas in surrounding expressions. For example, if `plan.count()` is declared
to return `int`, `send director plan.count()` is rejected because `send`
requires a string message.
Capability value-call inputs are checked as function inputs: zero-argument calls
use an empty record schema, one-argument calls use that argument's schema, and
multi-argument calls use a positional list schema. Statement-style capability
calls keep the request-envelope schema described above.

For file-backed resources, updates must be protected by a lock or atomic
compare-and-write. If the resource changed unexpectedly, the effect fails with a
conflict category and the workflow blocks or follows an explicit handler.

## Transaction Model

A transition has two stages:

```text
prepare
commit_and_dispatch
```

Prepare:

- evaluate guard
- run synchronous value effects
- apply data effects to a tentative state
- validate asynchronous effects
- build intended effect records

Commit and dispatch:

- append transition record
- persist new workflow state
- append intended asynchronous effect records, including the stable
  `idempotency_key`
- dispatch asynchronous effects idempotently
- append effect outcome records

If prepare fails, the current state is unchanged and a failure record is written.
If dispatch fails after commit, the transition remains committed and the effect
failure is visible in workflow status.

## Idempotency Keys

Idempotency keys must be stable across crash recovery:

```text
workflow_id/state/event_id/transition_attempt/step_index/effect_name
```

Adapters must either:

- return the same outcome for repeated idempotency keys, or
- declare that they are not idempotent, in which case the validator rejects them
  for effects that can be replayed after commit.

The durable effect log stores each effect's `idempotency_key`, and status JSON
exposes it in `recent_effects[]`. This gives operators and adapters a stable
repair/reconciliation handle without reading interpreter internals.

## Failure Categories

Minimum categories:

```text
blocked_by_contract
blocked_by_capability
blocked_by_validation
adapter_failure
baml_parse_failure
external_invocation_failed
resource_conflict
timeout
internal_error
```

Default behavior is fail visible with durable diagnostics unless the workflow
declares an explicit failure handler or an authored blocked/human-review path.

## Circuit Breakers

The first circuit breakers should be runtime-level, not clever workflow logic:

```text
max_consecutive_failures
max_effect_retries
max_baml_parse_failures
max_starts_per_window
```

Circuit breakers are not implemented in v0. When added, they should produce a
clear durable status reason instead of relying on hidden workflow state.
