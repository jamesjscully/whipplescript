# Dynamic Management Implementation State

This file is the shared implementation ledger for aligning the implementation
with `spec/dynamic-management-interface.md`.

Agents may edit this file directly. Keep updates factual and compact. Do not use
this file for design expansion; the authoritative design remains in:

- `spec/dynamic-management-interface.md`
- `spec/armature-v0.3.md`
- `docs/plans/dynamic-management-implementation-plan.md`

## Rules for Agents

1. Read the three files above before starting.
2. Work only on your assigned slice unless a small shared change is required.
3. Update your row when you start, finish, or hit a blocker.
4. Keep status values to `pending`, `in_progress`, `blocked`, `done`.
5. Do not revert work you did not make.
6. Keep every dynamic/runtime feature mechanical.
7. Do not introduce workflow DAGs, durable promises, semantic retries, semantic dedupe, agent graphs, or hidden workflow state.
8. Prefer e2e coverage for agent desire paths.
9. Do not mark `done` unless relevant checks have run, or the reason they could not run is recorded.

## Current State

| Slice | Status | Branch / Worktree | Owner | Result |
| --- | --- | --- | --- | --- |
| object-cli | done | `armature-dynamic-object-cli` | Codex | Pushed `d47985d`; integrated into primary checkout; checks: `cargo test -p armature-cli --bin armature`, `cargo test -p armature-cli --test e2e`. |
| query-wait | done | `armature-dynamic-query-wait` | Codex | Added filters, wait commands, subscribe streams, and e2e observation coverage. |
| adhoc-run | done | `armature-dynamic-adhoc-run` | Codex | Added tracked ad hoc `run start` / `exec`; checks: `cargo test -p armature-cli --test e2e adhoc_run_is_tracked_and_cancelable`; `cargo test`. |
| locks | done | `armature-dynamic-locks` | Codex | Added lock force-release/show/with recovery ergonomics. Pushed `82a96ec`; integrated into primary checkout; checks: `cargo test -p armature-cli --test e2e lock_recovery_and_with_lock`; `cargo test`. |
| dynamic-services | done | `armature-dynamic-services` | Codex | Added ephemeral dynamic service definitions; checks: `cargo test -p armature-cli --test e2e dynamic_service_lifecycle`; `cargo test`. |
| dynamic-tasks | pending | `armature-dynamic-tasks` | - | Add ephemeral dynamic task definitions. |
| sdk-docs | pending | `armature-dynamic-sdk-docs` | - | Align SDK and docs after CLI/runtime behavior stabilizes. |

## Integration Notes

- The loop is sequential by default. If parallel implementation is used, avoid overlapping write scopes.
- If a slice cannot merge cleanly, mark it `blocked` and describe the exact conflict or missing prerequisite.
- If the implementation shape changes, update this ledger and the implementation plan before proceeding.
- Keep top-level v0.3 aliases working unless the plan explicitly says otherwise.

## Slice Notes

### object-cli

Done. Added canonical `task`, `service`, `run`, `event`, `trigger`, and `log`
command groups with v0.3 aliases preserved. Pushed slice commit `d47985d` to
`origin/armature-dynamic-object-cli` and integrated it into the primary checkout.
Checks passed in primary: `cargo test -p armature-cli --bin armature`; `cargo
test -p armature-cli --test e2e`.

### query-wait

Done. Added run/trigger correlation filters, `wait` commands for events/runs/
triggers/services, NDJSON `subscribe` streams for events/runs/triggers, `lock
list --expired`, and e2e coverage in `wait_and_subscribe_agent_flow`. Pushed
slice commit `3dd6621` (`Add query wait observation commands`). Checks in slice
and primary: `cargo test -p armature-cli --test e2e
wait_and_subscribe_agent_flow`; `cargo test`.

### adhoc-run

Done. Added daemon-mediated ad hoc finite command execution through `run start`
and `exec`, with `adhoc` run origin, provenance/correlation event linkage,
cwd/env/payload/timeout support, logs, cancellation, and e2e coverage. Checks:
`cargo test -p armature-cli --test e2e adhoc_run_is_tracked_and_cancelable`;
`cargo test`.

### locks

Done. Added daemon-audited `lock force-release --reason`, `lock show`, `lock
with`, stale-token release protection coverage, and expired-lock recovery e2e.
Pushed slice commit `82a96ec`; applied the lock delta into the primary checkout.
Checks in slice and primary: `cargo test -p armature-cli --test e2e
lock_recovery_and_with_lock`; `cargo test`.

### dynamic-services

Done. Added in-memory dynamic service definitions through `service add/remove`,
dynamic service inspection/listing, shared service supervision/log/run
machinery, shutdown cleanup, and lifecycle e2e coverage. Checks: `cargo test -p
armature-cli --test e2e dynamic_service_lifecycle`; `cargo test`.

### dynamic-tasks

Pending.

### sdk-docs

Pending.
