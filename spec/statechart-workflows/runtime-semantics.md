# Statechart Workflow Runtime Semantics

Status: design sketch

The runtime is a trusted durable interpreter for validated workflow IR. It does
not execute user-authored TypeScript, shell, or arbitrary host-language code.

## Runtime Boundary

The workflow interpreter may perform only actions declared by the workflow IR
and allowed by the active contracts.

It may call:

- un-tie agent/session APIs
- generated BAML functions for `coerce`
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
timers
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
commit transition
dispatch asynchronous effects idempotently
record effect results
```

Effects must be idempotent or have durable idempotency keys.
Detailed effect semantics are defined in [effects.md](effects.md).

## Event Processing

Events enter the interpreter from adapters, timers, workflow `raise` effects,
compatibility bridges, or explicit user actions.

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

## Actions And Effects

Actions are declared in the statechart. Effects are the runtime operations
produced by those actions.

For example:

```armature
start worker {
  workItem selection.workItemId
}
```

compiles to an effect like:

```json
{
  "type": "start",
  "agent": "worker",
  "input": {
    "work_item": "impl-017"
  },
  "idempotency_key": "workflow/spec-implementation/state/selecting/event/evt-123/action/0"
}
```

Before dispatch, the runtime verifies:

- the agent exists
- the action is allowed for the agent
- the workflow is within concurrency limits
- the requested input matches the agent contract
- the target thread/session sandbox can satisfy the requested capability set

If validation fails, the transition is blocked and the workflow enters a
diagnostic state or creates a human-review obligation.

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

The runtime records:

```text
coerce function name
input payload
model/provider metadata
raw response
parsed structured output
validation errors, if any
```

`coerce` is a synchronous value effect. If a BAML call fails parsing or schema
validation, no later steps in the transition run. The statechart must have an
explicit failure transition or the workflow enters a built-in blocked state.

## External Work

The user-facing primitive for asynchronous external work is `start`.

`start` begins work through a declared target:

```text
agent
adapter
legacy task, if compatibility is enabled
```

The `start` effect completes when the external invocation is accepted. The
result of that external work arrives later as a typed event. The interpreter
must record the invocation id before returning to the statechart.

`start` commits the transition before dispatch. Dispatch is reconciled by
idempotency key after crashes.

The transition commit is atomic over event status, durable state, transition
logs, and intended effect logs. Adapter dispatch and effect outcome logs happen
after that commit. If the process crashes after an event is dequeued but before
the transition commit, startup recovery requeues the `processing` event and
preserves its incremented attempt count.

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

Accepted `start` outcomes still count as active invocations until a matching
processed `finished` event retires them.

## Timers

Timers are durable. A timer declaration records:

```text
timer name
fire_at
correlation scope
repeat policy
```

Timer events are ordinary events. The runtime must tolerate missed process
wakeups by firing overdue timers when the interpreter resumes.

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
In the current runtime skeleton, this is enforced at `start` dispatch time:
before a `start` effect is dispatched, the runtime projects active invocations
from durable successful `start` effects minus processed `finished` events. If
the target agent is already at its declared `maxActive`, the `start` effect is
recorded as a durable failed effect and is not dispatched.

Effect projections use the latest durable outcome for each `effect_id`.
Reconciliation may append multiple outcome records for one effect, but active
invocation counts must count a `start` effect at most once. If a later outcome
marks that same effect failed or rejected, it no longer contributes to the
active count.

The v0 completion convention is explicit: bounded `start` workflows must
declare a `finished` event with required `name string` and must process at least
one `finished` handler. The processed `finished.name` value identifies the
agent by prefix, for example `worker-01` decrements the active count for
`worker`. If agent names overlap, the runtime uses the longest matching started
agent prefix, so `worker-team-01` is attributed to `worker-team`, not `worker`.

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
resource_conflict
timeout
internal_error
```

Each category must have a default policy:

- retry if safe and bounded
- transition to blocked
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
latest transition
pending events
blocked reason
recent failures
next timers
latest coerce decisions
```

This is a core part of the product. If the user cannot see why the workflow is
waiting, the workflow system has failed its purpose.

Status is a projection over durable workflow records. It must not call adapters
for hidden live data. Adapter observations must be recorded first as events or
effect outcomes.
