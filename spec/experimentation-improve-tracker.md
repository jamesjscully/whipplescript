# Experimentation / improve subsystem — tracker

Status: active (registered 2026-07-10 at the v0.4 close-out — the release
banner's "improve/evals" half was previously pre-ADR research notes with no
tracker; this makes it OPEN INTENT under tracker discipline). Design SSOTs:
`experimentation-subsystem-research-note.md` (surface + identification +
evidence-plane IFC + consent/canary postures) and `improve-design-note.md`
(the improve loop). This tracker holds the build intent and the release-scope
decision; the notes hold the design.

## Settled ground (design passes with Jack, 2026-07-01 → 2026-07-04)

Not re-opened here: the `gauge` + `mark` surface (pin/suppose/settle/
evidence/why; ambient); identification/quasi-experimental posture;
evidence-plane IFC (§18.2 — scope ⊥ clearance; no-new-readers); GEPA
rejected; HOLDOUT = 20% / floor-2 / k=3; CAMPAIGN surface =
naming-is-partition + a `campaign` declaration; consent/canary = shadow
posture, canary = RCT.

## ⚑ Release-scope decision (Jack — blocks the v0.4 cut definition)

- [ ] ⚑ **Does improve/evals build into 0.4, or does 0.4 re-cut as the
      version-control release with improve/evals as 0.5's banner?**
      *(Update 2026-07-11: the improve/evals v1 build LANDED — DR-0037,
      build items below — which weakens the "cannot start immediately"
      arm of the against-case; the four design passes remain open but v1
      deferred each on settled conservative ground. The decision is still
      Jack's: ship v1 in 0.4, or hold it for 0.5 with the design passes.)*

      *Analysis (2026-07-10; decision is Jack's):*

      **For building into 0.4.** (a) The release plan (settled 2026-07-05)
      names both halves, and the versioned workspace is the substrate the
      evidence plane and campaign partitions were designed to ride — landing
      them together exercises the new store immediately. (b) The settled
      ground above is substantial; the surface design is not starting cold.

      **Against (for re-cutting).** (a) Four design questions are still
      OPEN and Jack-held (campaign scope, proposer leakage, the utility
      model, the cross-workspace evidence door) — under the
      discussion-before-design-choices house rule these need real design
      passes, not autonomous resolution, so the build cannot start
      immediately regardless. (b) The version-control half is complete,
      gated, and independently valuable today; holding it hostage to a
      pre-ADR subsystem inverts the ship-what's-done discipline the 0.2/0.3
      cuts followed. (c) 0.3 set the precedent: scope decisions at the cut
      (Jack, 2026-07-09) trimmed to what was verified.

      **Lean:** re-cut 0.4 as the version-control release; improve/evals
      becomes 0.5's banner with the four open design questions as the first
      agenda. The runbook in `release-checklist.md` stages the mechanical
      steps either way.

## Build items (v1 built 2026-07-11 on settled ground only — DR-0037; the
## four Jack-held passes stay open and v1 deferred each conservatively)

- [~] The four open design passes with Jack (discussion-first; each
      closes into the design notes). **Campaign scope SETTLED 2026-07-11**
      (improve note §3: gauge-layer partition refinements, complement
      auto-guarded; case-feature predicates + pin-time tag rung; grant
      grammar shares the predicate form only; campaign target decoupled).
      Remaining three: proposer leakage tiers; the utility model; the
      cross-workspace evidence door. (v1 postures until each settles:
      propose-don't-apply + review door only; always ask on tradeoffs;
      single-workspace.)
- [x] ADR for the `gauge`/`campaign` surface — DR-0037 (accepted + built
      2026-07-11; `mark` deliberately excluded from v1, below). Word-form
      bars (`at least`/`at most`, `within N percent`) close the
      expect-forms open per the declaration-tokenizer precedent.
- [x] Evidence-plane build, v1 slice: `gauge` declaration (judge via
      coerce|prompt|exec|labels, chance/stat bars, derived-gauge `inputs`),
      built-in `std.spend`/`std.latency`/`std.tokens`, append-only
      evidence rows in the sibling improve store
      (`whipplescript-store/src/improve.rs`), ambient exec/builtin scoring
      from `whip dev`, `whip pin` + `whip gauges`. Evidence: parser tests
      (`gauge_and_campaign_*`), store unit tests, e2e
      `crates/whipplescript-cli/tests/improve_loop.rs`.
- [ ] `mark` declaration + prefix-cut scenarios + `suppose`/`settle`:
      deliberately NOT in v1 (needs the checkpoint-substrate integration;
      v1 scenarios are whole-run input replays, honestly recorded in
      DR-0037).
- [ ] §18.2 IFC refinement on the evidence plane (scope ⊥ clearance,
      no-new-readers for judges): not built; v1 judges are exec/labels
      (no provider flow) or the same native coerce provider.
- [x] Improve loop v1: `whip improve` (naming-is-partition, inline reach,
      `then` stages recorded, `--sacrifice`/`--within`/`--spend-cap`,
      repair mode, declared-campaign adoption), dominance-invariant
      acceptance + holdout sealing (20% / floor-2 / k=3, cumulative wear,
      `unheld-out` below floor) per `models/maude/improve-acceptance.maude`
      + `improve-holdout.maude` (gate-registered), holdout-blind
      fixture/native proposer, campaign records + `whip campaigns` /
      `whip campaign <id>` / `whip adopt` (baseline-hash-guarded).
      Evidence: e2e improve_loop tests; engine unit tests.
- [ ] Improve-loop residuals (recorded in DR-0037): candidates as
      versioned-workspace branches (v1 stores candidate sources in the
      campaign record; the branch/workstream-tier integration + certified-
      merge rebase is the upgrade), ratchet execution of later `then`
      stages, coerce-judge scoring, priced spend accounting (the cap is
      enforced per round + parks the campaign, but token-only usage is
      unpriced), campaign park/resume across invocations,
      `whip evidence <gauge>` naming unification (the runtime
      instance-evidence command owns the name), evaluation efficiency
      (per-scenario recompiles; serial scenario evaluation — hoist a
      precompiled-IR path and pool the arms).
- [ ] Consent/canary posture: shadow default; canary = RCT. (Untouched by
      v1 — propose-don't-apply means nothing reaches live traffic.)
