# Content-addressed DAG substrate — research note (pre-ADR)

Status: research note / pre-ADR. Opened 2026-07-15 (Jack). Grew out of the
std.tracker phase B1 build (content-hash Merkle event DAG + per-field conflict
engine, `spec/tracker-phase-b-tracker.md`) and Jack's question: *that design
looks very strong — should it apply to the whole language, not just the issue
tracker?* Refined by Jack to: **upgrade the CORE DATA STRUCTURE from an
append-only log to a content-addressed DAG** — the substrate, not the
tracker's merge policy.

This note records the seam audit that answers "can the four existing
append-only-log sites share one substrate, and what would it look like." It is
evidence-first per the standing preference: *unify on proof of duplication, not
on speculation* ([[feedback-avoid-premature-abstraction]]).

## The one distinction that governs everything

"That design" is two separable ideas glued together:

- **The substrate** — state as an append-only log of immutable, content-addressed
  nodes forming a DAG (dedup, tamper-evidence, merge-stable identity). This is
  *universal* and, as the audit shows, **already reinvented four times** in the
  codebase.
- **The merge policy** — "a field's value is its bef-maximal setters; two
  disagreeing = conflict." This is *domain-specific* (right for a record of
  independent scalar fields, wrong for text / counters / effects) and must NOT
  be generalized.

The upgrade Jack asked for is entirely about the first. The rest of this note is
about factoring the substrate out without flattening the real, load-bearing
differences the audit found.

## The evidence: four sites, one recurring pattern

All four live in `crates/whipplescript-store`, all on SQLite, all are
"append-only log + mutable projections folded from it." Three of the four
literally copy the same FNV-1a content-addressed blob store.

| Axis | **Tracker** (phase B1) | **Workspace VCS** | **Restorable context** | **Runtime effect log** |
|---|---|---|---|---|
| Atomic unit | content-hash event | *cut* (manifest snapshot) + op journal | content blob + checkpoint/restore event | event (effect lifecycle) |
| Append-only spine | `tracker_events` | `content_blobs`+`cuts` immutable; branch **heads mutable** | `events` + `content_blobs` | `events` (+ mutable `effects`/`runs`/`leases`) |
| Content hash | **SHA-256** (event id, incl. parents) | **FNV-1a** (blobs+manifests); `cut_id` **opaque** | **FNV-1a** (blobs+manifest); event id random+seq | **FNV-1a** (idempotency key + exec fingerprint); event id random+seq |
| Structure | **Merkle DAG** | **DAG** (opaque cut ids; branches, merges) | **line + rewind markers** (fold `seq ≤ target`) | flat per-instance **line**; effects form a **DAG** via `effect_dependencies` |
| Identity model | id **= content hash** | **two planes**: content-hash blobs + intent `change_id` that **survives rebase/transport** | `sequence`+`cut_id`; blob = content-hash | idempotency key from **identity inputs, decoupled from payload** |
| Merge policy | per-field maximal-setter **conflict-surface** | layered **certified 3-way** (path → rerere → declaration → region), mediated, first-class conflict rows | **none** (rewind-only) | **none — must reject** (effects aren't mergeable) |
| Dedup | byte-identical events collapse | content-hash blob/manifest convergence | blob dedup + event idempotency | effect idempotency unique index |
| Tamper-evidence | **hash-chain (yes)** | reference-consistency only (FNV + opaque cut ids → **not** tamper-proof) | presence-based, no chaining | none (idempotency keys only) |

Sources: `vcs.rs`/`branches.rs`/`content.rs`/`merge.rs`/`reconcile.rs`;
`lib.rs` (`capture_checkpoint`/`plan_restore`/`rebuild_projections_impl`,
`stable_hash`); `migrations/0001_runtime_store.sql` (`events`, `effects`,
`effect_dependencies`); `rule_lowering.rs`/`trace.rs`. The content-addressed
blob store (`content_blobs`, `id = stable_hash_hex(body)`, FNV-1a) appears in
**at least three separate tables** already — the single strongest argument that
Layer 0 below is deduplication, not abstraction.

## The load-bearing divergences (these become PARAMETERS, not blockers)

The audit's real value: it shows exactly where the four disagree, and every
disagreement is a knob, not a contradiction.

1. **Hash function.** Three sites use FNV-1a (cheap, non-adversarial dedup);
   the tracker uses SHA-256 (adversarial tamper-resistance). Not
   interchangeable. A shared blob store must be **generic over the hasher** — or
   a global SHA-256 switch is a migration (restorable-context compares FNV ids
   byte-for-byte against `file.write.completed` payloads, so a swap isn't a
   drop-in).
2. **Identity ≠ content, sometimes.** The tracker collapses identity onto the
   content hash. But the workspace needs `change_id` — *same intent, new
   content* — to survive rebase; the effect log needs an idempotency key derived
   from identity inputs, deliberately **decoupled** from payload data. "Node id =
   content hash" is therefore NOT universal; identity is a separate axis.
3. **DAG vs dense line.** The tracker and workspace are DAGs; the effect log is a
   flat per-instance sequence whose `whip trace --check` replay *requires* dense
   monotonic `sequence`; restorable-context is a line with rewind markers. But
   **a line is just a DAG where every node has one parent** — the structures
   unify; what varies is whether a site also needs a dense total order.
4. **Merge policy** (confirmed domain-specific): surface / certified-mediated /
   none / must-reject. The workspace already models its merger as a pluggable
   `SourceMerger` trait — the precedent for making this a registry.
5. **Head semantics.** Workspace heads are movable, CAS-guarded branch pointers
   (`advance_head(expected_head → Stale)`); tracker heads are a *derived*
   frontier. Different enough to keep distinct.

## Proposed shape: a layered, parametric substrate

"Content-addressed DAG as the core data structure" lands cleanly as **four
layers**, where the divergences above are the type parameters of the lower ones:

- **L0 — content-addressed blob store, generic over `Hasher`.** `put(bytes) →
  id`, `get(id)`, `INSERT OR IGNORE` dedup, reachability GC + erasure
  tombstones. Collapses the ~3 FNV copies; the tracker's SHA-256 becomes a
  *hasher choice*, not a fork. **Behavior-preserving; highest value, lowest
  risk.**
- **L1 — append-only node log + folded projections + idempotency uniqueness.**
  Append an immutable node under a caller-supplied idempotency key (unique
  index), fold to a materialized projection. Recurs verbatim across
  `tracker_events`, `events`, `cuts`/`ops`.
- **L2 — graph shape (the actual "DAG" upgrade).** A node MAY declare parents by
  id → a **content-chained Merkle DAG** (tamper-evidence + merge-stable heads +
  fork/merge); a site that doesn't gets the **single-parent degenerate case = a
  line**, optionally with a dense per-instance `sequence` for ordered replay.
  Identity is a *separate* axis: content-hash id, or a caller-stable intent id
  (`change_id`), or an idempotency key decoupled from content.
- **L3 — merge / reconcile policy: a registry, not a fixed rule.** Plugins:
  `conflict-surface` (tracker), `certified-3-way` (workspace `SourceMerger` +
  rerere + region), `rewind-only` (context), and **`reject`** (effects). This is
  the layer that must never be unified into one policy.

The upgrade = **make L2's parent-declaring, content-chained shape the default
available spine of the shared log.** Every site that wants dedup +
tamper-evidence + merge-stable identity opts in; the effect log stays a
single-parent line with its dense sequence, as the degenerate case of the same
structure.

## The hard boundary: the effect seam

Effects are side-effecting and **not mergeable** — you cannot reconcile "sent the
email" with "didn't." The mergeable-DAG world must END at the effect seam, which
(usefully) coincides with the IFC egress boundary. The effect log therefore
takes L0+L1+L2(line) but **L3 = reject**: linearizable, conflict-rejecting
append + a per-attempt execution fingerprint for replay/change-detection,
*never* branch/merge. Any substrate that offered effects a merge operation would
be a correctness hazard.

## Reconciling with prior decisions

- **"Mediated MVCC, not CRDT"** (the workspace's deliberate choice, certified
  merge that can be *rejected*): honored — the substrate is merge-policy-agnostic.
  Mediated certified merge is one L3 plugin; conflict-surface another; reject
  another. No forced CRDT auto-convergence anywhere.
- **"Avoid premature abstraction / keep genuinely-different mechanisms
  separate"** ([[feedback-avoid-premature-abstraction]],
  [[project-lease-mechanisms-kept-separate]]): L0 and L1 are extracted *because
  the audit found literal duplication* (three FNV blob stores, three
  log+projection copies), not on a hunch. L2/L3 stay per-site parametric. We are
  deduplicating proven copies, not inventing a framework.

