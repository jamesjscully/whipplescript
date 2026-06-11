# Runtime & operations

What happens after a workflow compiles: where state lives, how instances and
effects move through their lifecycles, how failures surface, and how to
operate running instances.

## Runtime loops

The runtime separates deterministic progress from provider execution:

| Loop | Responsibility |
| --- | --- |
| starter | Create an instance and seed input facts/events. |
| stepper | Evaluate ready rules; commit facts, effects, and dependencies atomically. |
| worker | Claim ready effects, run providers under leases, record completions. |
| projection | Maintain current views (facts, effects, status, traces) over the event log. |

`step` never executes providers. `worker` never invents policy. `dev`
composes the loops for local convenience without changing the boundaries.

## The store

State lives in a SQLite file, by default `.whipplescript/store.sqlite`.
Select one explicitly per environment with `--store <path>` or the
`WHIPPLESCRIPT_STORE` environment variable; every command that touches an
instance must use the store that created it.

The store holds program versions, instances, the append-only event log, and
projections over it: facts, effects and their dependencies, provider runs,
leases, workflow invocations, inbox items, evidence, artifacts, and
registered capabilities/profiles/plugins. The event log is the source of
truth; everything else can be rebuilt from it.

## Instance lifecycle

```text
running -> paused -> running        (pause / resume)
running -> completed                (a rule ran `complete`)
running -> failed                   (a rule ran `fail`)
running -> cancelled                (operator ran `cancel`)
```

`completed`, `failed`, and `cancelled` are terminal: the instance rejects
further rule commits and lifecycle transitions. Cancellation is an operator
action, distinct from workflow `fail` — there is no source syntax for it.

## Effects, runs, and leases

An *effect* is a durable request for external work. A *run* is one provider
attempt at an effect. A *lease* protects a running attempt from being claimed
by a second worker.

```text
queued -> running -> completed | failed | timed_out | cancelled
queued -> blocked_by_dependency | blocked_by_capacity
        | blocked_by_capability | blocked_by_profile
```

A provider failure is recorded as effect/run state, events, and evidence — it
does **not** fail the workflow. Rules decide policy: retry, escalate, ignore,
or execute `fail`. This is the central operational property; an instance with
ten failed provider runs is still `running` until a rule or operator says
otherwise.

| Outcome | Recorded as | Instance state |
| --- | --- | --- |
| Provider run failed or timed out | effect/run terminal state, events, evidence | unchanged until rules react |
| Rule executed `fail ... { ... }` | `workflow.failed` event, terminal payload | `failed` |
| Operator ran `cancel` | transition event | `cancelled` |

Provider adapters capture real diagnostics — exit codes, stderr excerpts,
SDK errors, timeout reasons, artifact paths, correlation ids — as evidence.
Secrets are never persisted.

## Inspecting an instance

```sh
whip --store <store> instances
whip --store <store> status      <instance>
whip --store <store> log         <instance>
whip --store <store> facts       <instance>
whip --store <store> effects     <instance>
whip --store <store> runs        <instance>
whip --store <store> diagnostics <instance>
whip --store <store> --json evidence <instance>
whip --store <store> --json trace <instance> --check
```

When an effect did not run, work down this list in order: `effects` (status
and `policy_block_reason`), `runs` (provider attempts), `diagnostics`,
`evidence`, then `trace --check` for lifecycle conformance. Add `--json` to
any of these for machine-readable output.

## Operating an instance

```sh
whip --store <store> pause  <instance>     # block new provider starts
whip --store <store> resume <instance>
whip --store <store> cancel <instance>     # terminal
whip --store <store> retry  <instance> <effect>
```

`retry` moves an eligible failed or timed-out effect back to `queued`.
`recover` reconciles interrupted native provider runs from persisted
evidence after a crash; see [providers & plugins](providers.md).

## Child workflows

`invoke Workflow { ... } as child` creates a durable child instance with its
own event log. The parent's invocation effect resolves from the child's
terminal state — declared output payload on completion, declared failure
payload on failure. The parent never reads child-local facts directly.

## Revising a running instance

`whip revise` switches a non-terminal instance to a new program version
after compatibility checks. Always preview first:

```sh
whip --store <store> revise <instance> candidate.whip --root Workflow --dry-run
whip --store <store> revise <instance> candidate.whip --root Workflow --cancel keep
```

Revision is append-only: the runtime records an activation event, a new
revision epoch, and diagnostics. Existing effects keep their original
program version; future commits use the new one. The `--cancel` policy
controls what happens to old-version work:

| Policy | Effect on old-version work |
| --- | --- |
| `keep` | Stays claimable and runnable. |
| `queued` | Queued/blocked/claimable effects are terminally cancelled. |
| `running` | As `queued`, plus cancellation is requested for running effects. |

A `running` cancellation is a request, not a result — the provider still
records the terminal outcome through the normal effect lifecycle.

Revision is limited to the same root workflow in v0. Root changes and
schema-breaking fact migrations are tracked in
[`spec/workflow-revision-followups-tracker.md`](../spec/workflow-revision-followups-tracker.md).

## Capturing an incident

Before repairing or deleting runtime state, capture the JSON views:

```sh
for cmd in status log facts effects runs evidence; do
  whip --store <store> --json $cmd <instance> > incident-$cmd.json
done
whip --store <store> --json trace <instance> --check > incident-trace.json
```

For provider issues, also preserve artifacts and the provider configuration
names involved — never credential values.
