# Architecture

Status: draft

WhippleScript has six conceptual layers:

```text
source rule language
  -> typed rule IR
  -> static analyzer and verifier
  -> event-sourced runtime kernel and store
  -> control plane
  -> registries
  -> package/provider registration and capability enforcement
```

The core should stay intentionally small. It should provide the machinery for
durable orchestration and typed authority, then let packages register most
domain-specific behavior.

Core integrations:

```text
agent harness interface
capability registry
skill registry
schema-coercion effects, with coerce as a current backend
human review inbox
artifact/evidence store
observability/status views
```

Package/provider-by-default integrations:

```text
memory systems
GitHub / Linear / Jira
browser automation
web research
notification systems
custom dashboards
specialized evaluators
```

## Runtime State

The runtime state is:

```text
R = (L, F, Q, D, C)
```

Where:

- `L` is an append-only event log.
- `F` is the current fact set, derived from accepted events and committed rule
  rewrites.
- `Q` is a durable effect outbox.
- `D` is the durable effect-dependency relation.
- `C` is runtime control metadata such as leases, clocks, attempts, and
  idempotency keys.

This state is persisted by the runtime store and inspected through the control
plane. The product surface should expose both history and current projection:

```text
whip log
whip facts
whip effects
whip status
```

The kernel owns the durable transaction primitives — event append, projection,
rule commit, effect enqueue/claim/complete, lease lifecycle, and the
instance-lifecycle transitions (create/pause/resume/cancel/revise activation).
The control plane decides *when* to invoke those transactions (recovery loops,
workers, CLI) but does not reimplement their semantics: pause/resume/cancel and
revision activation are kernel transactions sequenced by the control plane, not
control-plane-private state mutations. See [kernel-api.md](kernel-api.md).

Facts in `F` are typed projection records with explicit provenance. See
[fact-provenance.md](fact-provenance.md).

## Source Program

A source program defines:

```text
P = (Schemas, Agents, Capabilities, Skills, Rules, Invariants)
```

- `Schemas` define typed facts and event payloads.
- `Agents` define addressable agent roles and requested profiles.
- `Capabilities` define external effect surfaces that the runtime may expose.
- `Skills` define deterministic context bundles attached to agents or turns.
- `Rules` define how facts/events produce new facts and effects.
- `Invariants` define safety properties the compiler and verifier should check.

A source program is not a process. It compiles into an immutable program
version. Running that version creates a durable instance.

```text
source file -> program version -> instance
```

Multiple instances of the same program may run concurrently.

## Execution Boundary

Rules do not call providers directly. A rule may enqueue an effect graph:

```text
tell(worker, work)
askHuman(question)
coerce(function, input)
capability(name, input)
```

The runtime writes effect records to `Q` and dependency edges to `D`. A harness
may start a run only after the effect's dependencies are satisfied and policy
accepts the requested capability/profile. The harness runs the provider, writes
artifacts, and appends a completion event.

This gives every external action the same durable lifecycle:

```text
queued -> running -> completed | failed | timed_out | cancelled
```

## Why This Shape

The previous statechart design made global modes readable but made concurrent
agent work awkward. Petri-net-inspired token flow made concurrency visible but
created an ugly nested formalism. The rule-machine design keeps the core data
structure singular:

```text
facts + rewrite rules + durable effects
```

Statecharts may return later as optional mode sugar if the rule core needs it,
but they are not the foundation.

## Supporting Specs

The architecture depends on these supporting specs:

- [control-plane.md](control-plane.md) defines programs, instances, and CLI
  operations.
- [kernel-api.md](kernel-api.md) defines deterministic instance transition
  operations and transaction boundaries.
- [runtime-store.md](runtime-store.md) defines the durable database model.
- [fact-provenance.md](fact-provenance.md) defines fact ownership and replay
  expectations.
- [execution-contract.md](execution-contract.md) defines effect graphs,
  dependency edges, scheduling, and completion facts.
- [effects-and-capabilities.md](effects-and-capabilities.md) defines outbox
  effects and authority binding.
- [capability-registry.md](capability-registry.md) defines capability binding
  and enforcement.
- [plugin-system.md](plugin-system.md) preserves legacy runtime
  provider-registry notes from the retired plugin model.
- [skills.md](skills.md) defines deterministic skill loading and attachment.
- [agent-harness.md](agent-harness.md) defines provider adapters for real
  coding-agent turns.
- [coerce.md](coerce.md) defines typed schema-coercion as a durable effect.
- [human-review.md](human-review.md) defines human inbox semantics.
- [observability.md](observability.md) defines evidence and trace UX.
