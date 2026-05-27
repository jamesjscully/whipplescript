# Statechart Workflow Runtime Semantics

Status: design sketch

The runtime is a trusted durable interpreter for validated workflow IR. It does
not execute user-authored TypeScript, shell, or arbitrary host-language code.

## Runtime Boundary

The workflow interpreter may perform only actions declared by the workflow IR
and allowed by the active contracts.

It may call:

- the native local agent harness ledger
- selected BAML backends for `coerce`
- allowlisted adapter actions
- legacy Armature APIs only through explicit compatibility adapters, if used

It must not provide ambient access to:

- shell execution
- filesystem writes outside declared state/artifact scopes
- network access outside declared adapters
- package imports
- process environment beyond declared variables

## State

Each workflow instance has durable state:

```text
workflow_id
workflow_version
current_state
data
event_cursor
seen_run_ids
active_invocations
last_transition_at
failure_count
```

The runtime writes state transactionally around transitions. The effect category
determines whether work happens during prepare or after commit:

```text
load workflow state
load next admitted event
evaluate guards
run synchronous value effects
apply deterministic data effects to tentative state
validate asynchronous effects
record intended effects
record native agent invocations/messages
commit transition
dispatch explicitly adapter-backed asynchronous effects idempotently
record effect results
```

Effects must be idempotent or have durable idempotency keys.
Detailed effect semantics are defined in [effects.md](effects.md).

## Event Processing

Events enter the interpreter from the native harness, adapters, workflow `raise`
effects, compatibility bridges, or explicit user actions.

Every event has:

```text
event_id
event_type
payload
source
occurred_at
correlation_id
```

The interpreter processes events according to the current state. If no handler
matches, the event is recorded as ignored with a reason. It is not silently
dropped.

Queue ordering, statuses, retry/recovery, fanout, dedupe, and retention are
defined in [event-queue.md](event-queue.md).

## Guards

Guards are pure expressions over:

- current workflow data
- current event
- declared runtime observations
- coerce output values already materialized in the current transition or data

Guards cannot perform effects. They cannot call agents. They cannot inspect
undeclared files. They cannot invoke arbitrary code.

The expression language is intentionally small. It supports orchestration-grade
field access, matching, boolean guards, small collection helpers, string
interpolation, time/duration helpers, and typed value calls. It does not support
loops, lambdas, user-defined functions, general map/filter/reduce, numeric
libraries, regex, or inline multimodal manipulation. The exact primitive set is
defined in [expression-primitives.md](expression-primitives.md).

## Actions And Effects

Actions are declared in the statechart. Effects are the runtime operations
produced by those actions.

For example:

```armature
start worker {
  workItemId selection.workItemId
}
```

compiles to an effect like:

```json
{
  "type": "start",
  "agent": "worker",
  "input": {
    "workItemId": "impl-017"
  },
  "idempotency_key": "workflow/spec-implementation/state/selecting/event/evt-123/action/0"
}
```

Before commit, the runtime verifies:

- the agent exists
- the action is allowed for the agent
- the workflow is within concurrency limits
- the requested input matches the agent contract
- the target thread/session sandbox can satisfy the requested capability set

If validation fails before commit, the transition is not committed. The runtime
marks the triggering event failed, records durable diagnostics, and leaves the
workflow in its pre-event state. If a workflow wants a diagnostic state or
human-review obligation, it must model that path explicitly.

After a handled event, entry actions, and `always` transitions have reached a
stable state, the runtime evaluates expression invariants over the resulting
workflow data. If an expression invariant is false or does not evaluate to a
boolean, the transition fails and the interpreter rolls back to the pre-event
state before any durable state save. The queued event is marked failed with the
invariant error, and a diagnostic log record is appended. Built-in named
invariants remain validation/model obligations unless they are also represented
as expression invariants in IR.

## Coerce Calls

The user-facing primitive for typed model interpretation is `coerce`: convert an
input value into a declared output schema through a named coerce function that
lowers to BAML.

Both `coerce classifyRun(summary)` and `classifyRun(summary)` lower to the same
effect when `classifyRun` resolves to a coerce declaration. The explicit form is
preferred in examples where model-dependent control flow should be obvious.

`coerce` calls are deterministic from the workflow runtime's point of view once
the model response is recorded.

The selected v1 target backend is generated BAML client execution over
stdin/stdout. This is the default when real `coerce` execution is needed because
it does not require the coding agent sandbox to open a local listening socket. A
supplied `--baml-url` tells Armature to use an externally managed BAML HTTP
endpoint instead. Brokered mode records durable coerce requests for a trusted
out-of-sandbox service to complete. Every backend uses named JSON arguments
derived from the `coerce` declaration's parameter names.

The runtime records:

```text
coerce function name
named input payload
idempotency key
BAML backend mode and runner/service metadata
BAML source artifact hash
model/provider metadata
raw response
parsed structured output
validation errors, if any
```

`coerce` is a synchronous value effect. If a BAML call fails parsing or schema
validation, no later steps in the transition run. In implemented v0 semantics,
the triggering event is marked failed, tentative state changes are discarded,
and durable diagnostics plus coerce failure records make the failure visible in
`status`, `overview`, `events`, and `log`. Explicit failure transitions are
future syntax; v0 does not create a hidden built-in blocked state.

