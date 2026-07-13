# DR-0038 — Marks, prefix-pinned scenarios, and `whip suppose` (the checkpoint-substrate integration, v1)

Status: accepted + built (2026-07-13; the "mark + scenarios + suppose"
step of the experimentation build spine — research note §18.1 step 2 —
previously excluded from DR-0037's v1). Witness model:
`models/maude/prefix-replay.maude` (a replayed pre-cut effect never fires
again; post-cut recorded outcomes are dropped; suffix sites become
claimable only after the seed completes), extending
`restore-replay.maude` (fold coherence) and `branch-effect-key.maude`
(counterfactual key distinctness). Tests: parser
(`mark_declaration_parses_lowers_and_validates`), e2e
(`mark_pinned_scenario_replays_prefix_and_regenerates_suffix` — mark →
pin-at → suppose under a candidate → campaign over the pinned prefix).
Cross-refs: DR-0037 (the improve v1 this completes), restorable-context
DR (the cut discipline this rides), versioned-workspace research note
(per-door containment, the recorded dependency).

## Problem

DR-0037's scenarios were whole-run input replays: campaign evaluation
re-ran entire workflows per scenario per arm, pairing only at the input.
The design's center — checkpoint-and-regenerate as a literal do-operator,
with paired regeneration on a frozen prefix as a matched-pairs design —
needs named cut points, prefix-pinned scenarios, and a driver that
replays the prefix and re-executes only the suffix.

## Decision

### 1. `mark "<name>" after <site>` — cuts are named positions

A core hand-parsed declaration. The store stamps a `mark.reached` event
(payload: mark, site, committing event id; idempotent on the commit event
id, so a looping site marks each pass) INSIDE the rule-commit transaction
— native and DO alike — so a durable commit can never exist without its
cut coordinate; the event is causation-chained to the commit.
The mark event's OWN sequence is the cut coordinate. Because the event
log is append-only, this makes capture *retroactively ambient*: any
historical position is already a valid cut; marks merely name them.
Validation: unique names; the site must resolve to a rule (flow segments
have lowered to rules by validation time). Not allowed in pattern bodies.

### 2. `whip pin <instance> at <mark> --as <name>` — prefix-pinned scenarios

A mark pin stores the FIRST `mark.reached` occurrence's sequence plus the
canonicalized source-store path alongside the input (the scenario row
widened; input-only pins unchanged). Pinning at a mark the run never
reached refuses with the stamped set.

### 3. The replay driver — snapshot-and-truncate

Regeneration for a mark-pinned scenario, in a disposable store
(hardened by the same-day review pass):

1. **Snapshot** the source store via `VACUUM INTO` — transactionally
   consistent against live writers and WAL sidecars (a plain file copy
   loses un-checkpointed commits). Then `DELETE` the instance's events
   after the cut (a store API documented as counterfactual-store-only —
   the live store's rewind remains `commit_restore`'s marker), which also
   clears the instance's `workflow_invocations` and `inbox_items` rows:
   those are not sequence-keyed and would otherwise let a re-claimed
   pre-cut invoke harvest the RECORDED post-cut child outcome, or a
   re-asked question find its recorded answer. Truncation does not bump
   the restore generation, so every pre-cut identity re-derives
   byte-identically and replayed work dedupes exactly (INV-P1 by exact
   dedup).
2. **Cancel scan** (pre-truncation, against the full log): a `cancels`
   consequence of the marked commit lands after the mark event, so the
   cut would resurrect an effect the recorded run cancelled — detected
   and refused.
3. `rebuild_projections`, then: a folded TERMINAL prefix refuses (a mark
   on a terminal-committing site has no suffix); a live post-cut
   activation that stamped the instance row with a version the truncated
   log no longer contains is reconciled back to the prefix's own version;
   only then is the row reset to running.
4. **Quiescence at the cut**: a folded `running` effect (mid-flight at
   the mark) refuses replay. All refusals so far are PRE-drive: the
   caller degrades to input replay with a `replay-fallback` tag. A
   mid-drive error is a hard failure — falling back after suffix provider
   work ran would execute it twice.
5. Register the candidate program version (idempotent per source/IR
   hash): identical content resolves to the recorded version and drives
   without activation; a different program is revision-activated after
   `analyze_revision_compatibility` (diagnostics refuse → fallback).
6. Drive the suffix with the shared dev-loop under PER-EVALUATION
   side-store containment (one shared scratch would leak coordination
   state between scenarios and arms, breaking the pairing). A suffix that
   never settles (wedged effect, exhausted drive budget) tags
   `drive-incomplete` rather than passing for a finished regeneration.
   `std.latency` is removed from replayed observations (folded prefix
   runs carry fold-time timestamps, so the reading would be fabricated);
   `std.tokens` folds through faithfully.

**The epoch-bump refire residual, handled honestly:** after activation, a
pre-cut NON-consuming rule whose trigger facts are still live re-derives
a distinct effect id (the documented cross-revision semantics) and can
refire — `prefix-replay.maude`'s hazard surfacing on the version axis.
Post-drive detection uses exact identity (a NEW effect id with an
IDENTICAL settled-prefix (rule, kind, input) triple — loop iterations
with fresh inputs are not refires); collisions tag every reading
`replay-refire`. Placing marks at consumption boundaries avoids it; a
pre-flight refusal is the recorded upgrade.

**Pairing is enforced, not assumed:** the campaign verdict consumes only
COMPARABLE scenario pairs — both arms in the same regeneration mode,
neither poisoned by a refire or an incomplete drive. Mismatched pairs are
dropped and counted on the card (`pairs-dropped:N`); a candidate with no
comparable pairs is rejected with that reason, never verdicted on
asymmetric estimands (the mode/refire asymmetry otherwise biases resource
gauges against every textually-different candidate).

### 4. `whip suppose <scenario>` — the what-if verb

One regeneration (never a sample count): prefix replay when pinned at a
mark, input replay otherwise; the recorded run scored in place as the
paired control; output = per-gauge recorded → regenerated with pass
verdicts, replay accounting (events replayed, refires), and tags. Every
suppose lands in the evidence ledger (`regen`, tagged `suppose`).

### 5. Campaign evaluation upgrades transparently

`whip improve` evaluation dispatches per scenario: mark-pinned scenarios
regenerate from the cut on BOTH arms (paired at the cut — the frozen
prefix costs nothing and its variance cancels), input pins re-run whole.
Readings carry `prefix-replay`; programs declaring clock sources carry
`clock-sensitive` (the §9.6 virtual-clock hazard, tagged until
mode-aware identity lands).

## Honest v1 gaps

- Suppose regenerates in a disposable store: branch-grade containment in
  the live store (and egress-door diversion) remains the
  versioned-workspace dependency.
- Refire detection is post-hoc tagging; the pre-flight refusal and
  consumption-boundary lint are follow-ons.
- `mark.reached` stamps inside the rule-commit transaction (native and
  DO), so a durable commit can never lose its cut coordinate; rule-level
  `cancels` still land after the commit, which is why the cancel scan
  refuses rather than replays (in-transaction cancels are the deeper
  follow-on).
- Pin takes the FIRST mark occurrence and says so when the mark fired
  more than once; occurrence selection is a follow-on.
- Evidence rows stamp the driving program's hash over observations whose
  replayed prefix was executed by the recorded program — consistent
  within a campaign (both arms share the prefix), a recorded caveat for
  cross-campaign per-hash aggregates.
- `settle` (racing + stopping) is the next spine step; suppose prints the
  paired comparison without a belief update line until the evidence
  machinery grows the estimator.
- The recorded control is scored at suppose time (judges run against the
  source store), not read from ambient rows — folding ambient evidence in
  is the transfer-layer step.
