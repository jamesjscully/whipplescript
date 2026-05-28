# Statechart Workflow Product Surface

Status: design proposal

The intended product surface is running `.whip` workflow files.

The user should not need to understand tasks, services, triggers, TypeScript
workers, or daemon internals to use the system. Those may exist as
implementation details or adapters, but the product should present workflows as
the primary object.

## Core User Loop

The core loop:

```sh
whip init
whip validate workflow.whip
whip check workflow.whip
whip run workflow.whip
whip status workflow.whip
```

For a single workflow project, the common commands should be short. The user
should not have to type `workflow` in every command unless ambiguity requires
it. The implemented v0 CLI still requires the workflow file path for commands
that inspect or mutate a workflow instance.

Verbose workflow forms may also exist for scripting and help output:

```sh
whip workflow validate workflow.whip
whip workflow check workflow.whip
whip workflow run workflow.whip
whip workflow status spec-implementation
```

The implemented v0 CLI exposes the short workflow forms directly, for example
`whip validate workflow.whip` and `whip status workflow.whip`.
Namespaced `whip workflow ...` forms are not implemented yet; if added,
they should be aliases over the same workflow operations. This intentionally
breaks from the legacy task/service CLI. Compatibility, if needed, should live
under explicit legacy commands or adapters rather than preserving old short-form
meanings.

## Files

Default project layout:

```text
workflow.whip
.whippletree/
  build/
    ir.json
    baml_src/
    models/
  state/
  workflows/
    <workflow-name>.sqlite
  runs/
    <invocation-id>/
      stdout.log
      stderr.log
  policy.json
```

Larger projects may use multiple workflows:

```text
workflows/
  spec-implementation.whip
  nightly-maintenance.whip
.whippletree/
  state/
    spec-implementation/
    nightly-maintenance/
```

## Source Format

The source format is the native `.whip` statechart DSL described in
[authoring-format.md](authoring-format.md).

Reasons:

- keeps standard statechart concepts visible
- supports local declarations near the states that use them
- follows BAML syntax for `class`, `enum`, and `coerce`
- avoids forcing workflow logic into manifest-shaped HJSON
- still compiles to ordinary JSON-shaped WorkflowIR for tests and runtime work

The file extension remains `.whip` because the product object is an
orchestrated workflow.

## Commands

### `whip init [dir] --name [machine-name]`

Creates a minimal local project. The implemented v0 command is noninteractive,
defaults to the current directory, and refuses to overwrite existing scaffold
files unless `--force` is passed. `--name` defaults to `Workflow` and must be a
valid `.whip` identifier.

```text
workflow.whip
.whippletree/policy.json
.whippletree/state/
.whippletree/workflows/
```

It should ask as few questions as possible. Defaults are permissive for local
use.

### `whip validate [file]`

Parses `.whip` source, generates BAML source artifacts for `coerce`, builds
IR, and runs static validation. `--adapter-manifest` validates adapter-backed
effect contracts, and `--policy` validates manifest-required capabilities
against explicit capability policy documents. `--profile-policy` validates
agent profile references and the harness authority policy that will later
resolve native provider launches.

Checks:

- source syntax
- event schemas
- states and transitions
- guards and expressions
- effect names and argument schemas
- declared agents
- declared harness profiles, when a profile policy is supplied
- declared capability references
- coerce function schemas
- obvious unbounded loops
- invariant declarations

This command should be fast and should not require heavyweight formal tooling.

### `whip validate-adapter [manifest...]`

Validates adapter manifest files independently from a workflow. This checks
manifest shape, effect/event duplicates, schema references, idempotency
requirements, and model abstractions.

### `whip validate-policy [policy...]`

Validates capability policy documents independently from a workflow. This checks
policy document shape, duplicate capability entries, empty capability names, and
direct allow/deny conflicts.

### `whip check [file]`

Runs validation plus bounded model checks when tooling is available. If
`--adapter-manifest` or `--policy` is supplied, those contracts are validated
before model checking starts.

The first target is likely TLA+/Apalache because useful counterexamples matter
more than proof elegance early.

### `whip emit-config [file] --target tla`

Emits the checker configuration that matches the generated formal model. This is
useful for CI, debugging, and agents that need to inspect exactly which
generated invariants `check` will run. In the current implementation this is
only meaningful for TLA; Maude checks are embedded in the generated `.maude`
file.

