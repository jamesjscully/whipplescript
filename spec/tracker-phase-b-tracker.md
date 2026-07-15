# std.tracker phase B — the DAG / conflict campaign

Status: active — **B1 hard core COMPLETE 2026-07-15** (commits eac8baf→6463fe3);
only B2 additive items remain. Scope is ADR-0002's deferred phase B
(spec/decision-records/0002-work-tracker-package.md "Phase B1"), taken on
hard-core-first per Jack: the merge-friendly / distributed tracker that
multi-user / multi-agent workflows (gaugedesk's multi-writer workbench) need.
Design settled + decisions recorded in ADR-0002 "Phase B1"; model-first
artifact `models/maude/tracker-merge.maude` (verdicts SSSSSNNNS) landed before
any Rust. The gaugedesk scenario — two clones diverge on a field, merge surfaces
the conflict, a set resolves it — works end-to-end through the CLI.

B1 residual (tiny, non-blocking): `list`/`ready` JSON conflict enrichment
(`show` + `conflicts` cover it); a dedicated `resolve` verb (sugar over `set`);
provenance-based `duplicate_submission` suppression so incremental re-sync is
quiet (needs per-event origin — naturally a B2 rider).

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
- [x] Slice i unit 2 — issue-identity flip (2026-07-15, commit d12629f; store
      tests `events_are_keyed_by_opaque_content_id_not_alias` +
      `rebuild_reproduces_projection_through_the_alias_bridge`): issue id = the
      content-hash of its `issue.created` event (identity = the creation event);
      `WS-N` demoted to a durable clone-local alias in `tracker_aliases`
      (re-aliased on merge); every event `issue_id` + relation payload
      references the opaque id. Projections stay alias-keyed; the CLI/WorkItem
      surface is unchanged (append/query sites resolve alias→content_id at the
      boundary). Co-built with slice iii per the sequencing note below.
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
- [x] Slice iii — merge / import-events (2026-07-15, commits 0d66185 store +
      6463fe3 CLI/self-heal; store tests
      `two_clones_editing_one_issue_merge_to_a_conflict` /
      `two_clones_agreeing_merge_cleanly` /
      `reimport_dedups_and_warns_on_duplicate_submission`):
      `export_events`/`import_events` = set-union of the content-addressed log
      deduped by `event_id` (UNIQUE index added — it was missing, so dedup never
      bit), re-aliasing each newly seen issue, `duplicate_submission` warnings,
      never a silent collapse. `whip issue export` / `import <path|->`. Verified
      two-clone divergence → conflict end-to-end through the CLI. Schema
      self-heals a pre-phase-B `tracker_events`.
- [x] Slice iv — resolution SEMANTICS (2026-07-15): a `set` on a conflicted
      issue parents on all heads → supersedes both forks → clears (store test
      `merge_resolution_clears_conflict`; verified via CLI on two merged
      clones). `whip issue conflicts` reads shipped in slice ii. A dedicated
      `resolve` verb is pure sugar over `set` (same operation) — DEFERRED, no
      new capability; documented as resolution-via-set.
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
