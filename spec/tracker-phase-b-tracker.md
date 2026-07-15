# std.tracker phase B — the DAG / conflict campaign

Status: active. Registered 2026-07-15. Scope is ADR-0002's deferred phase B
(spec/decision-records/0002-work-tracker-package.md "Phase B1"), taken on
hard-core-first per Jack: the merge-friendly / distributed tracker that
multi-user / multi-agent workflows (gaugedesk's multi-writer workbench) need.
Design settled + decisions recorded in ADR-0002 "Phase B1"; model-first
artifact `models/maude/tracker-merge.maude` (verdicts SSSSSNNNS) landed before
any Rust.

Reframe that governs the build: **heads come from MERGE, not internal
branching** — a single SQLite serializes, so forks only arise when two clones'
logs union. The core is a merge (set-union of content-addressed events) + a
deterministic per-field fold. Model and transport are DECOUPLED: B1 builds the
model on the existing SQLite event log; the portable-file / WorkspaceVcs
transport is B2.

## B1 — the hard core (build first)

- [x] Design settled + Maude model (2026-07-15): content-hash event ids,
      issue id = created-event hash, WS-N demoted to a stored alias, per-field
      conflict detection, `heads`/`state_token`, resolution. ADR-0002 "Phase
      B1"; `tracker-merge.maude` (confluence + conflict-soundness +
      non-conflict-merge + resolution + tiebreak negative fixture; SSSSSNNNS).
- [ ] Slice i — event identity + DAG: `tracker_events` gains `parents` +
      content-hash `event_id` (Merkle-DAG, tamper-evident); issue id = the
      content-hash of its `issue.created` event; `WS-N` becomes a
      stored-at-creation alias (local counter), never transaction identity;
      relations/parents reference the opaque id. Store `items.rs`.
- [ ] Slice ii — DAG-aware per-field fold: the projection rebuild folds the
      event DAG, computing each field from its bef-maximal setters; `conflicted`
      / `field_conflicts` / `heads` / `state_token` on every rich issue shape
      (`show`/`list`/`ready`/JSON). A conflicted issue is not ready.
- [ ] Slice iii — merge / import-events: set-union deduped by content-hash;
      on a byte-identical `issue.created` re-submission, a
      `duplicate_submission` WARNING (never a silent collapse).
- [ ] Slice iv — resolution: a `resolve` command appending an event whose
      parents are the conflicting heads → conflict clears; `whip issue
      conflicts [--json]`.
- [ ] Slice v — optimistic concurrency: `whip issue set <id> <field> <v>
      --expect-state-token <t>` applies only if the token still matches.

Per-slice gates: model already covers the invariants; each slice adds direct
store/CLI tests + keeps the frozen `WorkItems` seam and
`project_tracker_issues`/`rule_pass` transparent. No back-compat for the WS-N
identity change (one-way; the opaque id is new primary identity).

## B2 — additive (after B1)

- [~] Full relation-kind set (`parent-of`/`related`/`duplicates`/`supersedes`/
      `discovered-from` + the `hard/soft/order/resource/review/contract/
      discovered` dependency taxonomy compiling to `blocks`).
- [~] Comments / evidence (`comment.added`/`evidence.added` events + projections).
- [~] External providers with claim-strength (GitHub/Linear/Jira adapters
      normalizing into tracker events; weak/advisory claims surface as such).
- [~] DO `rebuild_projection` parity.
- [~] Cross-machine transport: portable `.whip/tracker/tx/**/*.json` and/or the
      WorkspaceVcs integration (the gaugedesk multi-writer exchange). Trigger:
      the first real cross-machine / cross-clone sync.
