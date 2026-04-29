# Armature v0.3 Implementation State

This file is the shared implementation ledger for the sequential Paseo build loop.

Agents may edit this file directly. Keep updates factual and compact. Do not use this
file for design expansion; the authoritative design remains in:

- `spec/armature-v0.3.md`
- `spec/implementation-plan-v0.3.md`

## Rules for Agents

1. Before starting, read this file and the two spec files above.
2. Work only on your assigned slice unless a small shared change is required.
3. Update your row when you start, when you finish, and when you hit a blocker.
4. Keep status values to `pending`, `in_progress`, `blocked`, `done`.
5. Commit your work atomically.
6. Merge your work back to `main` and push `main` before marking `done`.
7. Do not mark `done` unless tests or relevant checks have run, or you recorded why they could not run.
8. Do not introduce workflow DAGs, durable promises, agent graphs, semantic retries, semantic dedupe, built-in adapters, capabilities, Windows support, or `armature plan`.

## Current State

| Slice | Status | Branch / Worktree | Owner | Result |
| --- | --- | --- | --- | --- |
| foundation | done | `armature-foundation` | Codex | merged `d944f1c` to `main` as `a3daf03`; checks: `cargo test`, `npm test` |
| config | pending | `armature-config` |  |  |
| store | pending | `armature-store` |  |  |
| daemon | pending | `armature-daemon` |  |  |
| triggers | pending | `armature-triggers` |  |  |
| cli | pending | `armature-cli` |  |  |
| sdk | pending | `armature-sdk` |  |  |
| recipes | pending | `armature-recipes` |  |  |

## Integration Notes

- The loop is sequential by design for now. Each slice should merge before the next slice starts.
- If a slice cannot merge cleanly, mark it `blocked` and describe the exact conflict or missing prerequisite.
- If the implementation shape changes, update this file with the new boundary before proceeding.

## Slice Notes

### foundation

Done. Added Rust workspace + SDK skeleton, shared core types/IDs/errors, and baseline Rust/TS tests. Merged `d944f1c` into `main` as `a3daf03`.

### config

Pending.

### store

Pending.

### daemon

Pending.

### triggers

Pending.

### cli

Pending.

### sdk

Pending.

### recipes

Pending.
