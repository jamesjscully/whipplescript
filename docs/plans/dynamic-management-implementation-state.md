# Dynamic Management Implementation State

This file is the shared implementation ledger for aligning the implementation
with `spec/dynamic-management-interface.md`.

## Current State

| Slice | Status | Branch / Worktree | Owner | Result |
| --- | --- | --- | --- | --- |
| object-cli | in_progress | `armature-dynamic-object-cli` | Codex | Adding canonical object-oriented commands and alias equivalence coverage. |
| query-wait | pending | `armature-dynamic-query-wait` | - | Add filters, wait commands, and subscribe streams. |
| adhoc-run | pending | `armature-dynamic-adhoc-run` | - | Add tracked ad hoc run execution via `run start` / `exec`. |
| locks | pending | `armature-dynamic-locks` | - | Add force-release, show/list filters, `lock with`, and recovery audit. |
| dynamic-services | pending | `armature-dynamic-services` | - | Add ephemeral dynamic service definitions. |
| dynamic-tasks | pending | `armature-dynamic-tasks` | - | Add ephemeral dynamic task definitions. |
| sdk-docs | pending | `armature-dynamic-sdk-docs` | - | Align SDK and docs after CLI/runtime behavior stabilizes. |

## Slice Notes

### object-cli

In progress. Adding canonical `task`, `service`, `run`, `event`, `trigger`, and
`log` command groups while preserving existing v0.3 aliases.

### query-wait

Pending.

### adhoc-run

Pending.

### locks

Pending.

### dynamic-services

Pending.

### dynamic-tasks

Pending.

### sdk-docs

Pending.
