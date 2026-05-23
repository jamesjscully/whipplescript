# Armature Statechart Workflows

Status: design track

This folder specifies the proposed next Armature product surface: restricted,
verifiable workflow orchestration for coding agents from `.armature` workflow
files.

This design is intentionally fresh. The existing script/task runner may provide
useful Rust plumbing, but it is not the product model this track is trying to
preserve.

The new core runtime is deliberately constrained:

```text
.armature statechart source files
validated workflow IR
durable event queues
append-only transition/effect logs
trusted Rust interpreter
typed effects
runtime status
optional formal verification
```

The system is for users who need reliable agent orchestration without granting
arbitrary TypeScript, shell, or host-language authority to workflows.

## Problem

Plain scripts are powerful, but they create two problems for agent
orchestration:

1. Coding agents often overfit to brittle control-flow patterns. They can write
   event loops that look plausible, miss durable state, and then leave the
   system idle.
2. Arbitrary TypeScript or shell scripts can bypass the security posture of the
   agents they orchestrate. If a worker thread is intentionally denied shell
   access, an ambient Node runtime should not reintroduce equivalent authority.

The desired authoring experience is still code-like. Users and coding agents
should be able to express:

```text
when this happens, inspect this structured signal, send this agent, start this
bounded worker, wait for this terminal condition, escalate if stuck
```

But they should not have to implement durable loops, idempotency, run cursors,
concurrency bounds, event replay, or capability checks themselves.

## Direction

Armature should provide a restricted statechart workflow system:

```text
.armature source file
  -> Armature DSL parser
  -> static validator
  -> generated BAML artifacts for coerce declarations
  -> statechart IR
  -> optional TLA+/Apalache/Veil model
  -> trusted durable interpreter
```

The source format should keep the whole workflow in one place, including:

- agent declarations
- capability contracts
- events
- timers
- states
- transitions
- BAML-shaped `class`, `enum`, and `coerce` declarations
- invariants

The runtime should execute only allowlisted effects. The workflow file may
describe operations like `send`, `start`, `coerce`, `assign`, `askHuman`, and
`raise`; it must not contain arbitrary filesystem, network, process, package
manager, TypeScript, or shell authority. File edits, database writes, approved
scripts, and similar operations must be exposed through declared capabilities
and policy-checked adapters.

## Design Documents

- [architecture.md](architecture.md) defines the system components, candidate
  architectures, and selected direction.
- [product-surface.md](product-surface.md) defines the intended UX, CLI, file
  layout, and status surfaces.
- [authoring-format.md](authoring-format.md) describes the native `.armature`
  statechart DSL.
- [grammar.md](grammar.md) sketches the v0 grammar and parser architecture.
- [source-to-ir.md](source-to-ir.md) defines parsing, expression lowering,
  interpolation, and source-to-IR normalization.
- [baml-integration.md](baml-integration.md) defines how Armature `class`,
  `enum`, and `coerce` declarations lower to BAML artifacts.
- [workflow-ir.md](workflow-ir.md) defines the normalized representation shared
  by parser, validator, runtime, adapters, and model generators.
- [component-contracts.md](component-contracts.md) defines typed interfaces
  between parser/language, engine, adapters, policy, status, and modelgen.
- [effects.md](effects.md) defines effect categories, schemas, synchronous versus
  asynchronous behavior, and idempotency.
- [event-queue.md](event-queue.md) defines durable event queue semantics.
- [storage.md](storage.md) defines the SQLite-backed durable queue/log/state
  storage model.
- [policy.md](policy.md) defines capability modes and authority resolution.
- [runtime-semantics.md](runtime-semantics.md) defines the trusted interpreter
  semantics.
- [verification.md](verification.md) describes how workflow invariants compile
  to model-checkable/provable transition systems, with Veil as the first target.
- [implementation-plan.md](implementation-plan.md) sequences specification,
  formal modeling, runtime scaffolding, generated validation, and gates.
- [reuse-boundary.md](reuse-boundary.md) defines what may be kept from the
  existing Armature implementation and what should not constrain the new system.
- [external-validation.md](external-validation.md) records documentation checks,
  local probes, corrections, and remaining unvalidated assumptions for external
  systems.
- [spec-implementation-example.armature.md](spec-implementation-example.armature.md)
  sketches the workflow we wish the managed spec orchestration example could
  become.

## Non-Goals

This system is not intended to become a general-purpose programming
language. If users need arbitrary computation, they should put that computation
behind an explicitly declared capability boundary and call it as a runtime
action, agent, or coerce function.

This system does not support arbitrary TypeScript or shell as workflow logic.
External tools may still exist behind declared effects or adapters.

## Relationship To Existing Armature

This design supersedes the old product center for this track. The old Armature
runtime may be mined for useful ideas and code:

```text
CLI structure
Rust crate organization
process/log capture ideas
event/run naming where useful
tests and packaging
```

But the old assumptions are not binding:

```text
arbitrary scripts own workflow semantics
TOML tasks/services are the primary abstraction
triggers are the main control-flow mechanism
TypeScript is an acceptable workflow runtime
```

The new boundary is:

**Users author `.armature` workflow files. Armature validates and executes them
through a constrained Rust interpreter.**
