# Existing WhippleScript Reuse Boundary

Status: design guardrail

This document prevents accidental baggage from the existing WhippleScript
script-runner design from shaping the new workflow system.

The new product surface is:

```text
.whip workflow files
native statechart DSL source
validated IR
durable event queue
append-only workflow log
trusted Rust interpreter
typed effects
status and verification commands
```

The old task/service/script model is not the conceptual foundation for this
track.

## Keep If Useful

Existing code or concepts may be reused when they serve the new model directly.

Candidates:

```text
Rust workspace and crate structure
CLI argument parsing patterns
error/reporting utilities
process stdout/stderr capture code
log path conventions, if still useful
serialization helpers
tests and packaging setup
runtime overview lessons
event/run terminology where it remains accurate
```

Reuse standard:

```text
Does this make the .whip workflow runtime simpler, safer, or faster to
ship without importing old assumptions?
```

If the answer is no, do not reuse it.

## Do Not Preserve By Default

These old assumptions should not constrain the new design:

```text
TOML tasks/services as primary user objects
triggers as the main workflow mechanism
arbitrary shell/TypeScript scripts own semantics
scripts are expected to manage durable state
scripts implement their own polling/event loops
process runs are the only important runtime records
WhippleScript is only cron + watcher + process supervisor
```

The workflow system may still invoke external processes through declared
effects, but arbitrary process execution is not workflow logic.

## Rename Or Reframe

Some old words are useful but need sharper meanings.

### Event

Keep the word `event`, but make it a typed durable workflow input.

Old risk:

```text
event as a loose message that triggers a script
```

New meaning:

```text
event as a typed queued input consumed by the interpreter
```

### Run

Keep `run` only for external invocations, such as agent work, tasks, or adapter
jobs.

The workflow itself is not just a process run. It is a durable state machine
with a transition log.

### Log

Distinguish:

```text
process logs: stdout/stderr from external invocations
workflow logs: append-only semantic transition/effect records
```

### Trigger

Avoid exposing `trigger` as a first-class workflow concept unless a concrete
need returns.

In the workflow model, event admission and statechart transitions replace most
trigger semantics.

## Fresh Core Objects

The new system's core objects should be:

```text
WorkflowSource
WorkflowIR
WorkflowInstance
EventQueue
TransitionLog
EffectLog
Interpreter
Adapter
CapabilityPolicy
StatusView
ModelArtifact
```

External invocations may be represented as:

```text
Invocation
```

`Invocation` can cover agent sessions, tasks, external adapter jobs, or future
runtime work. It should not force the old `Run` model if that model is too
narrow.

## Compatibility Posture

Compatibility with old WhippleScript configs is optional and should be treated as a
separate adapter or migration tool.

Examples:

```text
whip migrate old-project.whip --to workflow.whip
adapter: start an old task by name
adapter: observe an old run completion event
```

Compatibility should not leak into the core workflow IR.

## Implementation Rule

When implementing a workflow feature, start from the new object model.

Only reuse old code after answering:

1. What new workflow object does this serve?
2. Does it preserve the no-arbitrary-workflow-code boundary?
3. Does it work with durable event queues and append-only logs?
4. Does it produce useful status/diagnostics?
5. Can it be modeled or abstracted for verification?

If any answer is weak, rewrite or defer.
