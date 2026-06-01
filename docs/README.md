# WhippleScript Documentation

This directory is the user-facing documentation path. The `spec/` directory
remains the design record and implementation tracker; docs here should explain
the current authoring and operating model directly.

Stability note: the in-repo tests and local runtime checks may call individual
subsystems stable, but the public language, CLI, runtime behavior, and
provider/plugin interfaces are still early and may change.

## Start Here

- [Quickstart](quickstart.md): install from source, run a fixture-backed local
  workflow, and inspect the result.
- [Tutorial](tutorial.md): route work to logical agents, review the results,
  and inspect durable facts/effects.
- [Concepts](concepts.md): workflow, fact, event, rule, effect, agent,
  provider, worker, and local command boundaries.
- [Manual](manual.md): end-to-end guide for writing, running, inspecting, and
  operating workflows.
- [Install](install.md): source install, planned binary releases, verification,
  and troubleshooting.
- [API Reference](api-reference.md): exact CLI, language, status/event, JSON,
  and Rust crate surfaces.
- [Language Reference](language-reference.md): `.whip` syntax and lowering
  behavior.
- [Runtime And Operations Reference](runtime-operations.md): instance lifecycle,
  effects/runs/leases, provider failures, and inspection commands.
- [Providers And Plugins](providers.md): fixture provider, experimental native
  providers, plugin entry points, and validation scripts.
- [Examples](examples.md): which checked workflow to try and why.
- [Current State](current-state.md): what works today, what is experimental,
  and what "stable" means in this repo.
- [Troubleshooting](troubleshooting.md): first-10-minute setup and runtime
  issues.
- [Operator Guide](operator-guide.md): store paths, lifecycle controls,
  profiles, providers, and recovery bundles.
- [Plugin Authoring](plugin-authoring.md): how plugins extend
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
