# Un-tie substrate readiness tracker — whip-side preparation (replace Pi + git)

**Purpose (open intent):** everything **not yet true in this repo** that whip
needs so it can serve as un-tie/gaugewright's runtime + versioning substrate.
Design SSOT: `spec/untie-substrate-replacement-research-note.md` (goal,
authority split, settled decisions), with
`spec/versioned-workspace-research-note.md` and
`spec/compute-plane-design-note.md` for the substrate designs. **Un-tie-side
work is NOT here** (the `WhipHarness` impl, `crates/workspace`
reimplementation, archetype/definition migration, the policy-epoch producer,
auth flows — all live in the un-tie repo). Milestone note: the research
note's early M1/M2 sequencing analysis is **stale** (Jack, 2026-07-04);
sequencing is decided at build start, not assumed here.

> **Update 2026-07-05:** the gaugewright-side repo is now
> **`gaugedesk`** (un-tie archived; four-repo split done). Its half
> of the decision is workbench **ADR 0071**; its seam-finishing pass (SUB-0) is
> done — the `Workspace`/`ChatWorkspace` traits and `HarnessSpec`/`HarnessFactory`
> surfaces this tracker's Phases 2-3 must fit are now concrete code there.

Registered in `spec/TRACKERS.md` (status: active).

**This tracker is the canonical build home for the versioned workspace**
(the design notes hold intent-as-design; no other tracker holds its build).
House invariants apply: model-first (coverage AND bite before greenfield
code); per-piece review gate; native gates stay green.

---

## Release mapping (v0.2 / v0.3 / v0.4 — sliced 2026-07-05)

This tracker's phases **do not ship together**. Under the release plan
([[project-release-plan]]) they split across two releases — **nothing here is
v0.2**:

- **v0.3 (cloud deployment + owned harness).** Only **Phase 3 minus its fork
  item**: the Pi-conformance surface (tool ergonomics, turn lifecycle, abort,
  thread continuation, multimodal, workbench event projection) and the
  chat-shaped instance. It rides with the DO runtime
  (`durable-object-runtime-tracker.md`) and the owned-harness context assembler
  (`context-assembly-tracker.md`) — those two are the *rest* of the v0.3 owned
  harness and are not in this tracker.
- **v0.4 (version control).** Everything else: **Phases 0–2** (versioned-workspace
  floor + API + per-blob erasure), the **Sequencing fork**, **Phase 3's fork
  item** (needs Phase 1 branches), and **Phases 4–5** (policy epochs/auth + store
  seam) — the full substrate replacement.

Per-heading `· vN` tags below restate this at each phase.

---

## Phase 0 — formal models (the workspace build's model-first gate) · **v0.4**

- [x] Shared program/edit model for merge + transfer: disjoint-slice
      composition over manifests (coverage) + the essential bite — a
      **cross-file semantic conflict that text merge silently accepts and
      slice-overlap rejects** — + an anti-dependence merge bite
      (write-or-consume ∩ read ⇒ no certificate). *(2026-07-10:
      `models/maude/merge-slice.maude` — text-proxy vs certified engine over
      one manifest; bites: cross-file write∩read both directions,
      consume∩read, same-declaration; coverage: same-file disjoint certified
      merge preserving both writes, honest escalation, read∩read allowed.)*
- [x] Confluence claim: pairwise-disjoint edits fold order-independently;
      only overlap-graph components escalate jointly. *(2026-07-10:
      `models/maude/merge-confluence.maude` — nondeterministic fold over the
      full order lattice; bites: no order reaches a different cut, a 3-edit
      overlap chain whose ends are pairwise disjoint escalates jointly
      (neither end ever folds), no wedged normal form.)*
- [x] Workstream invariants imported from un-tie's `workstream.qnt`
      (*membership-gates-autosync*, *archive-rehomes-members*) restated in
      our formal stack, coverage + bite. *(2026-07-10:
      `models/maude/workstream.maude` — plus the note's refinements:
      auto-admit requires a certificate (uncertified push bite) and
      single-valued membership (double-home bite); archive closes the line
      immediately and re-homes every member with a rebase-down pass.)*
- [x] Branch-distinct effect keys bite: a counterfactual/branch effect that
      dedupes against a real one absent the branch id in the key (silent
      corruption demonstrated, then rejected). *(2026-07-10:
      `models/maude/branch-effect-key.maude` — naive vs branch-keyed store
      side by side; corruption demonstrated in BOTH directions (cf absorbed
      into real, real absorbed into cf), rejected under bkey; within-branch
      idempotency retained.)*
- [x] Selective-undo stranding bite: a file-scoped `undo` that a naive
      path filter accepts and the dependency-closure check rejects (a
      later edit read the undone writes). *(2026-07-10:
      `models/maude/selective-undo.maude` — naive path filter accepts the
      stranding plan (demonstrated with the stranding test true); the
      closure check refuses it honestly, and accepts a selection that
      contains its own reader — closure-shaped, not read-shaped.)*
