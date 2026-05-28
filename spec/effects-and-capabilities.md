# Effects And Capabilities

Status: draft

Effects are the only way Armature interacts with external systems. Capabilities
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
baml.coerce      request typed model coercion
capability.call  call a registered external capability
event.emit       append a typed event
```

Core integrations may expose namespaced effects directly when the compiler or
runtime needs to understand their ordering:

```text
docket.claim
docket.release
docket.note
docket.close
```

Plugins may also expose namespaced effect kinds:

```text
memory.query
memory.write
thoth.verify
github.comment
```

These are still ordinary durable effects. Namespacing improves typed contracts
and status UX; it does not grant plugins new control-flow semantics.

All effects share the same terminal lifecycle:

```text
queued -> claimed -> running -> completed | failed | timed_out | cancelled
```

An effect is claimable only when dependencies are satisfied, policy allows it,
retry/backoff allows it, and any capacity constraint is available. The store may
represent claimable as a query rather than a persisted status.

Scheduling and policy may also mark an effect:

```text
blocked_by_dependency
blocked_by_policy
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

The generic lifecycle predicates are:

```text
succeeds
fails
completes
```

They are shared by every effect kind. The typed payload exposed through a named
binding is defined by the specific effect contract.

For a named effect binding `x`:

```text
after x succeeds   x has the success output type
after x fails      x has the failure output type
after x completes  x has a tagged terminal-output union
```

The language can make effects feel direct:

```armature
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

The same Armature program can run in different environments if all required
capabilities and profiles are satisfied.

## Script Declarations

A source program should be explicit about non-built-in authority:

```armature
requires capability issueTracker

agent worker {
  profile "repo-writer"
  capacity 5
}
```

Built-in effects such as `tell`, `coerce`, `askHuman`, and Docket operations
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
review-only
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

Armature must not claim stronger enforcement than a provider can supply.

Each provider binding reports:

```text
requested authority
enforced authority
best-effort gaps
warnings
```

If a governed environment requires strict enforcement and the provider cannot
enforce the requested boundary, the effect is blocked before execution.

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