Successful coerce results are reused by idempotency key during replay. The
runtime must not silently call BAML again for the same committed transition and
produce a different branch decision.

## Agent Work

The user-facing primitive for asynchronous agent work is `start`.

For local agents, `start` creates a durable invocation in the native agent
ledger. For explicitly adapter-backed agents, `start` may still call a declared
adapter capability.

`start` begins work through a declared target:

```text
local agent harness
explicit adapter-backed agent
legacy task, if compatibility is enabled
```

The local `start` effect completes when the invocation row is recorded. The
result of that work arrives later as a typed event written by the harness. The
interpreter must record the invocation id before the transition is committed.

Local `start` commits the transition, effect log, and invocation row in one
SQLite transaction. Adapter-backed `start` records intended effects durably and
uses idempotency keys to reconcile dispatch after crashes.

The transition commit is atomic over event status, durable state, transition
logs, intended effect logs, and native invocation/message rows. Adapter dispatch
and adapter effect outcome logs happen after that commit. If the process
crashes after an event is dequeued but before the transition commit, startup
recovery requeues the `processing` event and preserves its incremented attempt
count.

Post-commit effect failures do not unwind a processed transition. The runtime
records an intended effect followed by a failed effect outcome. For `raise`, a
malformed raised-event request, or a storage failure while enqueueing the raised
event, is treated as a failed effect and no follow-up event is enqueued.

Effect log status is derived from the adapter outcome, not merely from whether
the adapter call returned without a transport error:

```text
accepted  -> dispatched
succeeded -> succeeded
rejected  -> failed
failed    -> failed
```

Queued, claimed, and running native invocations count as active until a matching
validated completion and processed `finished` event retire them. Adapter-backed
`start` outcomes follow the same logical convention, but the adapter must expose
enough durable identity for reconciliation.

## Timers

Timers are reserved for a later runtime slice. The implemented v0 runtime does
not schedule durable timer events; recurring loops should be driven by explicit
external observations, for example an `idle` event from a supervisor.

## Concurrency

The interpreter processes one transition per workflow instance at a time.
Parallel agent work happens outside the interpreter as external invocations.

Concurrency is represented explicitly:

```text
active_invocations[agent]
max_active(agent)
work_item.status
```

The interpreter must reject transitions that would exceed declared bounds.
The runtime enforces this before creating a native invocation or dispatching an
adapter-backed start. It projects active invocations from native
`agent_invocations` rows plus compatible adapter-backed records, minus valid
processed completions. If the target agent is already at its declared
`maxActive`, the `start` effect is recorded as a durable failed effect and no
new invocation is created.

Effect projections use the latest durable outcome for each `effect_id`.
Reconciliation may append multiple outcome records for one effect, but active
invocation counts must count a `start` effect at most once. Native invocation
idempotency keys prevent replay from creating duplicate active work.

The v0 completion convention is explicit: bounded `start` workflows must
declare a `finished` event with required `name string` and must process at least
one `finished` handler. The processed `finished.name` value identifies the
agent by prefix, for example `worker-01` decrements the active count for
`worker`. If agent names overlap, the runtime uses the longest matching started
agent prefix, so `worker-team-01` is attributed to `worker-team`, not `worker`.
The native harness uses the standard completion payload
`{id string, name string, status string, summary string, exitCode int?}`.
Provider stdout/stderr are stored as artifacts referenced from the invocation
row, not embedded in the workflow event payload.

## Failure Semantics

Failures must become durable state. A failed action cannot disappear into logs.

Minimum failure categories:

```text
blocked_by_contract
blocked_by_capability
blocked_by_validation
adapter_failure
baml_parse_failure
external_invocation_failed
completion_schema_mismatch
resource_conflict
timeout
internal_error
```

Each category must have a default policy:

- retry if safe and bounded
- fail visibly with a durable status/log reason
- create human review
- fail workflow

The workflow may override defaults only with explicit transitions. Circuit
breakers such as maximum consecutive failures and maximum starts per window are
defined in [effects.md](effects.md).

## Status View

Statechart workflows should expose a status view with:

```text
workflow name
current state
active invocations
queued/running harness invocations
latest transition
pending events
blocked reason
current effect failures
current blockers
recent failures history
latest coerce decisions
current coerce failure
latest coerce failures history
workflow data snapshot or redacted summary
```

This is a core part of the product. If the user cannot see why the workflow is
waiting, the workflow system has failed its purpose.

`blocked reason` is not a hidden runtime state. It is reserved for explicit
durable blockers produced by policy/runtime failure handling or by
workflow-authored blocked states.

Current blockers describe unresolved failed/dead-lettered events and policy
denials. Current effect failures describe failed effects from the latest status
projection, such as a `start` rejected by `maxActive`, without necessarily
making them the waiting reason. Recent failures and latest coerce failures are
historical diagnostics; they remain visible after a retry succeeds so operators
can audit what happened without confusing old failures for the current waiting
reason. A failed coerce call is current only while the triggering event remains
failed or dead-lettered; retrying the event makes the old coerce failure
historical while the retry is queued.

Status is a projection over durable workflow records. It must not call adapters
or providers for hidden live data. Adapter and harness observations must be
recorded first as events, effect outcomes, invocation rows, completion rows, or
harness events.
