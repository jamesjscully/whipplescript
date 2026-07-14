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

- [x] ⚑ **SETTLED 2026-07-14 (Jack): improve/evals SHIPS IN 0.4** — the
      release keeps both banner halves (version control + improve/evals).
      The against-case's "cannot start immediately" arm had already
      retired (all four design passes closed 2026-07-11; the build spine
      completed 2026-07-14: settle DR-0040, estimator/reopener DR-0041,
      priced spend + park/resume, ratchet stages, coerce judges).
      Original question + analysis kept below for the record.
      **Does improve/evals build into 0.4, or does 0.4 re-cut as the
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
- [x] Standing-contradiction reopener — BUILT 2026-07-14 (DR-0041, with
      the estimator): for every ACCEPTED precedent, live rows under the
      accepted candidate's hash after the answer fold into a per-gauge
      contradiction posterior vs the answer-time operating point; flag
      only when the last 3 informative points are non-decreasing AND
      final ≥ 0.8 (sustained, never a spike — modeled in
      `contradiction-reopener.maude`, which deliberately has NO path to
      `revoked`: advisory only, citing the precedent). Surfaced on `whip
      gauges` (⚠ lines + JSON `contradictions`). E2e
      `sustained_live_contradiction_reopens_an_answered_call`.
- [ ] Precedent residuals: the optional EVSI ask-pricer (deferred until
      ask volume justifies it).
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
- [x] `mark` declaration + prefix-cut scenarios + `suppose` — BUILT
      2026-07-13 (DR-0038): `mark "<name>" after <site>` stamping
      `mark.reached` at rule commit; `whip pin ... at <mark>`; the
      clone-and-truncate replay driver (prefix fires nothing —
      `models/maude/prefix-replay.maude`; quiescence-at-cut refusal with
      honest input-replay fallback; revision activation for candidates;
      epoch-bump refires detected + tagged); `whip suppose` (paired
      recorded control, replay accounting, evidence rows); campaign
      evaluation paired at the cut for mark pins. Evidence: parser
      `mark_declaration_*`, e2e
      `mark_pinned_scenario_replays_prefix_and_regenerates_suffix`.
