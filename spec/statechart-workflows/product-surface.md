# Statechart Workflow Product Surface

Status: design proposal

The intended product surface is running `.armature` workflow files.

The user should not need to understand tasks, services, triggers, TypeScript
workers, or daemon internals to use the system. Those may exist as
implementation details or adapters, but the product should present workflows as
the primary object.

## Core User Loop

The core loop:

```sh
armature init
armature validate workflow.armature
armature check workflow.armature
armature run workflow.armature
armature status workflow.armature
```

For a single workflow project, the common commands should be short. The user
should not have to type `workflow` in every command unless ambiguity requires
it. The implemented v0 CLI still requires the workflow file path for commands
that inspect or mutate a workflow instance.

Verbose workflow forms may also exist for scripting and help output:

```sh
armature workflow validate workflow.armature
armature workflow check workflow.armature
armature workflow run workflow.armature
armature workflow status spec-implementation
```

The implemented v0 CLI exposes the short workflow forms directly, for example
`armature validate workflow.armature` and `armature status workflow.armature`.
Namespaced `armature workflow ...` forms are not implemented yet; if added,
they should be aliases over the same workflow operations. This intentionally
breaks from the legacy task/service CLI. Compatibility, if needed, should live
under explicit legacy commands or adapters rather than preserving old short-form
meanings.

## Files

Default project layout:

```text
workflow.armature
.armature/
  build/
    ir.json
    baml_src/
    models/
  state/
    events/
    transitions.log
    effects.log
    current.json
  workflows/
    <workflow-name>.sqlite
  policy.json
```

Larger projects may use multiple workflows:

```text
workflows/
  spec-implementation.armature
  nightly-maintenance.armature
.armature/
  state/
    spec-implementation/
    nightly-maintenance/
```

## Source Format

The source format is the native `.armature` statechart DSL described in
[authoring-format.md](authoring-format.md).

Reasons:

- keeps standard statechart concepts visible
- supports local declarations near the states that use them
- follows BAML syntax for `class`, `enum`, and `coerce`
- avoids forcing workflow logic into manifest-shaped HJSON
- still compiles to ordinary JSON-shaped WorkflowIR for tests and runtime work

The file extension remains `.armature` because the product object is an
orchestrated workflow.

## Commands

### `armature init [dir] --name [machine-name]`

Creates a minimal local project. The implemented v0 command is noninteractive,
defaults to the current directory, and refuses to overwrite existing scaffold
files unless `--force` is passed. `--name` defaults to `Workflow` and must be a
valid `.armature` identifier.

```text
workflow.armature
.armature/policy.json
.armature/state/
.armature/workflows/
```

It should ask as few questions as possible. Defaults are permissive for local
use.

### `armature validate [file]`

Parses `.armature` source, generates BAML source artifacts for `coerce`, builds
IR, and runs static validation. `--adapter-manifest` validates adapter-backed
effect contracts, and `--policy` validates manifest-required capabilities
against explicit capability policy documents.

Checks:

- source syntax
- event schemas
- states and transitions
- guards and expressions
- effect names and argument schemas
- declared agents
- declared capability references
- coerce function schemas
- obvious unbounded loops
- invariant declarations

This command should be fast and should not require heavyweight formal tooling.

### `armature validate-adapter [manifest...]`

Validates adapter manifest files independently from a workflow. This checks
manifest shape, effect/event duplicates, schema references, idempotency
requirements, and model abstractions.

### `armature validate-policy [policy...]`

Validates capability policy documents independently from a workflow. This checks
policy document shape, duplicate capability entries, empty capability names, and
direct allow/deny conflicts.

### `armature check [file]`

Runs validation plus bounded model checks when tooling is available. If
`--adapter-manifest` or `--policy` is supplied, those contracts are validated
before model checking starts.

The first target is likely TLA+/Apalache because useful counterexamples matter
more than proof elegance early.

### `armature emit-config [file] --target tla`

Emits the checker configuration that matches the generated formal model. This is
useful for CI, debugging, and agents that need to inspect exactly which
generated invariants `check` will run. In the current implementation this is
only meaningful for TLA; Maude checks are embedded in the generated `.maude`
file.

### `armature prove [file]`

Runs the strongest generated verification bundle currently implemented. In the
current implementation, this command validates the workflow and supplied
adapter/policy contracts, then runs both supported generated backends: TLA+ and
Maude. `--json` returns a structured aggregate with each backend's check
result. Future proof-oriented targets such as Veil can be added to this bundle
once their generated model target is mature.

This is an expert or enterprise command. It should not be required for the
first local prototype loop.

### `armature run [file]`

Starts or resumes a workflow instance from a `.armature` file.

The command:

- validates the workflow
- validates supplied adapter manifests and policy documents
- builds or loads IR
- initializes durable state if needed
- starts the Rust interpreter
- connects declared adapters
- connects a BAML HTTP server when real `coerce` calls are enabled
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
--agent-file agents.json
```

`--plan-file` backs `plan.snapshot()`, `plan.unfinishedItems()`,
`plan.nextReadyItem()`, and plan status updates. `--review-file` backs
`askHuman(...)` by appending visible open review obligations. For
`armature emit`, `--review-file` supplies the typed
`humanReview.responded` adapter event schema so review responses can be
validated before entering the durable queue.
`--agent-file` backs `start` and `send` by appending inspectable invocation and
message records. For `armature emit`, `--agent-file` supplies the typed
`finished` completion event schema used by the file-backed agent bridge. These
flags can supply built-in JSON adapter manifests for workflows that only need
those surfaces; larger workflows can still pass explicit adapter manifests.
`status` and `overview` accept the same file-backed flags for validation
context, but they must not call adapters or read those JSON files.

Managed `baml-cli serve` process mode may be added later, but the first real
execution path should use an explicitly supplied BAML HTTP URL.

### `armature emit [file] --event <event> --payload <payload>`

Adds a typed event to the durable workflow event queue.

This is useful for local testing, adapters, and integration with external
systems.

`--adapter-manifest` contributes adapter-owned event schemas at the event intake
boundary. `--policy` validates policy document shape for command consistency,
but `emit` does not validate adapter-backed workflow effects because it does not
execute them.

For multiple workflows, `emit` requires a workflow selector unless routing is
unambiguous.

### `armature status [file]`

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
  start worker Dispatched requires=adapter.agent.start
recent failures: none
policy blockers: none
latest coerce:
  chooseNextStep Succeeded http=None
latest coerce failures: none
```

### `armature events [file]`

Shows queued, processing, processed, ignored, failed, and dead-lettered events.
The status filter uses durable event status names, such as `--status failed`
and `--status dead_lettered`. Human text output includes attempt counts when
nonzero and the durable `last_error` when present, so operators can triage
failed events without opening SQLite.

### `armature retry-event [file] --event-id [event]`

Administrative retry for failed or dead-lettered events. It requeues the event
without hiding prior attempts, so status and event inspection still show retry
history. Human text output confirms the requeued event id, queued status, and
resulting pending event count.

### `armature log [file]`

Shows the append-only transition/effect log in a human-readable form.

This should be separate from process stdout/stderr logs. The workflow log is the
semantic audit trail.

### `armature build [file]`

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