- [x] Stat-cache soundness invariant: import-back must never miss a
      same-size-same-mtime content change (git's racy-timestamp hazard) —
      model the invariant + a bite where a naive fingerprint cache
      silently drops a real change. *(2026-07-10:
      `models/maude/stat-cache.maude` — naive importer drops the racy-granule
      change (demonstrated: skip reachable, import unreachable); sound
      importer re-hashes inside the racy window (trust unreachable there),
      keeps the O(touched) trust path outside it, no spurious imports.)*

## Phase 1 — versioned-workspace floor (canonical build home) · **v0.4**

- [x] Branch manifests: cuts with **divergent children** (parent pointers,
      not a linear chain); O(1) branch creation over the content-addressed
      store. *(2026-07-10: `crates/whipplescript-store/src/branches.rs` —
      `Branches` trait for DO parity; O(1) creation = two pointers off the
      parent head, branch point pinned at creation, divergent children,
      optimistic head advance + atomic `rebase_branch`, fail-closed
      terminal statuses, idempotent create. Integrated + exercised
      end-to-end through `vcs.rs`/`whip branch` (P1h). **DO-host parity
      2026-07-10:** the VCS core is generic over the `Branches` +
      `ContentBlobs` seams; `whipplescript-host-do/src/do_branches.rs`
      implements both over the shared `DoSql` handle (same schema/
      semantics, single-writer posture), `DurableInstance::create` selects
      the branch working set as the instance's file surface when bound
      (cut seed = hash(instance, head) — collision-free across
      rehydrations since minting moves the head), and
      `DurableInstance::bind_branch` is the DO-side birth bind (row +
      `branch.bound` event + live surface swap). DO test: bound instance's
      `file.write` lands on the branch through the same generic
      `WorkspaceVcs`, plain DO file plane untouched, rebind refused.
      Deploy-shell verb routing (index.ts) for bind/branch ops is the
      remaining production-enable step.)*
- [x] Virtual working set: sandbox-mediated per-branch file surface,
      copy-on-write. *(2026-07-10: the surface landed —
      `crates/whipplescript-store/src/working_set.rs`:
      `VirtualWorkingSet` implements the `FileStore` seam over (head
      manifest → ContentStore) reads + a COW overlay for writes/deletes
      (tombstoned outcomes); `manifest()` folds the next cut and feeds
      `merge_manifests` directly (integration-tested); identical bodies
      dedupe. Integrated into `vcs.rs`/`whip branch` (P1h), and per-instance
      effect dispatch is WIRED (P1i, 2026-07-10): `whip dev --branch <id>`
      binds the instance at birth (write-once `branch_instances` table +
      `branch.bound` instance event); the native worker's four `file.*`
      wrappers and the instance-driver arms select `BranchFileStore` (the
      working set with write-through cuts keyed `<effect-id>-f<n>`) when
      bound, `NativeFileStore` otherwise (branch store consulted only if it
      exists — non-VCS workspaces unaffected); `revision_branch_key` reads
      the binding from the instance log, so bound instances derive
      branch-distinct effect keys host-agnostically. End-to-end test:
      branch-bound `file.write` lands on the branch (real root untouched),
      merge propagates to main, unbound runs write natively. **DO parity +
      stat cache DONE 2026-07-10** (see below); materialize/import-back
      consumer wiring landed the same day (item below) — box complete.)*
- [x] Two-plane consistent cut: substance manifest + workspace-plane
      **high-water positions** (the plane-store enumeration is the pump
      audit walked twice — do both in one pass). *(2026-07-10: the plane
      stores gained position surfaces — `Coordination::ledger_positions`
      (per-(owner,ledger) high-water; leases/counters are current-state
      and deliberately positionless) and `WorkItems::event_position`
      (tracker event-log max seq) — native + DO impls; `whip checkpoint`
      and the DO checkpoint op record a `plane.positions` event in the
      SAME quiescent pass as the substance cut, keyed by cut id, and the
      checkpoint report surfaces it. The enumeration doubles as the pump
      audit's store list (telemetry exporter et al. remain the §9.2
      audit obligation). Consumer: regeneration reads the plane AT the
      recorded positions — that machinery rides the experimentation
      build.)*
- [x] Materialize-on-exec + import-back: real scratch dir from a branch
      manifest; diffs imported **atomic, recorded, complete**, keyed by
      effect id, idempotent. *(2026-07-10:
      `crates/whipplescript-store/src/materialize.rs` —
      `materialize_manifest` projects a branch manifest into a real
      scratch (coherence-checked up front; absolute keys mapped to
      relative entries with the mapping restored on import; seeded stat
      cache whose materialization granule stays racy, so a tool's
      immediate same-granule write is undroppable) + `import_scratch`
      (scan_dir diff, every changed blob stored, original keys restored) +
      `WorkspaceVcs::import_diff` (whole diff = ONE head advance, keyed by
      the effect-derived cut id; the crash-retry that finds the head at
      that cut is a no-op success). Wired: a branch-bound instance's raw
      `exec` runs `current_dir(scratch)` and imports back
      (`branch_import` metadata on the effect terminal; scratch cleaned).
      E2E test: exec `cat`s a materialized branch file into a new output,
      the output lands as an effect-keyed cut, unchanged input not
      re-recorded, merge carries the product to mainline. DO-side exec
      materialization rides the compute-plane sidecar (P8) protocol, not
      this seam.)*
