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
- [ ] Selective-undo stranding bite: a file-scoped `undo` that a naive
      path filter accepts and the dependency-closure check rejects (a
      later edit read the undone writes).
- [ ] Stat-cache soundness invariant: import-back must never miss a
      same-size-same-mtime content change (git's racy-timestamp hazard) —
      model the invariant + a bite where a naive fingerprint cache
      silently drops a real change.

## Phase 1 — versioned-workspace floor (canonical build home) · **v0.4**

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
- [ ] **Content-defined chunking** for large blobs (vw note §10.1):
      FastCDC-style chunk trees, file identity = stable Merkle root
      (nothing upstream re-keys); whole-blob below threshold; erasure at
      chunk level with retained root.
- [ ] **Stat cache** in the virtual working set: mtime/size/inode
      fingerprints so import-back is O(touched) not O(tree); implements
      the P0 soundness invariant.
- [ ] **Partial materialization**: manifest-subset materialization from
      slicer-computed input closures; fetch-on-demand; clear failure at
      disk bounds (required for Class-B sidecars; optional-lazy on
      desktop where reflinks apply — fallback matrix: reflink APFS/btrfs/
      XFS, copy on ext4/NTFS, hardlink only for read-only inputs).

- [ ] **Stable change identity (dual identity, jj import)**: an
      edit/intent id assigned at creation, stable across rewrites,
      carried alongside content hashes by selections and transport;
      merges reunify on either; intent-identical/content-divergent =
      detected divergent change (both versions surfaced).

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

## Phase 2 — workspace API for external hosts (the git-replacement surface) · **v0.4**

- [ ] The mapped operation surface (un-tie's 13 consumed git capabilities):
      init / branch / fork-with-lineage / cut-at-quiescence / merge-probe /
      merge / restore / promote / status+hash / reconcile-list / remove.
- [ ] Review-grade diff: an against-target diff surface consumable by an
      external UI (presentation quality, not just manifest delta).
- [ ] Workspace export/import bundle: manifest + reachable blobs;
      idempotent re-materialization on the receiving side (the handoff
      `STATE_BEFORE_HOME` carrier); **erasure-respecting** (tombstoned
      blobs never travel).
- [ ] **Chunk-granular transfer**: pull-missing extends from blobs to
      chunks (bundles, hybrid desktop↔cloud, sidecar warm-up become
      rsync-class incremental); object-tier **chunk packing** (pack
      objects indexed by the manifest — internal optimization, never
      user-visible); presigned direct transfer for big artifacts on the
      cloud path.
- [ ] Per-blob erasure: tombstone/crypto-erase with retained hashes + the
      honesty downgrade (keep scores/identity, lose payload/replay);
      discharge un-tie's content-erasure invariants (`HISTORY_PRESERVED`,
      `EXPORTED_COPY_NOT_RECALLED`) over the substrate itself.
- [ ] **Selection algebra + selective verbs** (vw note §7.3): a
      **revset-shaped composable expression language** (∪/∩/− plus
      structural operators: `dependents-of`, `slice-of`, `by-effect`,
      `in-branch`) over provenance events (path/declaration/effect/
      instance/agent/time/branch/semantic-impact); `undo <selection>`
      with the dependency-closure stranding check (slicer's 7th client;
      result = proposal, tagged synthetic until gates/gauges revalidate);
      `transport <selection>` (dual-identity-preserving — reunifies at
      later merge; divergence detected); `adopt --only <selection>`;
      dry-run previews as default.
- [ ] Structured conflict surface: conflict objects (base + both sides +
      provenance) per declaration/path; **conflict-bearing cuts are
      legal, tagged states** (never adoptable while conflicted; work
      proceeds around and atop them); per-item resolution as a
      provenance-carrying edit re-running gates; **resolution memory**
      keyed by content-addressed conflict pairs, with resolutions
      **auto-propagating to descendants** via the reconciliation daemon.
- [ ] Provenance archaeology surfaces: write-attribution query
      (blame-superseding), lineage/log views; checkout-free bisect over
      materialized cuts (mostly pre-answered by evidence attribution —
      surface, not machinery).
- [ ] **Workspace-operation undo as a front-and-center verb** ("undo the
      adopt", "undo that reconciliation") — the op-log query surfaced as
      jj's most-loved feature, not left as a property.

## Phase 3 — conversational runtime readiness (the Pi-replacement surface, whip half) · **v0.3** (fork item → v0.4)

- [x] Chat-shaped instance — **v1 DONE 2026-07-07**: `agent { thread
      continue }` (Managed-only knob; parser + partition diagnostics); a new
      tell seeds from the agent's latest completed-turn transcript (found via
      the agent.turn.completed fact) and appends the new user message; the
      machine now persists the FINAL assistant reply into the transcript, so
      the thread ends on the answer. Persistent thread = the recorded
      transcript events — survives restarts by construction. Branching (chat
      fork) stays v0.4 per the fork item below.
- [ ] Instance fork surface (chat fork) over branches. **[v0.4 — needs Phase 1 branches.]**
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

- [ ] Policy-epoch consumption: a versioned policy snapshot as ambient
      config (capability grants, provider allowlists, egress policy, label
      clearances); the guarantee report cites the enforced epoch; an epoch
      bump is identity-visible like a provider-profile bump.
- [ ] Auth simplification: provider profiles carry host-resolved
      credentials; whip's own auth shrinks to the thin standalone resolver
      (current env/keychain design becomes the fallback path).

## Phase 5 — store seam (two stores, three disciplines) · **v0.4**

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
