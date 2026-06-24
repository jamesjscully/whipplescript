# Effects And Capabilities

Status: draft

Effects are the only way WhippleScript interacts with external systems. Capabilities
define which effects a program may request and how the runtime may execute
them.

See [capability-registry.md](capability-registry.md) for binding and
enforcement details. See [type-system.md](type-system.md) for input/output
schema definitions and validation rules.

## Effect Categories

Initial built-in effect categories:

```text
agent.tell       request an agent turn
human.ask        request human input
schema.coerce    request typed schema coercion
capability.call  call a registered external capability
signal.emit      inject a typed signal fact
```

Current implementations may still emit the legacy effect kind `coerce`;
the target contract is `schema.coerce`, with coerce registered as one backend.

Core integrations may expose namespaced effects directly when the compiler or
runtime needs to understand their ordering:

```text
loft.show
loft.claim
loft.renew
loft.release
loft.note
loft.transition
loft.evidence
loft.resource_intent
loft.complete
loft.fail
```

Packages may also expose namespaced effect contracts:

```text
memory.query
memory.write
github.comment
```

These are still ordinary durable effects, currently lowering through
`capability.call` unless a platform-owned lowering class says otherwise.
Namespacing improves typed contracts and status UX; it does not grant packages
new control-flow semantics.

Only two lowering classes are package-authorable: `metadata_only` and
`capability_call`. The `typed_effect_call` and `resource_effect` lowering
classes are compiler-owned for now and may not be authored by packages. A
package effect therefore either contributes typed metadata only or lowers
through `capability.call`; it cannot introduce a new compiler-owned lowering.

All effects share the same terminal lifecycle:

```text
queued -> running -> completed | failed | timed_out | cancelled
```

An effect is claimable only when dependencies are satisfied, policy allows it,
retry/backoff allows it, and any capacity constraint is available. The store may
represent claimable as a query rather than a persisted status. Traces may expose
a synthetic claim step immediately before a provider run starts, but `claimed`
is not a current durable effect status.

Scheduling and policy may also mark an effect:

```text
blocked_by_dependency
blocked_by_capability
blocked_by_profile
blocked_by_capacity
```

## Effect Contract

Every effect has:

```text
kind
target
input schema
output schema
success output schema
failure output schema
timeout output schema
cancel output schema
required capabilities
profile, if applicable
idempotency key
timeout
retry policy
failure categories
dependency edges
```

The generic lifecycle predicates are the branch keywords of the canonical
terminal-output union defined in
[expression-kernel.md](expression-kernel.md) (`Completed<O> | Failed<E> |
TimedOut | Cancelled`):

```text
succeeds     Completed<O>
fails        Failed<E>
times out    TimedOut   (terminal status value `timed_out`)
cancelled    Cancelled
completes    the full four-tag union (for case)
```

They are shared by every effect kind. The typed payload exposed through a named
binding is defined by the specific effect contract; domain success/failure
payloads are the `O`/`E` parameters, not new tags.

For a named effect binding `x`:

```text
after x succeeds     x has the success output type O
after x fails as f   f has the failure payload E (defaults to core Failure)
after x times out    the TimedOut tag
after x cancelled    the Cancelled tag
after x completes     x has the full tagged terminal-output union
```

The language can make effects feel direct:

```whipplescript
tell worker "Implement this work item."
coerce classify(result.summary) as classification
askHuman "This task is blocked. What should happen?"
```

The runtime always lowers them into durable outbox records.

If a rule produces multiple effects, source order does not imply execution
order. Ordering must be expressed through dependency edges. See
[execution-contract.md](execution-contract.md).

## Capability Registration

Capabilities are registered outside the script. A source program declares what
it needs; the runtime environment binds those declarations to concrete
providers.

Example environment sketch:

```toml
[capability.issueTracker]
provider = "mcp"
server = "linear"
allowed = ["issues.read", "issues.write"]

[capability.files]
provider = "local"
root = "."
allowed = ["read", "write"]
deny = [".env", "secrets/**"]

[profile.repo-writer]
provider = "codex"
filesystem = "workspace-write"
network = "denied"
allowed_capabilities = ["files"]

[profile.research]
provider = "codex"
filesystem = "read-only"
network = "allowed"
allowed_capabilities = []
```

The same WhippleScript program can run in different environments if all required
capabilities and profiles are satisfied.

## Script Declarations

A source program should be explicit about non-built-in authority:

```whipplescript
requires capability issueTracker

agent worker {
  profile "repo-writer"
  capacity 5
}
```

Built-in effects such as `tell`, `coerce`, `askHuman`, and Loft operations
still require runtime bindings, but they do not need custom capability
declarations unless policy requires stricter naming.

## Profiles

Profiles are semantic authority bundles for agents or provider-like effects.
They are not provider names.

Good profile names:

```text
repo-reader
repo-writer
internet-research
human-review
enterprise-brokered-worker
```

Bad profile names:

```text
codex
claude
pi
```

Provider selection belongs in environment policy. The language names intent.

## Enforcement

WhippleScript must not claim stronger enforcement than a provider can supply.

Each provider binding reports:

```text
requested authority
enforced authority
best-effort gaps
warnings
```

If a governed environment requires strict enforcement and the provider cannot
enforce the requested boundary, the effect is blocked before execution.

## Admission Authority

An effect's output becomes a durable typed fact only through runtime-boundary
validation, which is **the authority** that admits effect output into typed
facts — not provider-side validation and not source claims (see
[admission-and-idempotency.md](admission-and-idempotency.md)). Providers and
packages never write facts directly. Closed classes reject unknown fields
*after* any backend normalization (e.g. coerce schema-aligned parsing); the
WhippleScript boundary is the final gate. A failed boundary validation produces
a failed effect with a diagnostic and admits no fact.

## Idempotency

Every effect must have a stable idempotency key derived from:

```text
instance_id
program_version
rule_name
trigger_event_id or consumed_fact_keys
effect_index
normalized_input_hash
```

Retries reuse the same effect identity unless the source rule explicitly
produces a new attempt.

When a rule both consumes a matched fact and enqueues effects, the consumed fact
ids are recorded in the same `rule.committed` event as the effects. Provider
workers never infer or perform consumption; they only observe the effect outbox
and append terminal events.
