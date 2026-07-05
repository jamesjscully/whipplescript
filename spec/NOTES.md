# Research & design notes — index and genre discipline

Research/design notes are a third document genre beside **trackers**
(registered in `TRACKERS.md`, hold only open intent, gated) and **decision
records** (`decision-records/`, status lifecycle). This index governs the
genre the same way `TRACKERS.md` governs trackers.

## Genre principles

1. **A research note holds design intent** — the current best understanding
   of a target, its settled-in-principle decisions, and its open forks. It
   is *not* reality (code + git + gates) and *not* a commitment (an ADR).
2. **On ADR, supersede — don't excerpt.** When a piece of a note graduates
   to a decision record, the DR is written fresh (model-first/greenfield
   applies to documents too), and the note gains a `superseded-by` pointer
   for that piece. Notes must not silently drift once building starts.
3. **Cross-document references use section *titles*, not numbers.** Section
   numbers churn as notes grow by passes (the experimentation note
   renumbered three times in one week); titles survive. Within-document
   numeric refs are fine.
4. **Status vocabulary:** `active-design` (still being shaped in
   conversation) · `feeding-ADR` (settled enough that the next step is a
   decision record) · `superseded` (DRs own the content; note kept for
   provenance).
5. **Shared questions get exactly one owning document**; other notes point
   at the owner instead of restating.

## Index

| note | status | scope |
|---|---|---|
| `experimentation-subsystem-research-note.md` | active-design (v6 + reflection pass) | Evidence/evaluation subsystem: surface (`gauge`/`mark`/`suppose`/`settle`/`evidence`), kernel foundation, duality, slice-hash identity, ledger, identification + quasi-experimental layers, build spine. |
| `improve-design-note.md` | active-design | `whip improve`: multi-objective campaigns, dominance invariant, licensed crossover, holdout policy, campaign records. |
| `versioned-workspace-research-note.md` | active-design | Whip-native VCS: branches, certified merge (per-plane decomposition, confluence, trains), reconciliation daemon (nose = slice), workstreams (imported from un-tie), mediated MVCC, scope semantics (make/know/owe/say; workspace plane; posture per category × role), materialization + evidence-grade boundary, egress-boundary identity, git bridge. |
| `std-package-ecosystem-shape.md` | feeding-ADR | Standard-package ecosystem shape: mechanism decisions M1–M8 (grammar authorization, provider seams, capability planes, renames, std-as-manifest, versions, DO plane, static checks) + ecosystem decisions E1–E7 + candidate build order; two ⚑ forks awaiting Jack. |
| `compute-plane-design-note.md` | active-design | DO-host compute plane (tracker Phase 8's design): two service classes, workspace-wide delta-kernel result cache, workspace-DO pool + per-turn Class-B containers w/ WS, image digest = env hash, IFC span, priority classes, `whip deploy` sketch. |
| `untie-substrate-replacement-research-note.md` | active-design | Long-term goal: whip replaces Pi AND git in un-tie/gaugewright. Authority split (gaugewright = policy authority via policy epochs; whip = machinery), WhipHarness + archetype-as-package, workspace-for-git mapping, separate stores + three seam disciplines, dependency-ordered plan. Cross-repo. |

(Notes rewrite in place per design pass; prior versions live in git history.)

## Owned shared questions

| question | owner |
|---|---|
| Consent surface for live-touching branches (canary, deliberate reversal) | `improve-design-note.md`, "Canary" section (settled in principle 2026-07-03: consent = re-grant of subject authority over irrevocables; grant object; exposure ladder regen → shadow → canary → adopt; system-chooses-samples / human-chooses-risk; residual = surface syntax + carve-out vocabulary) |
| Retention / GC (branches, scenarios, evidence, campaign records) | `versioned-workspace-research-note.md`, "Open questions" |
| Branch scope semantics (referent taxonomy, workspace plane, subject/instrument posture) | `versioned-workspace-research-note.md`, "Scope semantics" (settled in principle 2026-07-03) |
| Per-door regeneration containment policy | `versioned-workspace-research-note.md`, "Per-door containment policy" + following subsections (settled in principle 2026-07-03, rebased on scope semantics — posture = grants per category × role: replay/divert/live verdicts, counterfactual non-egress invariant, pump audit, chimera coherence + package state surface + two-plane cut; five sub-forks open in its "Open questions") |
| Evidence-plane IFC (labels on scores/scenarios/rationales; readers, doors, pooling) | `experimentation-subsystem-research-note.md`, "The evidence plane's own IFC" (settled in principle 2026-07-03: scope ⊥ clearance, refs-not-content rows, no-new-readers rule, pool-internally/view-per-reader; proposer leakage → improve note "The proposer", policy tiers open; cross-workspace evidence door registered undesigned) |
| Dependency-closure completeness (the central soundness bet, 4 edge kinds + spot-audit tripwire) | `experimentation-subsystem-research-note.md`, "Soundness hazards" |

## Queued design discussions (order of natural pressure)

1. ~~Per-door containment policy~~ — settled in principle 2026-07-03
   (versioned-workspace note, "Per-door containment policy"); five
   sub-forks remain there.
2. ~~Consent surface~~ — settled in principle 2026-07-03 (improve note,
   "Canary": grant object, exposure ladder incl. consent-free shadow
   posture, `design: randomized` canaries); residual = grant surface
   syntax + segment carve-out vocabulary.
3. ~~Evidence-plane IFC~~ — settled in principle 2026-07-03
   (experimentation note, "The evidence plane's own IFC"); residual =
   proposer leakage policy tiers (improve note, deliberately open).
4. ~~Improve campaign surface syntax + holdout policy~~ — both settled
   2026-07-04 (improve note §3 + §8: naming-is-the-partition, std.*
   built-ins, inline targets, `then`/ratchet, `campaign` declaration,
   derived gauges for complex objectives; 20%/floor-2/k=3 holdout).
   Residual: campaign scope (segments/marks, lands with grant carve-out
   vocabulary).
5. ~~Merge engine / reconciliation cadence depth~~ — settled in principle
   2026-07-04 (versioned-workspace note §6.1–§6.2 + §7.1–§7.2:
   per-plane decomposition with instance-as-unit, confluence + merge
   trains, nose-=-slice daemon with quiescence points + conflict
   prediction, and the **workstream tier** imported from un-tie's model);
   residuals listed in its "Open questions".
6. ~~DO tracker Phase 8 compute-plane design pass~~ — done 2026-07-04
   (`compute-plane-design-note.md`; four forks settled by Jack; tracker
   Phase 8 rewritten to build work only).
