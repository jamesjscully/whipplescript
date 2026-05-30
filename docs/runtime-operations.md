# Runtime And Operations Reference

Status: in progress

This page explains what happens after a `.whip` bundle is checked, compiled,
and started.

## Runtime Loops

WhippleScript separates deterministic rule progress from provider execution.

| Loop | Responsibility |
| --- | --- |
| starter | Create an instance and seed initial input facts/events. |
| stepper | Evaluate ready rules and commit facts/effects/dependencies atomically. |
| worker | Claim materialized effects, start provider runs, renew/expire leases, and record completions. |
| projection/recovery | Rebuild or advance current views from the durable event log. |

`step` must not execute providers. `worker` must not invent source policy. `dev`
may compose loops for local validation, but the durable boundaries stay the
same.

## Store

The default local store is:

```text
.whipplescript/store.sqlite
```

Use `--store <path>` or `WHIPPLESCRIPT_STORE` to isolate environments:

```sh
cargo run -p whipplescript-cli -- --store .whipplescript/dev.sqlite doctor
```

The store records:

```text
program versions
instances
events
facts
effects
effect dependencies
runs
leases
workflow invocations
inbox items
evidence
artifacts
capabilities
profiles
provider bindings
plugin manifests
```

The event log is the source of truth. Facts, effects, status views, traces, and
invocation links are projections over durable state.

## Instance Lifecycle

Instances move through these states:

```text
created
running
paused
blocked
completed
failed
cancelled
```

`completed`, `failed`, and `cancelled` are terminal. Terminal instances reject
additional rule commits and public lifecycle transitions.

`complete <output> { ... }` appends a `workflow.completed` event, stores the
terminal payload, and marks the instance `completed`.

`fail <failure> { ... }` appends a `workflow.failed` event, stores the terminal
payload, and marks the instance `failed`.

`cancel` is an operator control-plane action. It appends an instance transition
event and prevents further progress; it is not equivalent to workflow `fail`.

## Effects, Runs, And Leases

An effect is a durable request for external work. A run is one provider attempt
for an effect. A lease protects a running provider attempt from duplicate
workers.

Typical effect lifecycle:

```text
queued -> running -> completed
queued -> running -> failed
queued -> running -> timed_out
queued -> blocked_by_dependency
queued -> blocked_by_capacity
queued -> blocked_by_policy
```

Terminal provider outcomes are recorded as effect/run events. They do not
automatically fail the workflow instance. Source rules decide whether to retry,
escalate, ignore, or execute `fail`.

## Provider Failure Capture

Provider failures should be represented in three places:

```text
event stream  -> what happened and when
run/effect    -> current terminal provider status
evidence      -> diagnostic payload, provider name, artifacts, and causal links
```

This is the desired distinction:

| Failure | Meaning | Workflow state |
| --- | --- | --- |
| provider run failed | The harness/provider could not complete an effect. | unchanged unless rules react |
| effect timed out | The run exceeded policy or local dev bounds. | unchanged unless rules react |
| workflow failed | A rule executed `fail <failure> { ... }`. | terminal `failed` |
| instance cancelled | Operator or policy cancelled the instance. | terminal `cancelled` |

Provider adapters should capture real errors, not just synthetic status codes:

```text
process exit status
stderr/stdout excerpts
SDK exception class/message
HTTP status/body excerpt
timeout reason
missing credential/configuration reason
artifact paths
provider correlation ids
```

The captured diagnostic must be safe to store. Do not persist provider secrets.

## Workflow Invocation

`invoke Workflow { ... } as binding` creates a durable child workflow request.
The child has its own instance lifecycle and event log.

Parent effects resolve from child terminal state:

```text
child completed -> parent invocation succeeds with declared output payload
child failed    -> parent invocation fails with declared failure payload
child timed out -> parent invocation fails/times out according to policy
child cancelled -> parent invocation completes on the cancellation branch
```

The parent observes declared terminal payloads and invocation metadata. It does
not read child-local facts as ordinary parent facts.

## Inspecting State

Common commands:

```sh
cargo run -p whipplescript-cli -- --store .whipplescript/dev.sqlite instances
cargo run -p whipplescript-cli -- --store .whipplescript/dev.sqlite status <instance>
cargo run -p whipplescript-cli -- --store .whipplescript/dev.sqlite log <instance>
cargo run -p whipplescript-cli -- --store .whipplescript/dev.sqlite facts <instance>
cargo run -p whipplescript-cli -- --store .whipplescript/dev.sqlite effects <instance>
cargo run -p whipplescript-cli -- --store .whipplescript/dev.sqlite runs <instance>
cargo run -p whipplescript-cli -- --store .whipplescript/dev.sqlite evidence <instance> --json
cargo run -p whipplescript-cli -- --store .whipplescript/dev.sqlite trace <instance> --check --json
```

`status --json` includes parent/child invocation links when available.

## Lifecycle Controls

```sh
cargo run -p whipplescript-cli -- --store .whipplescript/dev.sqlite pause <instance>
cargo run -p whipplescript-cli -- --store .whipplescript/dev.sqlite resume <instance>
cargo run -p whipplescript-cli -- --store .whipplescript/dev.sqlite cancel <instance>
cargo run -p whipplescript-cli -- --store .whipplescript/dev.sqlite retry <instance> <effect>
```

Pause/resume are nonterminal controls. Cancel is terminal. Retry moves eligible
failed or timed-out effects back to queued when policy allows.

## Incident Bundle

Before manually repairing or deleting runtime state, collect:

```sh
cargo run -p whipplescript-cli -- --store <store> status <instance> --json
cargo run -p whipplescript-cli -- --store <store> log <instance> --json
cargo run -p whipplescript-cli -- --store <store> facts <instance> --json
cargo run -p whipplescript-cli -- --store <store> effects <instance> --json
cargo run -p whipplescript-cli -- --store <store> runs <instance> --json
cargo run -p whipplescript-cli -- --store <store> evidence <instance> --json
cargo run -p whipplescript-cli -- --store <store> trace <instance> --check --json
```

For provider issues, also preserve the relevant artifacts and provider-specific
configuration names, but not credential values.
