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
- [x] Slice i unit 1 (2026-07-15, commit 5f0ff94; store test
      `events_form_a_content_hash_chain`): `tracker_events` gains `parents_json`
      + SHA-256 content-hash `event_id` — the tamper-evident Merkle-DAG. Each
      append's parents are the issue's prior heads; SHA-256 (not FNV) for the
      adversarial-integrity property. `event_id`/`heads`/`state_token` are
      content-hashes, hence already merge-stable.
- [ ] Slice i unit 2 — issue-identity flip: issue id = the content-hash of its
      `issue.created` event; `WS-N` demoted to a stored-at-creation alias
      (re-aliased on merge); event `issue_id` + relation payloads reference the
      opaque id. **SEQUENCING (2026-07-15):** moved to land WITH slice iii
      (merge), because the clone-local→merge-stable key only becomes
      load-bearing (and testable — re-aliasing needs a second clone) once
      import exists. Building it earlier would be a large correctness-critical
      rewire tested only indirectly. Slice ii below is key-agnostic (folds "the
      issue's events"), so it does not block on this.
- [x] Slice ii — DAG-aware per-field conflict engine (2026-07-15, commit
      d7e8819; store tests `linear_field_history_never_conflicts` /
      `disagreeing_fork_conflicts_and_is_not_ready` / `agreeing_fork_converges`
      / `different_fields_fork_does_not_conflict` / `merge_resolution_clears_conflict`).
      `set_field` emits `issue.field_set`; `issue_conflicts` computes each field
      from its `bef`-maximal setters over the DAG; ≥2 disagreeing maximal setters
      is a conflict. `heads`/`state_token`/`conflicted`/`field_conflicts` +
      readiness excludes conflicted. Realizes `tracker-merge.maude` (SSSSSNNNS).
      CLI (commit b529e87): `issue set`, `issue conflicts <id>` /
      `--tracker`, and `issue show` (text+JSON) carry the conflict view.
      Residual: `list`/`ready` JSON enrichment (cheap follow-up; `show` +
      dedicated `conflicts` cover the surface now).
- [ ] Slice i unit 2 + Slice iii — merge / import-events (co-built): set-union
      deduped by content-hash + the WS-N→opaque-id flip / re-aliasing; on a
      byte-identical `issue.created` re-submission, a `duplicate_submission`
      WARNING (never a silent collapse). **DECISION CHECKPOINT for Jack** — the
      merge/import semantics + the identity flip are the design-loaded remainder;
      pause here for direction before building.
- [~] Slice iv — resolution: the conflict-clearing SEMANTICS ship (a `set` on a
      conflicted issue parents on all heads → supersedes both → clears; store
      test `merge_resolution_clears_conflict`, and `issue conflicts` reads).
      A dedicated `resolve` verb is sugar deferred to the merge slice (a
      conflict is only creatable via merge, so the verb is exercised there).
- [x] Slice v — optimistic concurrency (2026-07-15, commit 2961f9b; store test
      `optimistic_set_guards_on_state_token`): `whip issue set <id> <field> <v>
      --expect-state-token <t>` applies only if the token still matches, else
      refused with the actual token; check + append share one Immediate tx.

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
