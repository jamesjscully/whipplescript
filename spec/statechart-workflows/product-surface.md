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
armature status
```

For a single workflow project, the common commands should be short. The user
should not have to type `workflow` in every command unless ambiguity requires
it.

Verbose workflow forms may also exist for scripting and help output:

```sh
armature workflow validate workflow.armature
armature workflow check workflow.armature
armature workflow run workflow.armature
armature workflow status spec-implementation
```

The short forms should map to workflow operations because workflows are the main
product object in this design. This intentionally breaks from the legacy
task/service CLI. Compatibility, if needed, should live under explicit legacy
commands or adapters rather than preserving old short-form meanings.

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

### `armature init`

Creates a minimal local project:

```text
workflow.armature
.armature/policy.json
.armature/state/
```

It should ask as few questions as possible. Defaults are permissive for local
use.

### `armature validate [file]`

Parses `.armature` source, generates BAML schemas for `coerce`, builds IR, and
runs static validation. `--adapter-manifest` validates adapter-backed effect
contracts, and `--policy` validates manifest-required capabilities against
explicit capability policy documents.

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

Runs stronger backend-specific checks, likely Veil once the generated model
target is mature. In the current implementation, this command validates the
workflow and supplied adapter/policy contracts, then reports that proof backends
are not available yet. `--json` returns a structured unavailable response with a
suggested `check` command.

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
- begins processing durable events

It does not execute arbitrary TypeScript or shell workflow logic.

### `armature emit <event> --json <payload>`

Adds a typed event to the durable workflow event queue.

This is useful for local testing, adapters, and integration with external
systems.

For multiple workflows, `emit` requires a workflow selector unless routing is
unambiguous.

### `armature status [workflow]`

Shows what the workflow is doing and why.

Example:

```text
Workflow: spec-implementation
State: running.watching
Active:
  worker: 3/4
  quality: 1/2
Latest transition:
  finished -> running.choosing -> running.watching
Latest decision:
  coerce chooseNextStep -> start worker impl-017
Latest effects:
  start worker accepted requires=adapter.agent.start
Waiting for:
  finished from impl-014, impl-015, impl-017
Next observation:
  idle when activeRuns == 0
Blocked:
  none
```

### `armature events [workflow]`

Shows queued, processed, ignored, and failed events.

### `armature log [workflow]`

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
