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

### Concurrent effect execution

A worker pass executes its ready set of effects concurrently on a bounded
thread pool — the ready set is mutually independent (a dependent effect is not
claimable until its dependency's terminal), and the durable lease plus per-row
idempotency guarantee exactly-once even under concurrency (the
`AtMostOneRunExecutingEffect` invariant in `models/tla/ControlPlaneLifecycle.tla`).
This is what lets a fan-out of agent turns or `coerce` calls run in parallel
instead of one at a time, and gives `agent X { capacity N }` real runtime
meaning: a worker starts at most `N` turns of an agent at once and defers the
rest to a later pass. `WHIPPLESCRIPT_WORKER_CONCURRENCY` sets the per-pass bound
(default tracks available CPUs, capped); set it to `1` for a fully serial pass.
Each effect runs synchronously on its own pool thread (whip is not async); WAL
mode and a busy timeout let the per-effect store writes coexist safely, while
the slow provider I/O runs outside any transaction. Scale further by running
more worker processes against the same store.

## The store

State lives in a SQLite file, by default `.whipplescript/store.sqlite`.
Select one explicitly per environment with `--store <path>` or the
`WHIPPLESCRIPT_STORE` environment variable; every command that touches an
instance must use the store that created it.

The store holds program versions, instances, the append-only event log, and
projections over it: facts, effects and their dependencies, provider runs,
leases, workflow invocations, inbox items, evidence, artifacts, and
registered capabilities, profiles, packages, and providers. The event log is
the source of truth; everything else can be rebuilt from it.

## Cloud runtime (Cloudflare Durable Object) and deploy

The native SQLite runtime above is one substrate. The same workflow also runs
unchanged on the edge: the whole evaluation core executes inside a **Cloudflare
Durable Object** wasm isolate. The isolate is sans-IO — its only async
primitive is `fetch` — so every HTTP-bearing effect is a resumable step machine
that survives isolate eviction and resumes where it left off.

The DO host runs the *same* instance scheduler you run locally, over the Durable
Object's synchronous SQLite: a full port of the runtime, coordination, and
work-item stores. Timers and deadlines fire via DO **alarms** instead of worker
passes, and provider credentials come from DO **secrets** rather than the local
credential store.

The edge runtime is at feature parity with native operation:

- `file.*` effects run against a DO-owned file plane (no host filesystem).
- `whip checkpoint` / `whip restore` work as operator commands on a deployed
  instance, exactly as on a native store (see [Restoring to a prior
  point](#restoring-to-a-prior-point-checkpoint-restore)).
- An agent turn runs a real in-isolate tool set — read, write, edit, ls, find,
  grep, recall, plus work-tracker todos — against DO storage, with no
  filesystem and no subprocess.

### Deploying with `whip deploy`

`whip deploy` is a one-command edge deploy of a workflow to a Worker plus its
Durable Object.

```sh
whip deploy [--worker-dir <path>] [--name <worker>] [--dry-run] [--skip-build] [--set-secrets]
```

Use `--dry-run` to preview the build and upload without publishing,
`--skip-build` to reuse an existing wasm artifact, and `--set-secrets` to push
provider credentials into DO secrets as part of the deploy.

### The compute plane (`whip executor`)

`whip executor` runs a **Class-A** compute sidecar that the runtime calls over a
`whip-executor/1` wire.

```sh
whip executor [--bind <addr:port>]   # Class-A exec sidecar; default 127.0.0.1:8080
```

The sidecar authenticates every request with a bearer token from
`WHIP_EXECUTOR_TOKEN` (compared in constant time) and refuses non-loopback calls
that do not carry it. A Class-B per-turn container path also exists.

**Enabling the compute plane in production is a follow-on configuration step** —
it is not on by default. A workflow deploys and runs without it; the compute
plane is wired up separately when you need it.

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
queued -> blocked            (provider binding unavailable; recoverable)
```

A **blocked** effect is recoverable, not terminal: a later worker pass runs it
once the block clears. A provider-binding failure detected **before** provider
execution — the provider sidecar cannot launch, or a present provider config
lacks a required credential reference — blocks the effect rather than failing it,
so fixing the binding lets the run resume without a manual re-trigger. Every
blocked effect carries a categorized reason in `whip effects`/`status` as
`policy_block: { category, detail }`, where `category` is one of `capability`,
`profile`, `capacity`, `dependency` (scheduling-time) or `provider_config`,
`credentials`, `enforcement`, `provider_health` (binding-time). The detail never
contains secret values.

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
evidence after a crash; see [providers & packages](providers.md). When a run
started but crashed before any terminal or evidence was recorded and the
provider offers no idempotent re-query, recovery resolves it to an **`uncertain`
run terminal**: the effect becomes `failed` (so rules' `fails` branches react)
and the run carries the `runtime.recovery_uncertain` diagnostic, marking "we
could not confirm whether the external side effect happened." Recovery never
silently re-runs such an effect; `retry` is an explicit operator decision.

### Restoring to a prior point (checkpoint / restore)

`checkpoint` and `restore` rewind an instance's *context* — not just one
effect — back to an earlier point.

```sh
whip --store <store> checkpoint <instance>              # capture a cut of head
whip --store <store> checkpoint <instance> --cut-id <id>
whip --store <store> restore    <instance> <cut-id>     # rewind to that cut
```

A checkpoint captures a coherent *cut* across three planes at once: file
state, the agent transcript, and the event-log position. `restore` rewinds
all three back to a named cut as a single coherence-checked operation, so the
planes never drift out of step. It does a full reconcile and auto-checkpoints
head first, so the pre-restore state is itself recoverable.

The rewind is append-only: it records a `context.restored` marker that every
subsequent read folds, rather than deleting history — consistent with the
event log being the source of truth. Add `--json` for machine-readable output.
Checkpoint/restore operate on native file I/O.

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
[`spec/workflow-revision-followups-tracker.md`](https://github.com/jamesjscully/whipplescript/blob/main/spec/workflow-revision-followups-tracker.md).

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

To capture a rewindable point before an intervention (rather than just a JSON
snapshot), take a [checkpoint](#restoring-to-a-prior-point-checkpoint-restore)
of the instance first; `restore` can then return all three planes to that cut.
