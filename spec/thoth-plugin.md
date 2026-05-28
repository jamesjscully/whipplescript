# Thoth Plugin

Status: draft

Thoth should be a plugin, not part of Whippletree core.

Thoth owns governed repo resource truth:

```text
resource bindings
architecture graph
invariants
ADR capsules
checks
resource leases
deterministic context briefs
```

Whippletree owns orchestration. It should call Thoth through a capability provider
when a workflow needs resource governance.

## Capability Surface

Initial Thoth effects:

```text
thoth.map
thoth.find
thoth.touch
thoth.brief
thoth.lease.acquire
thoth.lease.release
thoth.verify
thoth.show
```

## Agent Workflow

Common sequence:

```text
Loft issue claimed
Thoth resource intent loaded or inferred
Thoth brief attached to agent turn
agent writes code
Thoth touch classifies changed resources
Thoth verify runs required checks
evidence is attached to Whippletree run and Loft note
```

## Policy

Thoth severities map to Whippletree policy:

```text
advisory    context only
gated       required checks before completion
serialized  lease + checks before completion
```

Whippletree should not reinterpret Thoth's resource model. The plugin registers
fact schemas and reports observations/failures through typed effects or events;
Whippletree rules decide how to react to the projected facts.
