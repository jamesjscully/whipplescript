# Memory Plugin

Status: draft

Memory should be a plugin, not a built-in language feature.

The core rule engine should not contain vector search, summarization policy, or
long-term memory semantics. It should provide a plugin surface where memory
systems can register typed effects, fact schemas, skills, and context
providers.

## Capability Surface

Initial memory plugin effects:

```text
memory.query
memory.write
memory.consolidate
memory.delete
```

Initial facts/events:

```text
memory.record
memory.queryResult
memory.writeCompleted
memory.writeFailed
```

Memory facts are plugin projection facts. The memory plugin registers their
schemas, but observations enter an WhippleScript instance through effects/events and
kernel-mediated projection.

## Scopes

Memory records must declare scope:

```text
project
repo
instance
agent
issue
resource
human
```

## Provenance

Every memory record should include:

```text
source instance/effect/run
author actor
created_at
input artifact references
confidence/policy label
retention policy
redaction policy
```

## Retrieval

The first memory plugin should be boring:

```text
SQLite
FTS5
tags
scope filters
explicit references
```

Vector or embedding-backed retrieval can be added later as a provider option,
but it must still produce auditable records explaining what was retrieved and
why.

## Agent Context

Memory may provide context before an agent turn:

```whipplescript
recall memory for issue as context
tell worker with context context "..."
```

Lowering:

```text
memory.query effect
completion fact
agent.tell effect with explicit context artifact reference
```

Memory context must not be silently injected. The evidence store records each
query and the selected records.