## Staged path (evidence-first — each step is independently shippable)

1. **L0: one content-addressed blob store, generic over the hasher.** Collapse
   the FNV copies; make SHA-256 a hasher choice. Behavior-preserving; regression
   tests are the existing per-site suites. *Buildable now, low blast radius.*
2. **L1: append-log + projection + idempotency trait**, retrofit tracker +
   runtime store onto it.
3. **L2: parent-declaring content-chained spine as an explicit capability** —
   offer it to restorable-context (fork/restore lineage) and workspace
   (`cut_chain`) where it removes bespoke code; the tracker already has it.
4. **L3: merge-policy registry**; consider unifying the two conflict-object
   models (tracker `field_conflicts` vs workspace `conflicts` rows) — *only if*
   the retrofit shows they're the same shape.

## Open decisions for Jack (prose, not a checklist — significant choices)

- **D1 — Hash unification.** *Parametric hasher* (keep FNV where non-adversarial
  and hot, SHA-256 where tamper-evidence matters) vs *standardize SHA-256
  everywhere* (cleaner, but a data migration — restorable-context's byte-for-byte
  FNV comparison is the cost site). **Recommend parametric**: it's the honest
  reflection of the audit (the sites chose different hashes for real reasons) and
  avoids a migration on the critical restore path.
- **D2 — Graduation.** Stay a research note until L0 is specced, or promote to an
  ADR now? **Recommend note now → ADR when L0's interface is drafted.**
- **D3 — Priority.** This is foundation/refactor work; v0.4's banner is already
  improve/evals + version-control. Slot L0 as a hygiene extraction alongside
  version-control work, or defer behind the tracker phase B1 merge slice
  (slice iii)? **Recommend finish tracker B1 (it's the live campaign and the
  proving ground for L2/L3), then take L0.**

## Non-goals (explicitly deferred)

- User-facing **mergeable/CRDT types** or **type-directed merge** (each value's
  type carrying its own merge function). That is the larger language-level
  vision this substrate would *enable*, but it must separately reconcile with
  "mediated, not CRDT" and is out of scope here.
- Changing any site's **merge policy**. This note moves the data structure
  underneath them, not the semantics on top.
