# 0001: WhippleScript Product Boundary and Standard Packages

Status: proposed

## Decision

WhippleScript should be the product boundary for orchestration. The durable work
tracking idea becomes an optional standard package:

```text
std.tracker  durable issue/work records, lifecycle, ready-work projections
```

The kernel remains the event-sourced orchestration runtime. Packages provide
domain capabilities over that runtime.

## Rationale

Long-running agent workflows need a durable record of work. That record should
be available inside WhippleScript without forcing every user to adopt a separate
project-management tool.

The separation that matters is not repository or binary separation. The useful
separation is conceptual:

```text
runtime      owns events, facts, effects, instances, retries, replay
std.tracker  owns issue/work records and tracker lifecycle
providers    adapt package semantics to local files, GitHub, Linear, Jira, etc.
```

This keeps WhippleScript coherent without inflating the kernel.

## Consequences

- Work-tracking workflows can say `use std.tracker` instead of learning a
  separate local tracker first.
- Package boundaries must be real: users can run simple orchestration without
  adopting unrelated domain systems, and non-coding workflows should not inherit
  coding concepts.
- The old `queue` surface should be revisited as a concise view over work
  records, not necessarily the primary model.

## Risks

- The product may absorb too much if package boundaries are weak.
- A powerful builtin work tracker can drift toward general project-management
  software.

## Open Questions

- Should `std.tracker` be bundled by default but explicitly imported, or
  loaded only when a workflow declares it?
- What naming should replace or supplement `queue` without losing the concise
  authoring surface?
