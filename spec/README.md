# Armature Specs

Status: draft reset

These specs define the new Armature design from first principles. The current
target is not a statechart language and not a general programming language. It
is a restricted event-sourced rule system for orchestrating agent work.

## North Star

Armature should let coding agents and humans write orchestration logic that is:

- explicit about when agent work is requested
- durable across crashes and restarts
- inspectable through an append-only history
- statically analyzable before it runs
- formally modelable with a small operational semantics
- safe to expose in enterprise environments through capability profiles

Armature should not require authors to debug arbitrary distributed systems
control flow. The runtime owns delivery, effect queues, leases, idempotency,
timeouts, and replay. The language owns policy.

## Current Spec Set

- [core-scope.md](core-scope.md): what belongs in the kernel versus plugins
- [architecture.md](architecture.md): system shape and component boundaries
- [kernel-api.md](kernel-api.md): deterministic runtime kernel operations and transaction boundaries
- [control-plane.md](control-plane.md): programs, instances, CLI, concurrent execution
- [runtime-store.md](runtime-store.md): durable store objects and transaction model
- [fact-provenance.md](fact-provenance.md): fact ownership, projection classes, and replay expectations
- [execution-contract.md](execution-contract.md): rule commits, effect graphs, dependencies, and completions
- [effects-and-capabilities.md](effects-and-capabilities.md): outbox effects, provider bindings, profiles
- [type-system.md](type-system.md): boundary types, schemas, validation, and BAML lowering
- [capability-registry.md](capability-registry.md): runtime authority bindings and enforcement modes
- [plugin-system.md](plugin-system.md): Pi-inspired package, plugin, and resource model
- [skills.md](skills.md): deterministic skill registry and attachment model
- [agent-harness.md](agent-harness.md): provider adapters for real agent turns
- [coerce.md](coerce.md): BAML-backed typed model-decision effects
- [docket-integration.md](docket-integration.md): Docket as a separate core work kernel
- [human-review.md](human-review.md): inbox and human-review effects
- [observability.md](observability.md): artifact/evidence store and status UX
- [memory-plugin.md](memory-plugin.md): memory as a registered plugin capability
- [thoth-plugin.md](thoth-plugin.md): Thoth governance as a registered plugin capability
- [companion-skill.md](companion-skill.md): first-party skill for authoring Armature workflows
- [language.md](language.md): author-facing rule language sketch
- [semantics.md](semantics.md): mathematical runtime model
- [static-analysis.md](static-analysis.md): compiler checks and restrictions
- [verification.md](verification.md): Maude, TLA+/Apalache, Veil, and static-analysis strategy
- [implementation-plan.md](implementation-plan.md): staged project tracker from formal verification through e2e testing
- [examples.md](examples.md): early syntax sketches

## Design Commitments

1. Rules are restricted rewrites over typed facts, not arbitrary programs.
2. Effects are durable outbox records. They never execute inline.
3. Agent completions return as events/facts and are correlated by the runtime.
4. Rules may enqueue finite effect graphs with explicit dependency edges.
5. Source order never implies effect ordering.
6. Recursive rule composition is allowed only under analyzable strata.
7. Effectful cycles must cross an external event, clock, or explicit durable
   boundary.
8. The compiler should be able to explain why a program is safe or rejected.
9. A source file compiles into a versioned program; each run is a durable
   instance managed by the control plane.
10. The core stays small: rule runtime, registries, harnesses, skills, BAML,
   Docket, human review, and observability.
11. Memory, Thoth, external trackers, browsers, research tools, dashboards, and
   evaluators start as plugins unless the kernel must understand them.
12. OpenClaw-lite is an example composition, not a product mode or language
    feature.