- [x] `settle` (racing + stopping) — BUILT 2026-07-14 (DR-0040): `whip
      settle <gauge> [--certify] [--threshold <k>]` names the decision
      (the gauge's bar; barless gauges refused) and stops itself —
      regenerations race round-robin over the pinned pool, the sound
      certifier walk (strong raises the evidence level, contrary lowers
      floored at zero) closes exactly at the threshold crossing
      (anytime-valid, so crossing-once suffices), and a full pass that
      sets no new evidence high-water mark is an honest `undetermined`,
      never an operator-chosen N. `--certify` tags the crossing
      observation's evidence row and mints `ct-<hash8>`. Modeled first:
      `models/maude/settle-stopping.maude` (naive run-until-N certifier
      as the bite). Evidence: `settle_walk_*` unit tests mirror the
      model; e2e `settle_races_pinned_scenarios_and_stops_at_the_crossing`
      + `settle_exhausts_to_an_honest_undetermined` +
      `settle_refuses_a_gauge_without_a_bar`.
- [x] Belief-update estimator — BUILT 2026-07-14 (DR-0041, design pass
      with Jack: two families, Jeffreys): family A = Bayesian sign test
      over paired bar verdicts (defined from one pair — `suppose`'s
      `p_better`); family B = Student-t posterior on paired deltas
      (≥2 deltas; the evidence cards' `p_better` for continuous gauges);
      `settle` reads out `p_bar_met` (θ vs the chance bar's own rate)
      alongside the certification walk, which stays the stopping rule.
      Self-contained numerics (ln_gamma/betainc/t-CDF) unit-tested
      against reference values; ambient dev rows now stamp their program
      hash (the same-hash warm scope's substrate). E2e:
      `suppose_reads_out_p_better_from_the_paired_sign_test`,
      `settle_reads_out_p_bar_met_alongside_the_walk`.
- [ ] Settle residuals (recorded in DR-0040/0041): identification-first
      + EVSI allocation (`est. to settle`, instrumentation-edit
      suggestions), `--compare <dim>` rankability + `--anchor`,
      warm-started paired posteriors (the transfer-layer step). (Priced
      `--spend-cap` LANDED 2026-07-14 with the price-table build: binds
      on priced regeneration cost, honest `spend-cap-reached`
      undetermined; e2e `settle_spend_cap_cannot_bind_on_unpriced_usage`
      documents the unpriced posture.)
- [x] Replay residuals, the buildable pair (DR-0038) — BUILT 2026-07-14:
      pre-flight refire refusal (a refire-shaped candidate — activation
      needed + a settled prefix effect from a non-consuming rule whose
      trigger facts are live at the cut — is refused BEFORE any suffix
      work and degrades to input replay; e2e
      `refire_shaped_candidate_is_refused_pre_flight` with the identical
      program as prefix-replay control) + consumption-boundary lint
      (`lint.mark_off_consumption_boundary`: exact pre-cut set via the
      site's transitive fact producers, trigger consumed by no rule;
      zero findings across examples/; e2e
      `lint_flags_marks_off_consumption_boundaries`).
- [ ] Replay residuals still open (DR-0038): live-store branch-grade
      suppose (the versioned-workspace containment dependency);
      ambient-row reuse for the recorded control (transfer layer);
      mode-aware identity for the clock hazard (v1 = `clock-sensitive`
      tag).
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
- [x] Improve-loop residuals, the buildable pair (DR-0037) — BUILT
      2026-07-14. **Ratchet execution of later `then` stages**: a stage
      whose ascend gauges ALL carry reach targets the baseline already
      meets advances at invocation time — its achieved levels become hard
      guard floors (refusing regression even inside the band), the next
      recorded stage's tokens re-parse to the active ascend set (targets
      now survive recording as raw tokens), each advance is a
      `stage.advanced` campaign event; a target-less stage is open-ended
      maximization and never auto-advances, and the final stage always
      executes. Evidence: `stage_ratchet_floor_refuses_regression_inside_
      the_band` + `later_stage_tokens_keep_their_targets` unit tests; e2e
      `then_stage_ratchets_and_executes_when_its_target_is_met`.
      **Evaluation recompile hoist**: byte-identical recompiles resolve
      from a process-lifetime cache keyed by (root, source hash) in
      `compile_program_with_root_cached` — `start_workflow_instance` and
      every `run_worker_once` pass previously re-lowered the unchanged
      program per scenario; changed bytes (including via includes, which
      change the bundled source) always recompile, failures never cache,
      warnings cache so callers print exactly what a fresh compile would.
      improve_loop e2e wall time ~2.5s → ~0.9s.
- [x] Priced spend + park/resume — BUILT 2026-07-14 (design pass with
      Jack same day: config-only tables, provider-config `prices` block,
      record-time pricing, per-invocation resume allowance). Price table
      = USD/Mtok per (provider, model), input/output separate; no shipped
      defaults (maintained example in
      `examples/provider-configs/native/native.example.json`); malformed
      entries refuse loudly; unmatched usage records `priced: false` cost
      0 (cap honestly unable to bind). Proposer turns carry
      provider/model/split (`TurnUsage`); spend events store cost at
      record time. `std.spend` gained its priced observable (Σ priced run
      usage in USD; skips with the reason if any usage-bearing run is
      unpriceable — a partial sum must not wear a full one). A cap
      crossing PARKS the campaign as the invocation's terminal event (no
      `campaign.closed` after it — the record folds to `parked`); `whip
      improve --resume <id>` continues it: spec/program/proposer/candidate
      numbering from the record, baseline-hash-guarded, fresh
      per-invocation allowance, `campaign.resumed` event. Pi straggler
      fixed in passing: check-native-provider-configs.sh still required
      the deleted pi-main provider. Evidence: unit
      `price_table_prices_at_record_time_and_refuses_malformed_entries` +
      `spend_reading_is_strict_about_unpriced_usage` +
      `campaign_spec_roundtrips_through_the_record`; e2e
      `spend_cap_parks_and_resume_continues_the_campaign`.
- [x] Coerce-judge scoring + `evidence` naming — BUILT 2026-07-14
      (design pass with Jack: explicit-argument binding, option (b);
      subcommand split). `judge via coerce Assess(input.ticket.title,
      facts.Assessment.priority)` — positional paths, arity/shape
      check-time-validated (drifted signature = check error, never a
      silent rebind); reserved `(record)` for one-parameter coerces;
      `facts.<Class>.<field>` reads the class's LAST recorded fact;
      scoring reuses the runtime's own `build_coerce_call_parts`
      rendering; campaign/settle-time only (like prompt judges); bare
      form parses, honestly unscoreable. `whip evidence [<gauge>]` = the
      gauge evidence view (estimates + contradiction flags); `whip
      evidence instance <id>` = the runtime evidence chain. Evidence:
      parser `coerce_judge_explicit_arguments_parse_lower_and_validate`;
      e2e `coerce_judge_scores_with_explicitly_bound_arguments` (mock
      OpenAI-compatible endpoint, asserts resolved bindings reach the
      rendered prompt) + `evidence_verb_routes_to_the_gauge_view_and_
      instance_subcommand`.
- [x] Evaluation-run spend + parallel evaluation — BUILT 2026-07-14.
      **Judge-turn spend**: prompt/coerce judge turns carry their usage
      (`RunObservation.judge_usage`), recorded per evaluation batch as
      `campaign.spend` events (`what: "judge turns (baseline|K-n[,
      sealed])"`, priced at record time, unpriced turns counted honestly)
      and counting toward the improve cap; settle's cap adds judge cost
      to the priced regeneration cost. Coerce provider label aligned to
      the configured name (`openai-generic`). **Parallel evaluation**:
      the env-var side-store containment replaced by explicit
      `SideStorePaths` (coordination + items) threaded through
      `step_instance` / `WorkerOptions` / child-workflow drives
      (children inherit the parent's overrides); the content store stays
      process-level (content-addressed, pairing-safe). `evaluate_all`
      runs scenarios on a bounded `thread::scope` pool
      (`WHIPPLESCRIPT_EVAL_CONCURRENCY`, default min(cores, 4)),
      preserving input order — the pairing. Settle stays serial (the
      walk is sequential by design). Evidence: e2e
      `judge_turns_are_priced_spend_and_bind_the_settle_cap` +
      `parallel_evaluation_pairs_scenarios_and_records_judge_spend`
      (4 scenarios, forced concurrency 4, sealing engaged, index-aligned
      pairing asserted, spend exactly 3 batches × 2 turns × $5).
- [ ] Improve-loop residuals still open (DR-0037): candidates as
      versioned-workspace branches (v1 stores candidate sources in the
      campaign record; the branch/workstream-tier integration +
      certified-merge rebase is the upgrade).
- [ ] Consent/canary posture: shadow default; canary = RCT. (Untouched by
      v1 — propose-don't-apply means nothing reaches live traffic.)