### `whip prove [file]`

Runs the strongest generated verification bundle currently implemented. In the
current implementation, this command validates the workflow and supplied
adapter/policy contracts, then runs both supported generated backends: TLA+ and
Maude. `--json` returns a structured aggregate with each backend's check
result. Future proof-oriented targets such as Veil can be added to this bundle
once their generated model target is mature.

This is an expert or enterprise command. It should not be required for the
first local prototype loop.

### `whip run [file]`

Starts or resumes a workflow instance from a `.whip` file.

The command:

- validates the workflow
- validates supplied adapter manifests and policy documents
- builds or loads IR
- initializes durable state if needed
- starts the Rust interpreter
- connects declared adapters
- connects a BAML HTTP server when real `coerce` calls are enabled
- records native local agent invocations in SQLite
- begins processing durable events

It does not execute arbitrary TypeScript or shell workflow logic.

Real BAML-backed `coerce` execution uses:

```text
--baml-url http://127.0.0.1:2024
```

File-backed local adapter slices use:

```text
--plan-file plan.json
--review-file reviews.json
```

`--plan-file` backs `plan.snapshot()`, `plan.unfinishedItems()`,
`plan.nextReadyItem()`, and plan status updates. `--review-file` backs
`askHuman(...)` by appending visible open review obligations. For
`whip emit`, `--review-file` supplies the typed
`humanReview.responded` adapter event schema so review responses can be
validated before entering the durable queue.

Local agent `start` and `send` do not use a JSON side file. They write native
SQLite ledger records that are claimed by `whip harness`. Explicit
adapter-backed agents can still be supplied through adapter manifests.

These file-backed flags can supply built-in JSON adapter manifests for workflows
that only need those plan/review surfaces; larger workflows can still pass
explicit adapter manifests.
`status` and `overview` accept the same file-backed flags for validation
context, but they must not call adapters or read those JSON files.

Managed `baml-cli serve` process mode may be added later, but the first real
execution path should use an explicitly supplied BAML HTTP URL.

### `whip harness once [file] --config <config> [--profile-policy <policy>]`

Claims one queued native agent invocation, runs the configured provider, records
stdout/stderr artifacts, records a durable completion, and enqueues the typed
workflow completion event. This command is the deterministic testable unit of
the local harness.

Provider config maps declared Whippletree agents to provider runners:

```json
{
  "agents": {
    "worker": {
      "provider": "command",
      "command": ["sh", "-c", "printf '%s\n' \"$WHIPPLETREE_PROMPT\""],
      "cwd": ".",
      "timeoutSeconds": 1800
    }
  }
}
```

Supported `provider` values are `command`, `codex`, `claude`, and `pi`.
`command` requires `command`. Presets supply a default command template but may
also receive an explicit `command` override and extra `args`.

Governed environments should pass `--profile-policy` and use source-level agent
profiles instead of exposing raw command/provider choices in workflow logic:

```whippletree
agent researcher = codingAgent() {
  profile "research"
  maxActive 2
}

agent worker = codingAgent() {
  profile "repo-writer"
  maxActive 3
}
```

The profile policy resolves semantic profiles to concrete provider runners,
filesystem posture, network posture, environment allowlists, tool hints,
timeout, and enforcement mode. Local experiments may keep using `--config`
directly. The long-term product path is a profile policy that can embed or
reference concrete runner config while keeping workflow source portable.

Command strings support these placeholders before execution:

- `{{prompt}}`
- `{{inputJson}}`
- `{{invocationId}}`
- `{{agent}}`
- `{{runDir}}`

`timeoutSeconds` is enforced by the harness. A timeout kills the provider
process, records `provider_timed_out`, marks the invocation `timed_out`, and
enqueues a `finished` event with `status: "timed_out"` when the workflow
declares a compatible completion event.

Failed provider commands are recorded as harness events so repeated agent
mistakes become desire-path signals.

### `whip harness run [file] --config <config> [--profile-policy <policy>] [--drive-workflow]`

Runs the harness supervisor loop. The MVP may poll the SQLite ledger for queued
work; polling is a wakeup strategy, not the data model. The durable truth is the
event/invocation ledger.

Without `--drive-workflow`, the loop claims and runs queued native invocations.
With `--drive-workflow`, it also processes queued workflow events between
provider runs, so a `finished` event can immediately update workflow state and
schedule follow-on work.

