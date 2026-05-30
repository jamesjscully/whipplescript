# WhippleScript Documentation

Status: in progress

This directory is the user-facing documentation path. The `spec/` directory
remains the design record and implementation tracker; docs here should explain
the current authoring and operating model directly.

## Start Here

- [Manual](manual.md): end-to-end guide for writing, running, inspecting, and
  operating workflows.
- [API Reference](api-reference.md): exact CLI, language, status/event, JSON,
  and Rust crate surfaces.
- [Language Reference](language-reference.md): `.whip` concepts, syntax, and
  lowering behavior.
- [Runtime And Operations Reference](runtime-operations.md): instance lifecycle,
  effects/runs/leases, provider failures, and inspection commands.
- [CLI Quickstart](../spec/quickstart.md): local commands for checking,
  compiling, running, and inspecting workflows.
- [Operator Guide](../spec/operator-guide.md): store paths, lifecycle controls,
  profiles, providers, and recovery bundles.
- [Plugin Author Guide](../spec/plugin-author-guide.md): how plugins extend
  capabilities and providers without changing language semantics.
- [Verification](../spec/verification.md): Maude, TLA+/Apalache, and static
  analysis strategy.

## Core Mental Model

WhippleScript is a durable rule system for agent orchestration.

```text
facts/events + rules -> durable facts/effects
effects + workers    -> provider runs
provider results     -> events/facts
workflow terminals   -> completed/failed instances
```

The language defines policy. The runtime owns delivery, leases, retries,
idempotency, replay, and inspection.

## Canonical Composition Terms

Use these words consistently in docs, examples, and diagnostics:

| Term | Meaning |
| --- | --- |
| `workflow` | Deployable and invokable runtime boundary. |
| `rule` | Atomic deterministic rewrite over facts/events. |
| `pattern` | Compile-time reusable fragment expanded by `apply`. |
| `include` | Source-file composition for `.whip` and supported boundary files. |
| `use` | Plugin import by name, for example `use memory`. |
| `apply` | Compile-time pattern specialization. |
| `invoke` | Durable child workflow invocation. |
| `revise` | Control-plane activation of a new program version for a running instance. |
| `complete` | Successful workflow terminal action. |
| `fail` | Failed workflow terminal action. |
| skill | Claude-style context bundle assigned to an agent or turn. |
| plugin | Package that registers capabilities, providers, schemas, resources, and optional skills. |

## Implementation Status

The current workspace implements the v0 spine: parser/IR snapshots, SQLite
runtime store, deterministic kernel, CLI, trace conformance, Maude checks,
TLA+/Apalache lifecycle checks, BAML coerce effects, Loft contracts, human
review, skills, plugin registration, pattern expansion, workflow terminal
events, child workflow invocation fixtures, and explicit workflow revision for
non-terminal running instances.

Some surfaces are still transitional. When docs describe intended behavior that
is only partially implemented, they should say so explicitly and link back to
the relevant tracker in `spec/`.