- [x] Merge engine v1: path-level three-way over manifests with
      provenance-carrying conflict detection + escalation (never fake
      auto-merge); declaration-granularity whip-source merge with slice
      certificates (whole-declaration, fail-closed). *(2026-07-10: blob
      half = pure `merge::merge_manifests` (structured `PathConflict`,
      identical-outcome reunification, deletes as outcomes, per-item
      escalation). Declaration half =
      `whipplescript-kernel/src/source_merge.rs` (`WhipSourceMerger`)
      behind the store's `SourceMerger` seam: whole-declaration three-way
      over the branch-point base; both-modified-same-declaration
      conflicts unless identical; disjoint changed rules certify iff the
      anti-dependence discipline holds over parser fact footprints
      (write-or-consume ∩ read-or-write-or-consume = ∅, unioned across
      each rule's versions — merge-slice.maude at source granularity).
      Fail-closed walls: non-compiling side, lossy split, modified/
      deleted non-rule decls (added ones are safe — they cannot interfere
      with edits that compiled without them), projection reads, merged
      result must re-compile. E2E: disjoint rule edits to ONE file
      certify through `whip branch merge`; a cross-declaration write∩read
      pair stays an honest conflict.)*
- [x] Text-merge tier: recursive token-level three-way merge for
      non-source text blobs in the both-modified path (word atoms;
      efficiency-only content-defined anchors; certified compose).
      *(2026-07-11 built, design settled with Jack same day — flat line
      diff rejected for prose, sentence atoms rejected for semantic
      dependence; SSOT spec/text-merge-spec.md. Certified layer modeled
      in text-merge-compose.maude (never-fabricate + overlap/proximity
      escalation; 5 solutions + 3 no-solutions in the gate). Impl =
      pure `whipplescript-store/src/text_merge.rs` (char-class
      tokenizer, ephemeral CDC anchor segments over token hashes —
      NOT identity-bearing, unlike chunking.rs — two-tier patience
      diff, compose with proximity dial WHIPPLESCRIPT_TEXT_MERGE_GAP
      default 2 words, provenance-tagged pieces, three-slice conflict
      regions). Wired as stage 3 of `refine_source_conflicts` after
      resolution memory and certified .whip merge; fail-closed:
      deletes, add/add, oversized (8 MiB dial), erased bodies, and
      .whip paths never text-merge (merge-slice bite untouched).
      Conflict rows carry no new fields: region detail is
      deterministically recomputable from the content-addressed
      triple. SAME-DAY FOLLOW-ONS BUILT: the evaluation corpus v1
      (tests/text_merge_corpus.rs — executable failure-mode registry
      with pinned dispositions, zero-bad-merge hard gate, 500-seed
      metamorphic sweep; corpus finding folded into spec §7.2: pure
      insertion pairs at distinct points are exempt from proximity,
      which composes block-reorder-vs-interior-edit to the ideal
      moved-block-carries-the-edit result) and the §12 editor-save
      surface (SUB-6 co-design: `save_with_base` base-carrying CAS
      verb with transient conflicts, `merge_preview`, region-level
      resolution memory — exact triple both orientations, live-only,
      auto-apply, `resolved` provenance; partial-memory-never-fakes-
      clean pinned in the model's memory tier). STILL OPEN, deferred
      by design: move detection (needs its own design pass — weakens
      never-fabricate to atom level; corpus-gated), corpus mining +
      agent red-team (candidate gauge/mark campaign; dials ratchet
      only on corpus evidence), and the gaugedesk fold UI consumer
      (SUB-6 there, gated on the whip pin repoint). CONVERGENCE PORT
      LANDED 2026-07-11: gaugedesk dropped its standalone
      whipplescript-workspace mirror crate and rides WorkspaceVcs
      end-to-end (worktrees via materialize/import-back with persisted
      stat caches; cut-carrying editor saves through `save_with_base`;
      region memory minted from fold resolutions and applied on saves
      AND the `merge_preview` live fold). Two branch-tier verbs grew
      out of it, both implemented for BranchStore + DoBranches and
      op-logged: `retarget` (mutable lineage parent, cycle-guarded,
      pointer-only — branch point stays the three-way base) and
      `merge_keeping` (keep-open merge for long-lived member lines:
      parent adopts the content, the branch rebases onto the merge cut
      with an empty delta; `VcsMergeOutcome::Landed`). Tests
      `retarget_rehomes_merge_target_and_refuses_cycles`,
      `merge_keeping_lands_content_and_line_lives_on`.)*
- [x] Reconciliation daemon v1: silent rebase-down of slice-disjoint
      mainline deltas; quiescence points (terminals, marks, task
      completion); staleness bound; merge-up serialized by the adoption
      lease. *(2026-07-10 complete: decision core `reconcile.rs`
      (TLA-modeled guard order) + executor
      `WorkspaceVcs::reconcile_branch` (blob-disjoint deltas fold in ANY
      phase; contested paths wait for quiescence where certified source
      merges refine and the residue escalates as the ask) + the daemon
      tick `whip branch reconcile` (every live branch; quiescence
      detected from the branch's bound instances — none `running`;
      per-branch JSON report) + merge-up serialized by the ADOPTION LEASE
      (coordination store; key = branch-store identity :: target line, so
      workspaces never cross-contend; contention is a refused normal
      outcome). E2E: a quiescent branch folds mainline's disjoint delta
      and re-points its base (second pass = up_to_date); its own
      divergence survives. Read-set-aware mid-run silent folds (the
      slicer's read tracking) and a self-scheduling background cadence
      remain future refinements; the tick is worker/operator-invoked.)*
- [x] Workstream tier: named shared lines + membership (single-valued,
      fail-closed to mainline); certificate-gated auto-admit in-stream;
      boundary-gated promotion; archive re-homes members. *(2026-07-10
      complete: the membership/lifecycle store (`workstreams.rs`,
      single-valued by schema, archive re-homes atomically, active-only
      names) + the sync mechanics (`WorkspaceVcs::sync_to_line` — fold
      the line down, certified merges refine, then advance the line and
      re-point the member fully in sync, NEVER adopted) + the daemon
      wiring: `whip branch reconcile` auto-admits quiescent stream
      members greedily (certificate-gated; a colliding contribution
      isolates per-contribution without moving the line) and
      `whip stream promote` is the boundary hop to mainline under the
      adoption lease; `whip stream create/join/leave/archive/list/show`
      is the surface. E2E: two members admit disjoint work, the sibling
      pick-up converges on the next tick, a collision isolates, promotion
      lands the line on main with the stream surviving, archive re-homes
      all members.)*
- [x] Branch-distinct effect keys as a general rule (branch/cut id joins
      program_version + revision_epoch in the idempotency key).
      *(2026-07-10: `rule_pass::revision_branch_key` — the composed
      revision-axis component every derived key carries (commit keys,
      effect ids, autofail/diagnostic keys, and everything downstream that
      derives from effect ids). The current branch/cut ref is the restore
      lineage (`main.r<generation>`, one head per `context.restored`
      marker); a workspace-branch id joins the same seam when instances
      are born on branches. Generation 0 = bare epoch, so every existing
      store derives byte-identical keys.)*
- [x] **Effects-plane restore fold (discovered 2026-07-10):**
      `list_effects` does not fold the `context.restored` marker, so a
      re-executed suffix sees the orphaned segment's effect rows and
      silently adopts their outcomes instead of re-offering the rules.
      Key-distinctness (above) removes the dedup half of the hazard; the
      visibility half needs the replay-frontier decision (which orphaned
      effects are legal replay vs which must re-execute).
      *(2026-07-10 closed: RC-4b now applies to the effects plane — an
      effect is live iff its `created_by_event_id` survives the marker
      fold (`live_event_ids_on`, native + DO `do_live_event_ids`),
      filtered in BOTH read paths: `list_effects` (rules re-offer instead
      of adopting orphaned outcomes) and `claimable_effects` (the worker
      is never handed an orphaned pending effect). No-marker instances
      take the fast path (no filtering). Replay-frontier semantics:
      effects at-or-before the cut stay live (replay of the past is
      always valid); everything on the orphaned suffix re-executes with
      branch-distinct keys. Regression test covers hide + claim-hide +
      post-restore liveness; full suite green (no v0.3 restore
      behavior change for terminal-state rewinds).)*
- [x] **Content-defined chunking** for large blobs (vw note §10.1):
      FastCDC-style chunk trees, file identity = stable Merkle root
      (nothing upstream re-keys); whole-blob below threshold; erasure at
      chunk level with retained root. *(2026-07-10 complete: pure core
      (`chunking.rs`, frozen gear table, dedup property-tested) +
      `chunk_str` (FastCDC snapped to UTF-8 boundaries — the string
      tier's own frozen identity) + the store tier
      (`ContentStore::put_chunked/get_chunked` — below-threshold IS plain
      `put`, chunks dedupe across roots via a junction table) +
      chunk-level erasure (`erase_chunks`: drops this root's chunk
      BODIES except those shared with a live sibling root — fail-closed
      sharing; the root row + chunk hashes + size are the retained
      honesty-downgrade handle). Object-tier packing/spill rides the DO
      P7/P8 work, not this seam.)*
- [x] **Stat cache** in the virtual working set: mtime/size/inode
      fingerprints so import-back is O(touched) not O(tree); implements
      the P0 soundness invariant. *(2026-07-10:
      `crates/whipplescript-store/src/stat_cache.rs` — `scan_dir` over a
      previous `StatCache` (size+mtime fingerprints + the scan STAMP);
      trust iff fingerprint matches AND entry mtime strictly older than
      the stamp; racy-granule entries re-hash. Tests: O(touched) trust
      path observable via trusted/rehashed counts, and THE bite — a
      same-size change with mtime forced back into the recorded granule
      is detected, never dropped (stat-cache.maude at runtime). Deletions
      reported, nested walk, JSON round-trip. Consumer =
      materialize-on-exec import-back when that slice lands.)*
- [x] **Partial materialization**: manifest-subset materialization from
      slicer-computed input closures; fetch-on-demand; clear failure at
      disk bounds (required for Class-B sidecars; optional-lazy on
      desktop where reflinks apply — fallback matrix: reflink APFS/btrfs/
      XFS, copy on ext4/NTFS, hardlink only for read-only inputs).
      *(2026-07-10: `materialize_manifest_subset` — include-set filter
      (the slicer's input-closure seam; full materialize delegates to
      it) + `MaterializeLimits` byte budget refusing CLEARLY before any
      write; import-back over a subset scratch is naturally partial
      (un-materialized paths are neither phantom removals nor touchable
      by the diff — tested). Honest residuals: fetch-on-demand for
      surprise reads is the DO Class-B pull-missing protocol's seam (a
      native subset miss is an ordinary file-not-found), and the
      reflink/copy fallback matrix is a later desktop optimization —
      subset+copy is the correct baseline everywhere.)*

- [x] **Stable change identity (dual identity, jj import)**: an
      edit/intent id assigned at creation, stable across rewrites,
      carried alongside content hashes by selections and transport;
      merges reunify on either; intent-identical/content-divergent =
      detected divergent change (both versions surfaced). *(2026-07-10:
      the `cuts` table (native + DO parity via the `Branches` trait —
      `record_cut`/`cut_change_id`) maps every cut to its CHANGE id: a
      write mints change = cut (a new intent), a rebase rewrites the cut
      but inherits the change, and sync/merge transport records the
      admitted cut under the member head's change — the eventual full
      merge recognizes it as the same change (content-identical
      reunification was already native to content addressing).
      Intent-identical/content-divergent at a sync boundary returns
      `SyncOutcome::DivergentChange` with both manifest hashes — both
      versions surfaced, nothing merged silently. Tested end to end at
      the store level. Residuals: an explicit `amend` verb (today every
      write is a new intent) and the divergence-presentation UX ride the
      Phase 2 selection algebra.)*

*(Out of scope here: counterfactual postures / subject-instrument grants /
divert plumbing — the experimentation subsystem's build, not needed to
replace git for working branches + workstreams.)*

## Sequencing fork (⚑ OPEN — decide at build start) · **v0.4**

- [ ] ⚑ **Git-backed workspace API first?** (jj's adoption path: semantics
      over a git object-store backend, native store as the later swap.)
      Ship the whip workspace API in gaugedesk over git
      objects — workstreams, certified merge on whip-legible content,
      selective verbs, the no-destructive surface all live early — then
      flip the backend when Phase 1 lands. Interim degradations, tagged:
      no per-blob erasure, no chunk transfer, no two-plane-cut elegance.
      Trade: earlier product value + de-risked storage swap behind a
      stable API, vs. a temporary second substrate.

      *Analysis (2026-07-10, decision still Jack's — the fork is
      whip-build-order independent, so the Phase 1 floor build proceeds
      under either branch):*

      **For git-backed first.** (a) The workspace API (Phase 2's 13
      operations) gets exercised by a real consumer (gaugedesk) months
      earlier, so its shape hardens before the native store freezes any
      mistake in. jj is the existence proof that semantics-over-git-objects
      works and that the backend swap can be deferred indefinitely without
      the surface changing. (b) Workstreams, certified merge on
      whip-legible content, and the no-destructive verbs are the *product*
      value; per-blob erasure and chunk transfer are substrate value —
      shipping the former early sequences user feedback where it changes
      design. (c) The Phase 0 models constrain the *semantics*, which are
      backend-agnostic — nothing modeled is lost by a git interim.

      **Against.** (a) A second substrate is real carrying cost: the git
      backend needs its own two-plane-cut approximation, its own effect-id
      keyed import discipline, and honest degradation tags — all throwaway.
      (b) The known degradations (no per-blob erasure, no chunk transfer,
      no two-plane-cut elegance) are exactly the properties un-tie's
      content-erasure invariants (`HISTORY_PRESERVED`,
      `EXPORTED_COPY_NOT_RECALLED`) eventually need discharged — a long
      git interim invites erasure-shaped debt. (c) whip's file surface is
      already sandbox-mediated and event-sourced (restorable context, DO
      file plane) — unlike jj, the native store is not greenfield; Phase 1
      mostly composes machinery that exists, so the de-risking premium is
      smaller than jj's situation suggests.

      **Lean:** build Phase 1 native floor now (fork-independent); hold the
      git-backed gaugedesk API for a cheap *subset* trial only if gaugedesk
      needs workstreams before Phase 1 lands. The fork's real payload is
      gaugedesk sequencing, which lives in that repo's court.

## Phase 2 — workspace API for external hosts (the git-replacement surface) · **v0.4**

- [x] The mapped operation surface (un-tie's 13 consumed git capabilities):
      init / branch / fork-with-lineage / cut-at-quiescence / merge-probe /
      merge / restore / promote / status+hash / reconcile-list / remove.
      **DONE 2026-07-10:** `workspace_api.rs` — one serializable protocol
      (`WorkspaceOp` in, `WorkspaceOpOutcome` out, tagged JSON both ways;
      refusals are data, not errors) over new `WorkspaceVcs` verbs
      `merge_probe` (full pipeline preview, moves nothing),
      `fork_with_lineage` (pin at a recorded cut), `cut_at_quiescence`
      (turn-finalize: the head cut IS the cut, recorded idempotently),
      `restore` (re-point to a recorded cut AS A NEW CUT — new intent,
      no rewind), `status_report` (ahead paths/behind/head hash/bound
      instances), `reconcile_list`. Plus the **op log** (`ops` table,
      both hosts): every mutating verb records before/after pointer
      states per touched branch — the record–narrative separation's
      reflog, first-class, and `undo-op`'s substrate — and cuts gained
      provenance columns (`parent_cut_id`, `origin`; in-place migration).
      CLI: `whip branch fork/status/cut/probe/restore/ops/reconcile-list`.
      Protocol + op-log unit tests, CLI e2e
      (`branch_probe_status_restore_and_op_log_surface`).
- [x] Review-grade diff: an against-target diff surface consumable by an
      external UI (presentation quality, not just manifest delta).
      **DONE 2026-07-10:** `diff.rs` — pure, host-agnostic Myers
      O((N+M)D) line diff over the content seam; per-path `DiffEntry`
      (kind, hashes both sides, hunks with unified `@@` coordinates +
      context lines, `to_unified()` rendering) — a side whose payload is
      unavailable (erased/missing) degrades to an honest hash-only entry,
      never a fabricated body. `WorkspaceVcs::diff_against` diffs the
      head against the branch point by default (the review question) or
      an explicit branch/recorded cut. CLI `whip branch diff` (JSON =
      structured entries; human = unified text). Reconstructs-the-target
      property test on the edit script.
- [x] Workspace export/import bundle: manifest + reachable blobs;
      idempotent re-materialization on the receiving side (the handoff
      `STATE_BEFORE_HOME` carrier); **erasure-respecting** (tombstoned
      blobs never travel).
      **DONE 2026-07-10:** `bundle.rs` (`whipplescript.bundle.v1`, one
      JSON document: branch lineage snapshot + head manifest + reachable
      blobs + recorded cuts so CHANGE IDENTITY travels — a bundle-
      imported change reunifies at a later merge). Erased blobs travel
      as tombstones (hash + size, NO body). `export_bundle` /
      `import_bundle` on `WorkspaceVcs`: import is idempotent
      (`AlreadyPresent`) and never clobbers divergent local content
      (`DivergentBranch`, honest refusal). CLI `whip branch
      export [--out] / import`.
- [~] **Chunk-granular transfer**: pull-missing extends from blobs to
      chunks (bundles, hybrid desktop↔cloud, sidecar warm-up become
      rsync-class incremental); object-tier **chunk packing** (pack
      objects indexed by the manifest — internal optimization, never
      user-visible); presigned direct transfer for big artifacts on the
      cloud path.
      **Pull-missing + packing DONE 2026-07-10:** reads became
      tier-transparent (`ContentStore::get` resolves loose rows, packed
      chunks, and reassembles roots — chunk-rooted manifests now work
      everywhere: working sets, diff, bundles); `ContentBlobs` gains
      `chunk_ids`/`put_chunk_root` so bundles carry root STRUCTURE and
      chunks as individual delta-eligible units; negotiation =
      `whip branch digest` (sender ids) → `whip branch have` (receiver
      filter) → `whip branch export --delta-have` (omitted units travel
      as structure; import verifies the receiver actually holds them —
      fail-honest). `pack_root` moves loose chunks into one indexed pack
      blob, reads fall through, and erasure DISSOLVES affected packs
      (survivors re-inline; shared chunks live on) so packed bytes
      actually die. Tests: `delta_bundles_move_only_missing_chunks`,
      `pack_root_is_read_transparent_and_erasure_safe`.
      **REMAINING:** presigned direct transfer for big artifacts on the
      cloud path — rides the production-container/R2 decision (Jack;
      DO-runtime tracker), not buildable until that lands.
- [x] Per-blob erasure: tombstone/crypto-erase with retained hashes + the
      honesty downgrade (keep scores/identity, lose payload/replay);
      discharge un-tie's content-erasure invariants (`HISTORY_PRESERVED`,
      `EXPORTED_COPY_NOT_RECALLED`) over the substrate itself.
      **DONE 2026-07-10:** `ContentBlobs` seam gains `status`
      (Live/Erased/Unknown with retained byte_len) and `erase` (default
      = honest `Unsupported`; native override drops the payload row,
      keeps a `content_erasures` tombstone; chunk roots delegate to the
      P1n fail-closed shared-chunk erasure). `WorkspaceVcs::erase_path`
      + op-log entry; CLI `whip branch erase <branch> <path>`. Both
      invariants discharged by test
      (`erasure_preserves_history_and_respects_exports`): manifests/
      cuts/ops/lineage all read after erasure and reads+diff degrade
      honestly (`HISTORY_PRESERVED`); a pre-erasure bundle keeps its
      payload and re-materializes elsewhere (erasure is local, honestly
      so) while a post-erasure export carries only the tombstone
      (`EXPORTED_COPY_NOT_RECALLED`). Note: content-addressed
      re-ingestion of identical bytes re-creates the payload under the
      same hash — an erase after re-ingestion must be re-issued.
- [x] **Selection algebra + selective verbs** (vw note §7.3): a
      **revset-shaped composable expression language** (∪/∩/− plus
      structural operators: `dependents-of`, `slice-of`, `by-effect`,
      `in-branch`) over provenance events (path/declaration/effect/
      instance/agent/time/branch/semantic-impact); `undo <selection>`
      with the dependency-closure stranding check (slicer's 7th client;
      result = proposal, tagged synthetic until gates/gauges revalidate);
      `transport <selection>` (dual-identity-preserving — reunifies at
      later merge; divergence detected); `adopt --only <selection>`;
      dry-run previews as default.
      **DONE 2026-07-10:** `selection.rs` — grammar `| ~ &` (loosest→
      tightest) + parens; atoms `path(glob)/by-effect/by-origin/
      in-branch/change/cut/since/until/dependents-of(expr)`; universe =
      change-units from cut lineage (cut, path, before→after,
      provenance). `undo <selection>` refuses stranding exclusions per
      selective-undo.maude (path-level dependence = the slicer seam's
      conservative floor; `dependents-of` repairs); the applied undo is
      a counterfactual proposal cut tagged `undo-selection`.
      `transport_selection` carries the change id when the selection is
      ONE change (cherry-pick reunifies) and refuses overlap with the
      target's divergence, moving nothing. `adopt_only` = transport onto
      the parent; the remainder stays live. CLI `whip branch
      select/undo/transport/adopt-only` — dry-run default, `--apply`
      executes. RESIDUAL (grammar is closed under new atoms): `decl()` /
      `slice-of(gauge)` land when the slicer joins as this algebra's
      client; agent/instance atoms need write-provenance rows the effect
      plane doesn't yet stamp per-cut beyond effect-derived cut ids.
- [x] Structured conflict surface: conflict objects (base + both sides +
      provenance) per declaration/path; **conflict-bearing cuts are
      legal, tagged states** (never adoptable while conflicted; work
      proceeds around and atop them); per-item resolution as a
      provenance-carrying edit re-running gates; **resolution memory**
      keyed by content-addressed conflict pairs, with resolutions
      **auto-propagating to descendants** via the reconciliation daemon.
      **DONE 2026-07-10** (modeled first: resolution-memory.maude —
      adoption-gate bite, exact-triple-match bite, work-proceeds +
      propagation coverage): a conflicted reconcile RECORDS each residue
      as an open conflict object (`conflicts` table, both hosts; id =
      content-addressed from branch/path/triple; identical recurrence
      re-opens) and supersedes rows the latest three-way no longer
      produces — the table stays truthful, never a stale block. Writes
      proceed atop the tagged state; `merge` refuses while any row is
      open (belt over the three-way's own re-detection). `whip branch
      conflicts` lists; `whip branch resolve <path> --ours|--theirs|
      --body` is an ordinary provenance-carrying cut that closes the row
      and stores memory keyed by the triple — PLUS the post-resolution
      triple (base, resolution, theirs), so the branch's own next
      reconcile folds while theirs is unchanged and a moved theirs is
      honestly a NEW conflict. Memory consult runs FIRST in conflict
      refinement (before the source merger) on every reconcile tick =
      daemon auto-propagation to descendants; erased resolutions never
      re-materialize (fail closed). Gates re-run at merge time via the
      recompile-checked source-merge path; the recorded resolution cut
      carries `resolve` provenance.
- [x] Provenance archaeology surfaces: write-attribution query
      (blame-superseding), lineage/log views; checkout-free bisect over
      materialized cuts (mostly pre-answered by evidence attribution —
      surface, not machinery).
      **DONE 2026-07-10:** `whip branch attribution` (newest recorded
      unit per live path: cut/change/origin/time — provenance, not line
      ranges), `whip branch log` (recorded cuts with lineage + origins;
      `whip branch ops` is the operation view), `whip branch bisect
      --good --bad --run <cmd>` (binary search over the recorded
      parent-pointer cut chain; each probe MATERIALIZES the cut's
      manifest into a scratch dir and runs the predicate there — no
      branch pointer ever moves). Surfaces over existing machinery, as
      specified.
- [x] **Workspace-operation undo as a front-and-center verb** ("undo the
      adopt", "undo that reconciliation") — the op-log query surfaced as
      jj's most-loved feature, not left as a property.
      **DONE 2026-07-10** (modeled first: op-undo.maude — moved-head
      bite, double-apply bite, append-only + undo-of-undo coverage):
      `whip branch undo-op [<op-id>]` (no arg = the newest op, jj's
      ergonomics) re-points EVERY branch the op touched back to its
      recorded before-state as a NEW compensating op — the log only
      appends; undo-of-undo is the same verb on the compensator. Guard:
      any touched branch not exactly where the op left it is an honest
      refusal (undo the newer ops first), never a lost update. Undoing a
      merge re-points the parent AND re-opens the adopted branch (the
      `restore_branch_state` compensator is the ONLY path through
      terminal statuses); undoing a create closes the head (discard —
      the record survives). Both hosts.

## Phase 3 — conversational runtime readiness (the Pi-replacement surface, whip half) · **v0.3** (fork item → v0.4)

- [x] Chat-shaped instance — **v1 DONE 2026-07-07**: `agent { thread
      continue }` (Managed-only knob; parser + partition diagnostics); a new
      tell seeds from the agent's latest completed-turn transcript (found via
      the agent.turn.completed fact) and appends the new user message; the
      machine now persists the FINAL assistant reply into the transcript, so
      the thread ends on the answer. Persistent thread = the recorded
      transcript events — survives restarts by construction. Branching (chat
      fork) stays v0.4 per the fork item below.
- [x] Instance fork surface (chat fork) over branches. **[v0.4 — needs Phase 1
      branches.]** *(2026-07-10: `whip fork <instance>` — births a NEW
      instance whose agent thread seeds from the source's completed turns
      (`seed_agent_thread`, the host-protocol seam reused) and whose file
      surface is a fresh branch forked at the source line's head, bound at
      birth (`WorkspaceVcs::fork_binding_for_instance`) — both planes from
      ONE quiescent coordinate (a running effect refuses). The fork carries
      the source's program version/input/authority but no event history and
      no `external.started` replay; it reacts to new input (`whip signal`/
      `message`) and its next `thread continue` turn resumes the seeded
      conversation while file effects land on its own line with
      branch-distinct keys (P1b/P1i). Modeled first:
      `models/maude/chat-fork.maude` — the corruption pattern (a quiet
      instance whose own line folds turns its thread never heard of) is
      reachable via all three naive forkers (shared-branch, mid-turn,
      stale-coordinate seed) and unreachable under the sound fork; shared
      bindings unreachable. E2e:
      `fork_seeds_thread_and_mints_own_branch_line`. Residuals: the
      host-protocol `fork_instance_from` stays thread-only (its embedding
      hosts own their branch stores; composition rides the workspace-DO
      broker decision), and a DO-side fork op rides the DO tracker.)*
- [x] **Pi-conformance checklist**: extraction pass DONE 2026-07-07 →
      **`spec/pi-conformance-checklist.md`** (pi-mono @ 351efc8; tool
      ergonomics, turn lifecycle, abort, thread continuation, error
      surfacing, multimodal — each with a PORT/KEEP verdict and the build
      order). Notable KEEPs: whip's content-addressed `recall` beats pi's
      temp files; bounded `max_steps`; no token streaming. *(System-prompt
      seam, skills, project instructions, compaction are OWNED by
      `context-assembly-tracker.md` — closed.)*
- [x] Implement the checklist deltas — **DONE 2026-07-07** (commits
      c776b9d, dbf6263, 23c7823, 436f291, 2d3dc03): owned-loop cooperative
      cancel (TurnStatus::Cancelled + durable-cancellation probe between
      rounds), bounded provider auto-retry (3, transient errors only), read
      line-window/continuation notices + binary guard, grep regex + context +
      line cap, edit robustness (arg tolerance/BOM/overlap), OpenAI is_error
      marker. Residual (small, non-blocking): §2 length-stop tool-call
      handling (fail truncated tool calls with a re-issue message) — needs a
      stop_reason field on ModelReply.
- [x] Multimodal input — **v1 DONE 2026-07-07** (2d3dc03): ImageBlock;
      ChatMessage::User {text, images} with a back-compatible transcript
      shape; tell effect input accepts an `images` array; Anthropic base64
      source blocks / OpenAI input_image parts. Read-tool image path +
      resizing deferred (needs an image codec dep); coerce images deferred
      until a use case.
- [x] Workbench event projection — **DONE 2026-07-07** (2d3dc03): the dev
      NDJSON stream (whipplescript.dev_stream.v0 envelope, monotone
      sequence) gains `dev.evidence` (incremental tool dispatch/settle +
      turn-observation evidence rows, shape-only) and `dev.asks` (the
      pending human-ask set, re-emitted on change) alongside the existing
      dev.events/step/worker/idle/report. No token streaming, per the
      settled decision.

## Phase 4 — policy plane + auth · **v0.4**

- [x] Policy-epoch consumption: **host trust boundary and complete host policy
      constraints published 2026-07-09.**
      The `whipplescript` package now has a library target exposing the exact
      governance/IFC implementation used by the CLI. Embedding hosts call
      `VerifiedEnvelope::verify_signed_text`, which requires and verifies the
      attestation and retains its canonical envelope hash + signer for epoch
      binding; malformed configured envelopes now reject instead of silently
      degrading to ungoverned mode. The published `whipplescript.host.v1`
      protocol now carries that identity on `StartTurnCommand`, every
      `LabeledRuntimeEvent`, and `TurnReceipt`; receipt validation rejects
      command/run/instance/policy mixups, and commands contain resource/provider
      refs rather than bodies or secrets. `HostGovernancePolicy` publishes typed
      capabilities, exact provider/model/base-url/credential refs, and placement
      constraints; the owned runtime enforces them before secret resolution.
      GaugeDesk compiles/signs live epochs and owns credential custody.
      Remaining: drive the owned turn through
      this protocol, enforce the complete capability/provider/egress/clearance
      envelope, and mint the referenced evidence/guarantee artifacts from real
      log positions. **REMAINDER DONE 2026-07-10 (DR-0036 accepted + built):**
      the protocol turn drives the one owned brokered machine
      (`run_brokered_agent_turn`) under full admission — capability (tool
      surface pinned to the package + every resource/provider/placement
      handle governed), provider/egress/clearance (package IFC under the
      verified envelope refuses an uncleared model principal at admission;
      internal-signal and party ceilings as before) — and the receipt's
      artifacts are now real: the resolver witnesses every mediated
      write/edit per segment (durable
      `host.turn.workspace_cut.segment` evidence, so human-suspended turns
      aggregate honestly), `workspace_cut_ref` references the aggregated
      `host.turn.workspace_cut` evidence (explicitly-empty cut for no-write
      turns; DECLINED — absent — when a native command mutated outside the
      mediated surface, per `turn-witness.maude`), and the guarantee report
      gains the envelope-declared `dynamic` section
      (`guarantee writes_within:<scope> <globs>` /
      `no_reads_beyond_grant` / `no_tainted_reads:<class>` in the policy
      DSL + canonical JSON, hash-stable for prior envelopes) evaluated per
      turn under the cited epoch as held/violated/not-evaluated. Test:
      `receipt_workspace_cut_and_dynamic_guarantees_from_witnessed_turn`.
      Residuals: `no_tainted_reads` reports not-evaluated until label-class
      read tainting is witnessed; protocol turns run package-declared
      context with `NoopCompactor` (host owns context per GaugeWright ADR
      0080 — compaction for embedded long threads is a quality follow-up).
- [x] Auth simplification: provider profiles carry host-resolved
      credentials; whip's own auth shrinks to the thin standalone resolver
      (current env/keychain design becomes the fallback path).
      *(2026-07-10: `WHIPPLESCRIPT_PROVIDER_PROFILES` = the policy channel —
      a host-written JSON file mapping the agent's declared `profile` (then
      `default`) to `{provider, model, api_key | api_key_env, base_url?,
      max_tokens?, timeout_secs?}`. Owned turns consult it FIRST
      (`host_resolved_profile_config`); when an entry matches the host owns
      auth and whip acquires nothing; a configured-but-broken entry fails
      the turn honestly instead of silently falling back. Whip's own
      resolver (env → `whip auth` store → codex OAuth — already thin, no
      login flow) is the standalone fallback, and `whip auth status` names
      the active channel. The embedded/protocol path already had
      host-resolved credentials by construction (`SecretResolver` →
      `ResolvedProviderBinding`, resolved after admission). Test:
      `host_resolved_profiles_select_validate_and_fail_honestly`.)*

## Phase 5 — store seam (two stores, three disciplines) · **v0.4**

- [x] Referenceable handles for external admission logs: stable event-log
      positions, workspace cut ids, effect ids exposed so a policy
      authority can admit *decisions + pointers* (one-owner-per-fact).
      `RuntimeEvidencePointer` publishes events, asks/answers, and terminal
      receipts without their evidence bodies.
      *(2026-07-10: `whip handles <instance>` →
      `whipplescript.handles.v0` — latest event position, effect ids with
      status, the workspace binding's line + head/recent cut ids (with
      origin provenance), and the latest position-pair cut; the
      host-protocol receipts already carry the evidence-ref handles
      (`usage_ref`/`guarantee_report_ref`/`workspace_cut_ref`). E2e:
      `handles_expose_pointers_and_checkpoint_records_position_pair`.)*
- [x] Position-pair cut: write-fence + capture of (external scope
      positions, workspace cut id) for cross-store backup/handoff.
      *(2026-07-10: `whip checkpoint <instance> --external-positions
      <json|@file>` — the authority's own scope positions ride INSIDE the
      same fenced `plane.positions` event as the workspace cut id (one
      coherent coordinate, P1q's two-plane pass extended), readable back
      via `whip handles` → `position_pair`. Same e2e.)*
- [x] Seam-contract draft, co-authored with the un-tie side: jurisdiction
      table (which whip side-effects map to admitted commands vs. declared
      whip-internal), idempotent crossing semantics, and a formal model of
      the crossing (their Quint / our stack as fits the
      formal-tool-division). *(2026-07-10: whip's half DRAFTED —
      `spec/store-seam-contract-draft.md`: handles table, jurisdiction
      table (embedded vs standalone per side-effect), idempotent crossing
      semantics; formal model `models/maude/seam-crossing.maude`
      (exactly-once folding over at-least-once delivery; bites: naive
      double-fold + unadmitted bypass; gate-registered SSNNSS). OPEN for
      the un-tie side (named in the draft): their Quint twin, the
      admitted-erasure command shape, handoff-export format details, and
      ratification (each repo records its own half). Aligned with
      GaugeDesk ADR 0080. GaugeDesk's `append_record_with_key` admits each
      pointer at most once and returns its stable position.)*

---

## Not in scope

Un-tie-side work (listed in the research note's plan steps 2–5 as they
apply to that repo); the DO host (desktop un-tie runs whip **native**; the
DO tracker owns its own path — nothing here gates on it); experimentation-
subsystem build (postures, evidence machinery, improve).
