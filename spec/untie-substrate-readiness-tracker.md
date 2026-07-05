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
> **`gaugewright-workbench`** (un-tie archived; four-repo split done). Its half
> of the decision is workbench **ADR 0071**; its seam-finishing pass (SUB-0) is
> done — the `Workspace`/`ChatWorkspace` traits and `HarnessSpec`/`HarnessFactory`
> surfaces this tracker's Phases 2-3 must fit are now concrete code there.

Registered in `spec/TRACKERS.md` (status: active).

**This tracker is the canonical build home for the versioned workspace**
(the design notes hold intent-as-design; no other tracker holds its build).
House invariants apply: model-first (coverage AND bite before greenfield
code); per-piece review gate; native gates stay green.

---

## Phase 0 — formal models (the workspace build's model-first gate)

- [ ] Shared program/edit model for merge + transfer: disjoint-slice
      composition over manifests (coverage) + the essential bite — a
      **cross-file semantic conflict that text merge silently accepts and
      slice-overlap rejects** — + an anti-dependence merge bite
      (write-or-consume ∩ read ⇒ no certificate).
- [ ] Confluence claim: pairwise-disjoint edits fold order-independently;
      only overlap-graph components escalate jointly.
- [ ] Workstream invariants imported from un-tie's `workstream.qnt`
      (*membership-gates-autosync*, *archive-rehomes-members*) restated in
      our formal stack, coverage + bite.
- [ ] Branch-distinct effect keys bite: a counterfactual/branch effect that
      dedupes against a real one absent the branch id in the key (silent
      corruption demonstrated, then rejected).

## Phase 1 — versioned-workspace floor (canonical build home)

- [ ] Branch manifests: cuts with **divergent children** (parent pointers,
      not a linear chain); O(1) branch creation over the content-addressed
      store.
- [ ] Virtual working set: sandbox-mediated per-branch file surface,
      copy-on-write.
- [ ] Two-plane consistent cut: substance manifest + workspace-plane
      **high-water positions** (the plane-store enumeration is the pump
      audit walked twice — do both in one pass).
- [ ] Materialize-on-exec + import-back: real scratch dir from a branch
      manifest; diffs imported **atomic, recorded, complete**, keyed by
      effect id, idempotent.
- [ ] Merge engine v1: path-level three-way over manifests with
      provenance-carrying conflict detection + escalation (never fake
      auto-merge); declaration-granularity whip-source merge with slice
      certificates (whole-declaration, fail-closed).
- [ ] Reconciliation daemon v1: silent rebase-down of slice-disjoint
      mainline deltas; quiescence points (terminals, marks, task
      completion); staleness bound; merge-up serialized by the adoption
      lease.
- [ ] Workstream tier: named shared lines + membership (single-valued,
      fail-closed to mainline); certificate-gated auto-admit in-stream;
      boundary-gated promotion; archive re-homes members.
- [ ] Branch-distinct effect keys as a general rule (branch/cut id joins
      program_version + revision_epoch in the idempotency key).

*(Out of scope here: counterfactual postures / subject-instrument grants /
divert plumbing — the experimentation subsystem's build, not needed to
replace git for working branches + workstreams.)*

## Phase 2 — workspace API for external hosts (the git-replacement surface)

- [ ] The mapped operation surface (un-tie's 13 consumed git capabilities):
      init / branch / fork-with-lineage / cut-at-quiescence / merge-probe /
      merge / restore / promote / status+hash / reconcile-list / remove.
- [ ] Review-grade diff: an against-target diff surface consumable by an
      external UI (presentation quality, not just manifest delta).
- [ ] Workspace export/import bundle: manifest + reachable blobs;
      idempotent re-materialization on the receiving side (the handoff
      `STATE_BEFORE_HOME` carrier); **erasure-respecting** (tombstoned
      blobs never travel).
- [ ] Per-blob erasure: tombstone/crypto-erase with retained hashes + the
      honesty downgrade (keep scores/identity, lose payload/replay);
      discharge un-tie's content-erasure invariants (`HISTORY_PRESERVED`,
      `EXPORTED_COPY_NOT_RECALLED`) over the substrate itself.

## Phase 3 — conversational runtime readiness (the Pi-replacement surface, whip half)

- [ ] Chat-shaped instance: a long-lived conversational instance pattern —
      persistent thread = instance event log + restorable context; resumes
      across process restarts.
- [ ] Instance fork surface (chat fork) over branches.
- [ ] **Pi-conformance checklist**: extraction pass over pi-mono 0.73.1
      for harness-owned behaviors — tool ergonomics (read/edit/bash/fetch,
      truncation, result digests), turn lifecycle, abort semantics, thread
      continuation, stderr/error surfacing. *(System-prompt seam, skills,
      project instructions, compaction are OWNED by
      `context-assembly-tracker.md` — pointer, not duplicate.)*
- [ ] Implement the checklist deltas in the owned harness.
- [ ] Multimodal input: image content blocks on the agent-turn (and
      coerce, where sensible) effect surface.
- [ ] Workbench event projection: a stable turn/effect event-stream shape
      (tool dispatch/settle, pending-asks) consumable by an external UI —
      **no token streaming** (settled 2026-07-04).

## Phase 4 — policy plane + auth

- [ ] Policy-epoch consumption: a versioned policy snapshot as ambient
      config (capability grants, provider allowlists, egress policy, label
      clearances); the guarantee report cites the enforced epoch; an epoch
      bump is identity-visible like a provider-profile bump.
- [ ] Auth simplification: provider profiles carry host-resolved
      credentials; whip's own auth shrinks to the thin standalone resolver
      (current env/keychain design becomes the fallback path).

## Phase 5 — store seam (two stores, three disciplines)

- [ ] Referenceable handles for external admission logs: stable event-log
      positions, workspace cut ids, effect ids exposed so a policy
      authority can admit *decisions + pointers* (one-owner-per-fact).
- [ ] Position-pair cut: write-fence + capture of (external scope
      positions, workspace cut id) for cross-store backup/handoff.
- [ ] Seam-contract draft, co-authored with the un-tie side: jurisdiction
      table (which whip side-effects map to admitted commands vs. declared
      whip-internal), idempotent crossing semantics, and a formal model of
      the crossing (their Quint / our stack as fits the
      formal-tool-division).

---

## Not in scope

Un-tie-side work (listed in the research note's plan steps 2–5 as they
apply to that repo); the DO host (desktop un-tie runs whip **native**; the
DO tracker owns its own path — nothing here gates on it); experimentation-
subsystem build (postures, evidence machinery, improve).
