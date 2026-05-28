# Whippletree v0.3 Implementation State

This file is the shared implementation ledger for the sequential Paseo build loop.

Agents may edit this file directly. Keep updates factual and compact. Do not use this
file for design expansion; the authoritative design remains in:

- `spec/whippletree-v0.3.md`
- `spec/implementation-plan-v0.3.md`

## Rules for Agents

1. Before starting, read this file and the two spec files above.
2. Work only on your assigned slice unless a small shared change is required.
3. Update your row when you start, when you finish, and when you hit a blocker.
4. Keep status values to `pending`, `in_progress`, `blocked`, `done`.
5. Commit your work atomically.
6. Merge your work back to `main` and push `main` before marking `done`.
7. Do not mark `done` unless tests or relevant checks have run, or you recorded why they could not run.
8. Do not introduce workflow DAGs, durable promises, agent graphs, semantic retries, semantic dedupe, built-in adapters, capabilities, Windows support, or `whip plan`.

## Current State

| Slice | Status | Branch / Worktree | Owner | Result |
| --- | --- | --- | --- | --- |
| foundation | done | `whippletree-foundation` | Codex | merged `d944f1c` to `main` as `a3daf03`; checks: `cargo test`, `npm test` |
| config | done | `whippletree-config` | Codex | merged `651cd02` to `main` as `05a3cee`; checks: `cargo test`, `cargo run -q -p whippletree-cli -- --workspace <tmp> config check` |
| store | done | `whippletree-store` | Codex | merged `2e7dd5f` to `main` as `fe7a9a9`; checks: `cargo test` |
| daemon | done | `whippletree-daemon` | Codex | merged `761a878` to `main`; checks: `cargo test` |
| triggers | done | `whippletree-triggers` | Codex | merged `fae27a1` to `main`; checks: `cargo test` |
| cli | done | `whippletree-cli` | Codex | merged `14c99d7` to `main` as `305e7de`; checks: `cargo test`, CLI smoke (`init`, `up`, `tasks`, `run`, `runs`, `logs`, `doctor`, `lock`, `down`) |
| sdk | done | `whippletree-sdk` | Codex | merged `c098d8a` to `main`; checks: `npm test --workspace @whippletree/sdk` |
| recipes | done | `whippletree-recipes` | Codex | merged `8ce7a57` to `main`; checks: `cargo test`, recipe smoke (`init recipe` x5 + `config check`) |

## Integration Notes

- The loop is sequential by design for now. Each slice should merge before the next slice starts.
- If a slice cannot merge cleanly, mark it `blocked` and describe the exact conflict or missing prerequisite.
- If the implementation shape changes, update this file with the new boundary before proceeding.

## Slice Notes

### foundation

Done. Added Rust workspace + SDK skeleton, shared core types/IDs/errors, and baseline Rust/TS tests. Merged `d944f1c` into `main` as `a3daf03`.

### config

Done. Added strict TOML parsing/validation, normalized config hashing, upward-only workspace discovery, and `whip config check`. Merged `651cd02` into `main` as `05a3cee`.

### store

Done. Added XDG state-root resolution keyed by canonical workspace hash, SQLite schema/bootstrap and event/run/log persistence APIs, and isolated `.whippletree/runs/<run-id>` artifact layout. Merged `2e7dd5f` into `main` as `fe7a9a9`.

### daemon

Done. Added Unix-socket daemon runtime, service reconciliation/supervision, process-group cancellation/timeouts, and invalid-reload protection. Merged `761a878` to `main`; checks: `cargo test`.

### triggers

Done. Added manual/schedule/watch/event routing through one event path, per-task admission with inspectable outcomes, trigger/event inspection APIs, and event-aware task artifacts. Merged `fae27a1` to `main`; checks: `cargo test`.

### cli

Done. Added foreground/detached daemon lifecycle, runtime inspection/task/service/run/log/cancel/config/doctor commands, JSON/text rendering, and manual TTL-backed locks. Merged `14c99d7` to `main` as `305e7de`; checks: `cargo test` plus temp-workspace CLI smoke.

### sdk

Done. Added a thin TypeScript SDK over Whippletree CLI/env surfaces with typed client helpers, event/env parsing, manual lock helpers, structured logging, JSON utilities, and package README examples. Merged `c098d8a` to `main`; checks: `npm test --workspace @whippletree/sdk`.

### recipes

Done. Added `whip init recipe <name>` scaffolding for file-watch tests, scheduled status, event source service, event hook task, and explicit named lock starters. Merged `8ce7a57` to `main`; checks: `cargo test`, recipe smoke for all five starters plus `config check`.