The loop:

- recover expired claims
- claim queued invocations
- run providers
- enqueue typed completions
- avoid corrupting claims on shutdown

### `whip harness status [file]`

Shows native harness state: queued invocations, claimed/running invocations,
recent completions, recent provider failures, stdout/stderr artifact paths, and
recent desire-path observations. It includes the projected workflow `status`
when a store exists, but it must not call providers for hidden live data.

### `whip emit [file] --event <event> --payload <payload>`

Adds a typed event to the durable workflow event queue.

This is useful for local testing, adapters, and integration with external
systems.

`--adapter-manifest` contributes adapter-owned event schemas at the event intake
boundary. `--policy` validates policy document shape for command consistency,
but `emit` does not validate adapter-backed workflow effects because it does not
execute them.

For multiple workflows, `emit` requires a workflow selector unless routing is
unambiguous.

### `whip status [file]`

Shows what the workflow is doing and why.

When supplied, `--adapter-manifest` and `--policy` validate the workflow
against the same contracts used by `run`, `build`, and `check` before status is
projected. Status itself still reads only durable local state; it does not call
adapters.

Example:

```text
workflow: spec-implementation
state: running.watching
waiting: waiting for active invocation(s): worker=3, quality=1
data: {"seenRuns":["impl-014","impl-015"],"lastIdleNudgeAt":"2026-05-23T10:00:00Z"}
data summary: {"seenRuns":2,"lastIdleNudgeAt":"2026-05-23T10:00:00Z"}
pending events: 0
active:
  worker: 3/4
  quality: 1/2
queued events: none
latest transition: running.watching.on.finished[0]
latest effects:
  start worker Dispatched requires=agent.worker.start
current effect failures: none
current blockers: none
recent failures (history): none
policy blockers: none
latest coerce:
  chooseNextStep Succeeded http=None
current coerce failure: none
latest coerce failures (history): none
```

`whip status [file] --compact` prints the same durable projection in a
short operator view:

```text
workflow: spec-implementation
state: running.watching
waiting: waiting for active invocation(s): worker=3, quality=1
pending events: 0
active: worker=3/4, quality=1/2
current blockers: none
latest transition: running.watching.on.finished[0]
```

### `whip events [file]`

Shows queued, processing, processed, ignored, failed, and dead-lettered events.
The status filter uses durable event status names, such as `--status failed`
and `--status dead_lettered`. Human text output includes attempt counts when
nonzero and the durable `last_error` when present, so operators can triage
failed events without opening SQLite.

### `whip retry-event [file] --event-id [event]`

Administrative retry for failed or dead-lettered events. It requeues the event
without hiding prior attempts, so status and event inspection still show retry
history. Human text output confirms the requeued event id, queued status, and
resulting pending event count.

### `whip log [file]`

Shows the append-only transition/effect log in a human-readable form.

This should be separate from process stdout/stderr logs. The workflow log is the
semantic audit trail.

### `whip build [file]`

Compiles a workflow file into build artifacts without running it:

```text
IR JSON
extracted BAML source
generated model artifacts
adapter manifest and policy bundles, when supplied
diagnostic metadata
```

This command is useful for CI and debugging.

## Status Philosophy

Status is not a convenience feature. It is the product answer to the failure
mode that motivated this design: agent-written scripts can go idle or wrong in
ways that are hard to inspect.

Every workflow should be able to answer:

```text
what state am I in?
why do I appear to be waiting?
what durable workflow data matters right now?
what event am I processing or waiting for?
what external work is active?
what did BAML decide recently?
what effects have been dispatched?
what failed?
what policy blocked me?
what invariant/check is relevant?
```

## Capability Modes

The product should support multiple policy postures:

```text
local       permissive defaults, warnings for broad authority
team        explicit capabilities encouraged, moderate defaults
enterprise  deny-by-default, intersection across all policy layers
```

The mechanism can be the same in every mode: resolve effective authority from
workflow declarations, workspace policy, and target adapter capabilities. The
defaults differ by mode.

The concrete policy algorithm is defined in [policy.md](policy.md).

## Explicit Non-Surface

These should not be primary user-facing concepts for this track:

```text
TOML task definitions
script-trigger wiring
arbitrary TypeScript orchestrators
long-running user-authored event loops
daemon process internals
```

They may exist behind adapters or in compatibility layers, but they should not
shape the workflow product.
