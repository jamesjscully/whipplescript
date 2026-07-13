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
      build items below — AND all four design passes CLOSED with Jack the
      same day, which retires the against-case's "cannot start
      immediately" arm entirely. The decision is still Jack's: ship v1 in
      0.4, or hold it for 0.5.)*

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

- [x] The four open design passes with Jack (discussion-first; each
      closed into the design notes 2026-07-11). **Campaign scope SETTLED 2026-07-11**
      (improve note §3: gauge-layer partition refinements, complement
      auto-guarded; case-feature predicates + pin-time tag rung; grant
      grammar shares the predicate form only; campaign target decoupled).
      **Proposer leakage tiers SETTLED + BUILT 2026-07-11** (improve note
      §7: reader-set-delta keying; campaign-attached `proposer redacted`
      clause + `--redacted-view` flag, tighten-only; `leakage-overlap`
      verbatim-fragment flag on cards; `redactedReflect` invariant added
      to improve-holdout.maude; cleared providers = reader-side machinery;
      no v1 budget, trigger observable via adoption counts). **Local
      utility model SETTLED 2026-07-11** (improve note §12: precedent set
      in the campaign record; monotone Pareto-dominance auto-resolution ON
      BY DEFAULT, locality-bounded, precedent-citing, revocable;
      direction-vector posterior optional and authority-free — EVSI ask
      pricing only; cold start = empty set, so the v1 always-ask build is
      already conformant). **Cross-workspace evidence door constitution
      SETTLED 2026-07-11** (research note §18.2: five ground rules —
      aggregates-only under signatures keyed by published version
      identity; consent = per-workspace/package/gauge grant; k-threshold
      subtraction-safe, floor 3 default 5; tag hygiene crosses,
      fixture-provider excluded; hosted rendezvous behind the open-core
      seam, give-to-get — build deferred until packages-with-gauges are
      real). **ALL FOUR PASSES CLOSED.**
- [x] Build: the tradeoff answer surface + precedent engine — BUILT
      2026-07-13. `whip answer <c>:<k> --accept|--reject|--revoke [--by]`
      (tradeoffs only; double-answer refused until revoked; accept makes
      the candidate adoptable — adopt gate extended); precedents =
      `preference.answered`/`preference.revoked` events in the campaign
      record, folded workspace-wide (`ImproveStore::list_events_of_type`);
      auto-resolution at verdict time by monotone Pareto dominance,
      default-on, locality-bounded to the answer-time band neighborhood,
      gauge-set-exact, conflict-asks, citing the precedent on the card
      (`auto-resolved:precedent` / `auto-rejected:precedent`); cards carry
      per-gauge `direction` so precedents are self-contained. Modeled
      first: `models/maude/improve-precedent.maude` (naive interpolating
      resolver as the bite; revoked/stale precedents grant nothing;
      reject dual). Evidence: engine unit tests
      (`precedent_dominance_grants_and_refuses` + siblings), e2e
      `answered_tradeoff_becomes_precedent_and_auto_resolves`.
- [ ] Precedent residuals: the standing-contradiction reopener (needs the
      parent surface's evidence flags — lands with `settle`/`evidence`);
      the optional EVSI ask-pricer (deferred until ask volume justifies
      it).
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
