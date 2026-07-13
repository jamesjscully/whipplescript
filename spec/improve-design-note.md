# `whip improve` — Design Note (multi-objective system improvement)

**Status: DESIGN NOTE — v1 BUILT (DR-0037, 2026-07-11).** The settled
ground below is built as the v1 `whip improve` loop
(`spec/decision-records/0037-gauge-campaign-improve-surface.md` records
what shipped and every deliberate v1 gap); the open policies below remain
open. The full design effort behind the verb
reserved in `experimentation-subsystem-research-note.md` ("`whip improve` —
reserved"). Effort opened
2026-07-03 (Jack). Frame and several core decisions are **settled in
principle** (§12 below); surface syntax and a handful of policies are
**open**. Depends on the experimentation subsystem's core (ledger, slicer,
gauges, scenarios, `settle`) — see §11 for sequencing. (Regeneration
side-effect containment, blocking when this note opened, was settled in
principle 2026-07-03 by the versioned-workspace note's per-door policy; §11
records the dependency.)

## 1. The problem, and the rejected port

Once gauges exist, "make it better" wants automation. The obvious move —
port GEPA (reflective evolutionary optimization of a single prompt against a
single objective) — is rejected as the *chassis*:

- **Single-site.** Real systems improve by touching many sites, sometimes
  structure (the rankability/curl signal routing to "branch on message kind"
  is an improvement no prompt rewrite can express).
- **Scalar objective.** Collapsing the gauge vector to a scalar bakes in a
  tradeoff the user never chose; greedy scalar ascent walks off the other
  dimensions — "improved the gauge, broke the system."
- GEPA's Pareto pool is across *task instances* (a diversity mechanism), not
  across *objectives* — it does not answer the multi-objective problem.

What survives the port: **reflective mutation** — using traces and judge
feedback as text to propose targeted rewrites. It becomes our candidate
proposer (§7), fed better reflection material than GEPA ever had.

## 2. The frame — the frontier is an invariant, not an object

Characterizing the Pareto frontier is unaffordable *and unwanted*: the user
needs better points and honest tradeoffs, not a map. Three cheaper things
jointly replace it:

1. **A dominance invariant.** Accept an edit only if it improves the focus
   dimension AND regresses no other in-scope gauge beyond its indifference
   band, with high probability (the ε-constraint method, sequentialized).
   You move toward the frontier by *refusing to move away from it*,
   repeatedly — never by knowing where it is.
2. **Candidate-local probing.** The frontier is only ever measured where
   candidates land. Candidates define the directions; the frontier is probed
   as a side effect of evaluating them.
3. **Lazy preference elicitation.** A candidate that genuinely trades off
   (improves X, regresses Y beyond noise) is a *decision, not an optimization
   step*: it surfaces to the user through the ask door, EVSI-priced, exactly
   like judge anchors. Each answered tradeoff trains a **local utility
   model** — the user's preference direction near the current operating
   point, the only region where their preferences matter. No global utility
   function is ever needed.

Constraints vs. objectives split on existing surface: gauge `expect` bars are
**hard constraints** (feasibility). Priority order: restore violated bars
first, then ascend subject to the dominance invariant, escalating genuine
tradeoffs to the human.

**Indifference bands (settled; amended 2026-07-03):** default = the gauge's
noise floor (minimal detectable effect), user-overridable. Amendment for the
built-in resource gauges (§3): their noise floor degenerates toward zero
(near-deterministic observables), which would escalate every +2-token
candidate as a "tradeoff" — resource gauges default to **relative** bands (a
percentage of the current operating point) instead, equally overridable.
Cheap to revisit.

## 3. Campaigns — a partition of the gauge vector

The general form subsuming all modes: **an improvement campaign is a
partition of the gauge vector into *ascend* / *bound* / *free-within-band***.

- Ascend one, bound the rest — the "focus on a problem" mode.
- Ascend a weighted few — same machinery (scalarized focus), bound the rest.
- **The engineering mode:** bound the quality gauges at their bars, ascend
  (minimize) the resource gauges — "meet the bar x% of the time while
  minimizing time/tokens/spend." The chance-constraint form is *already the
  gauge grammar* (`expect P(due_date_correct) >= 0.9`); this mode adds zero
  expressive surface.

**Cost, latency, and tokens are built-in first-class gauges (settled):**
deterministic observables already in the effect ledger — no judge, no noise,
no anchoring, scored exactly on every run for free, present without
declaration. The dominance invariant covers "improved quality but doubled
cost" uniformly from day one, and the engineering mode gets its objectives
for free.

**Surface syntax: SETTLED 2026-07-04 (Jack).** No modes anywhere: **the
partition is expressed by which gauges you name.** Every gauge has a
"better" direction (quality up; spend/latency/tokens down); named gauges
ascend toward better, unnamed gauges are guarded (held within indifference
bands; declared bars always hard). Built-in gauges are namespaced —
**`std.spend` / `std.latency` / `std.tokens`** — no reserved bare words
squatting on the user's gauge namespace. (Two kinds of money, kept
distinct: `std.spend` is the *workflow's* per-run cost being optimized;
`--spend-cap` bounds what the *campaign* spends searching.)

```
$ whip improve extract_quality
    # ascend extraction; everything else guarded

$ whip improve std.spend
    # the engineering mode with no mode flag: quality bars hard
    # (unnamed-means-guarded), per-run spend descends

$ whip improve extract_quality>=0.9 std.spend
    # inline target: reach 0.9, hold it as a hard bound, minimize
    # spend subject to it (targets ride the gauge's declared
    # expectation form — a chance-shaped bar stays chance-shaped)

$ whip improve extract_quality>=0.9 then std.spend then std.latency
    # lexicographic stages with RATCHET semantics: a completed
    # stage's achieved levels become guard floors for later stages

$ whip improve
    # bare form = repair mode: restore violated bars, touch nothing else

$ whip improve extract_quality --sacrifice verbosity --spend-cap $4
```

Flags speak constraint/risk language: **`--sacrifice <gauge>`** (release
from the guard set; the evidence card says so), **`--within
<gauge>=<band>`** (override a band), **`--spend-cap $`**.

**The `campaign` declaration** carries the full model with the same
grammar at higher ceremony — a CLI invocation is an anonymous single-use
campaign (bare name = `ascend`, comparison = `reach`, `--within` =
`guard`, `--sacrifice` = `sacrifice`); named campaigns are versioned,
diffable design intent, and the campaign record cites them ("adopted from
`release_tuning` v3" — program archaeology extended to objectives):

```whip
campaign release_tuning {
  ascend    extract_quality, reply_quality
  reach     std.latency <= 800ms
  guard     tone within 2%
  sacrifice verbosity
}
```

**Complex objectives are derived gauges, not campaign syntax** (settled
2026-07-04; whip does not grow math capabilities). A composite/business
cost function is a gauge whose judge is the shipped det-validation
pattern fed the gauge vector:

```whip
gauge fulfillment_cost {
  judge via exec "./cost_model.py"
  inputs extract_quality, reply_quality, std.spend, std.latency
}
```

`whip improve fulfillment_cost` then needs nothing new: it is a gauge, so
campaigns/bars/bands/transfer apply unchanged; it is a **delta kernel**
(script hash + input hashes; hermeticity-checked), hence
**Goodhart-resistant by construction** with §8's relaxed guardrails; and
the `inputs` clause hands the slicer its dependency edges. This
**subsumes weighted focus** — scalarization is the trivial derived gauge —
so no weights feature exists, now or later.

Lifecycle: `whip improve` starts a durable campaign (it is a whip);
`whip campaigns` lists; `whip campaign <id>` shows candidate evidence
cards; `whip adopt <candidate>` merges into current mainline; pause and
self-parking exist. Tradeoffs escalate through the ask door and train the
local utility model.

**Campaign scope — SETTLED 2026-07-11 (Jack).** "Improve extraction *for
subject-line cases*" carries the implicit obligation *don't break it
elsewhere* — which is the dominance invariant one level down. So scope
attaches at the **gauge layer, never the campaign layer**: a segment is a
**partition refinement of a gauge** (`extract_quality[subject-line]` and
its complement), and a scoped campaign simply names the refined gauge —
the complement is guarded automatically by unnamed-means-guarded, and
campaign syntax, dominance checking, sealing, and evidence cards need
nothing new (the same move that made composite objectives derived gauges
rather than campaign weights). A campaign-layer filter was REJECTED: it
evaluates only the segment, so out-of-segment regression is invisible —
exactly the failure the acceptance rule exists to refuse; patching that
rebuilds per-segment gauges with worse bookkeeping. The naming surface:
**case-feature predicates over input/recorded facts** are the settled
destination (statically visible, apply to ambient traffic, and draw from
the same well as §12's backdoor adjustment covariates); **pin-time
segment tags** (`whip pin … --segment <name>`) are the progressive-rigor
entry rung (scenario-corpus-only segments, ambient rows unsegmented,
honestly tagged) that may ship first; classifier-judge segments (a
deterministic judge emitting the label) remain an available *idiom*, not
the mechanism. The grant grammar (§9) shares the predicate **form** only
— never the machinery or admission discipline: grants stay fail-closed
and statically checkable, measurement scopes may be advisory; sharing the
form keeps the audit story coherent ("the segment the canary excluded is
the segment the campaign scoped to"). Campaign *target* (running against
a workstream's line rather than mainline) is DECOUPLED from scope — pure
mechanics once candidates-as-branches lands, no vocabulary of its own.

## 4. Why whip affords the multi-objective guard

Three structural subsidies no GEPA-like system has:

- **One trace scores every gauge.** Each regeneration produced for the focus
  gauge is a full trace scored by *all* gauges (ambient judging, adaptive).
  The marginal cost of guarding k objectives instead of one is judging cost
  only — the non-regression check is nearly free **by construction**.
- **Certified blast radii collapse dimensionality.** An edit whose slice
  doesn't reach gauge Y *provably cannot regress Y* — certificate, not
  measurement. The effective Pareto dimension of a candidate is the number
  of gauges in its blast radius, typically small. The global k-objective
  problem factorizes per candidate into a low-dimensional local one.
- **The static gate battery is a free feasibility oracle.** A candidate is a
  program: `whip check`, lint, IFC, the guarantee report all run at zero
  regeneration cost. Candidates that break invariants die before a sample is
  spent.

## 5. Global → local: how local mechanisms fall out

Global objective: *move the gauge vector to undominated better states,
subject to bars, at acceptable spend.* Then:

- **Prefer small-blast-radius edits** — fewer in-radius gauges to verify,
  cheaper non-regression, more evidence carried elsewhere. Search is biased
  toward local moves not by assumption but because certificates make local
  moves cheap to *verify globally*.
- **Disjoint-slice improvements compose, with proof** — the keystone. Two
  improving edits normally risk interaction; if their slices are disjoint,
  non-interference certifies each cannot touch the other's affected gauges:
  **the combination provably preserves both improvements.** Crossover stops
  being a heuristic and becomes a licensed operation with a static
  precondition. Global improvement accumulates from certified local wins.
- **GEPA is the degenerate case** — one site, one dominant in-radius gauge,
  reflective-mutation candidates: recovered as the innermost loop of the
  global frame rather than assumed as the design.
- **The curl signal routes between parameter and structural search.** High
  rankability-curl means no point on the *current program's* frontier
  dominates — the winning move is structural (introduce a branch), not
  textual. The proposer receives that routing signal; `improve` can propose
  program-shape changes GEPA cannot represent.

## 6. Search machinery

- **The lineage DAG is the population.** Candidates are branches, evaluated
  by the same evidence machinery (scenario regenerations, warm-started by
  edit-type priors, cross-variant evidence sharing by hash — all of the
  experimentation note's slice-hash identity rule ("Evidence identity")
  applies automatically, since candidates are just versions). Search state is durable: branches park and resume when ambient
  evidence or a new τ prior changes the picture. Improvement is a **process
  that lives with the workspace, not a run**.
- **Search-born edits are flagged** so they never contaminate the edit-type
  priors τ (which learn from *human* edits); search maintains its own impact
  model.
- **Anytime-valid racing, never generations.** Eliminate a candidate as soon
  as evidence suffices to show it is dominated (e-process racing). No
  generation sizes, no population parameters; the operator's only lever
  remains the spend cap. No surface anywhere accepts a sample count — the
  subsystem's standing principle applies to search unchanged.
- **Multi-fidelity, ending in the real world.** Pinned scenarios are the
  cheap fidelity; survivors escalate; the final fidelity for a would-be
  winner is ambient — canaried adoption watched by drift monitors and, where
  authorized (§9), the deliberate-reversal design. The quasi-experimental
  layer is how an adopted improvement confirms itself in production without
  further spend.
- **The baseline can move mid-campaign — rebase via certified merge (added
  2026-07-03).** Humans keep editing while a campaign runs; a candidate
  branched from a stale parent holds dominance certificates against the old
  baseline, and adopting it naively would revert or collide with the human's
  edit. The versioned workspace already answers this — no new mechanism:
  when mainline moves, live candidates **rebase through the certified-merge
  engine**. Disjoint slices (candidate edit vs. human edit) → both edits'
  certificates survive by the composition theorem, zero re-evaluation;
  overlapping slices → the candidate re-enters evaluation against the new
  baseline (warm-started by its own prior evidence) or dies. Adoption always
  merges into *current* mainline; a campaign can never silently undo a human
  edit. This is the merge engine paying inside the campaign loop — the
  fourth-client relationship running in both directions.

## 7. The proposer

A reflective agent turn, fed the evidence the system uniquely has: worst
slices, failing scenarios, judge rationales, curl structure, τ history
("prompt edits at this site historically move quality ±8%"), and the §5
routing signals. Proposal quality is where LLM intelligence enters;
evaluation rigor is where it is contained. The proposer never sees holdout
material (§8).

**The laundering problem (designed 2026-07-03; policy tiers open).** As a
*reader*, the proposer is governed by the no-new-readers rule like any
agent turn (experimentation note, "The evidence plane's own IFC"). The
deeper issue is its *output channel*: the proposer is gradient-free
learning whose artifact is program source — a prompt improved on
confidential traces can embed fragments of them, making the program a
derived artifact of the data (training-data memorization in miniature,
here at least visible). The honest statement: a proposed edit inherits the
labels of the reflection material that produced it. The response is
**stratified by threat model, not binary**:

- **In-workspace adoption — statistical control.** Source readers are
  principals, not adversaries; leakage is accidental disclosure, and the
  channel is capacity-bounded (an edit of K tokens carries at most K
  tokens about the corpus; prompt diffs are small). A **mutual-information
  heuristic at the review surface** — verbatim/n-gram overlap against the
  labeled inputs as the v1 MI lower bound, compression/perplexity-based
  estimates as the upgrade — shown as a *flag, never a block*. (This is
  the first practical customer of the QIF/graded-transfer horizon the
  duality section recorded: quantitative flow where binary is the wrong
  instrument.)
- **Source-egress doors — the adversary returns.** Package export and the
  git bridge make source world-readable; there the check hardens: clean
  overlap required, strict mode may refuse. Both are already doors.
- **The review door is the second line.** Propose-don't-apply makes human
  adoption an audited declassification act (campaign-record provenance
  makes it auditable); auto-apply policies are unavailable, not merely
  discouraged, for campaigns over labeled reach.

**Policy tiers — SETTLED 2026-07-11 (Jack).** Two decompositions first:
local/cleared provider profiles solve the *reader* problem (which
provider sees traces — already governed by no-new-readers + profile
selection) and are **not a tier of this policy**; and the tier structure
keys off the **reader-set delta** — leakage matters only when the program
source's reader set is wider than the reflection material's. The tiers:
**no delta** (program readers ⊆ data readers, the common single-team
case) → unrestricted, unflagged — nothing can be laundered by embedding
data everyone could already read; **delta present** → default
**unrestricted-with-flag** (the settled statistical-control stance: the
MI/overlap heuristic flags at the review surface, adoption remains the
audited declassification act, egress doors keep their hard checks);
**stratified reflection** (aggregates only, `proposer:redacted-view` tag)
is **campaign-attached** — a `proposer redacted` clause on the campaign
declaration or `--redacted-view` on the invocation, an operator's
explicit call, chosen over label-class-attached engagement so a data
owner's label choice never silently degrades someone else's campaign
quality (the flag may tighten a declared clause, never loosen it).
Built 2026-07-11 with the v1 loop's follow-on pass: redacted reflection
(no scenario names/inputs/traces), the verbatim-fragment overlap flag
(`leakage-overlap` on evidence cards, new-in-candidate fragments only,
flag-never-block), the `redactedReflect` invariant in
`improve-holdout.maude` (no read rule exists — the absence is the
policy). The reader-set-delta keying engages automatically when the
evidence-plane IFC build lands (v1 has no reader sets to compute deltas
from; until then the flag/clause is the whole surface). **No cumulative
leakage budget in v1**, with the upgrade trigger made observable rather
than aspirational: the campaign record counts adoptions per reflection
reach, so "hundreds of adopted edits against one corpus" is a queryable
fact when it happens and the budget can be specified then, with data.

## 8. Ground truth, Goodhart, and the holdout policy

**Ground truth = the anchor concept, generalized (settled).** The
measurement model has one slot for trusted scale-fixing observations; ground
truth fills it at three granularities:

- **human asks** — retail anchors (one comparison at a time, EVSI-priced);
- **labeled datasets** — wholesale anchors: labels keyed to scenarios or case
  facts, arriving via existing machinery (file import → facts; labels
  attached to pinned scenarios). Thin by design: datasets are files the user
  owns; whip reads labels. No dataset DSL, no label-store product (the
  eval-platform non-goal stands). Caveat recorded: labels are trusted *by
  declaration* — an exec judge disagreeing with dataset labels is a
  detectable flag, but "the labels are wrong" remains user-owned.
- **deterministic computations** — self-anchored instruments: `judge via
  exec "<validator>"` reuses the shipped det-validation pattern
  (`exec → Schema`, `WHIPPLESCRIPT_EXEC_ALLOW`); zero judge noise, cheap to
  run densely.

Gauge grammar consequence: `judge via coerce | prompt | exec | labels
<source>` — one line, no new import machinery.

**Goodhart threat model.** Search pressure will find an LLM judge's biases.
Defenses, in layers: gauges with deterministic or wholesale-anchored ground
truth are **Goodhart-resistant by construction** (you can't sweet-talk a
validator) and campaigns on them run with relaxed guardrails — the system
knows the regime from the gauge declaration. For LLM-judged gauges, the
measurement model (experimentation note, "Measurement — scores are
instrument readings") makes gaming *detectable* (a gamed judge diverges from
its anchors — a statistic already tracked), anchor density rises adaptively
during campaigns (EVSI does this unprompted: judge reliability becomes the
bottleneck exactly when search pressure rises), and post-adoption drift
monitors catch what slipped through.

**Holdout policy (SETTLED 2026-07-04, Jack — defaults confirmed):**

1. Per campaign, a random fraction of pinned scenarios (default 20%, floor
   2) is **sealed**: never shown to the proposer, scored only at promotion
   gates (candidate → survivor → proposal).
2. The proposer may see **aggregate** statistics over sealed scenarios
   (pass rate) but never their traces, contents, or judge rationales.
3. Holdouts **rotate per campaign** and carry a wear-out counter: a scenario
   used in k promotion gates (default k=3) is retired from holdout duty and
   refreshed from recent ambient runs (retroactive pinning at marks makes
   this cheap — the cuts were already taken).
4. **Ambient traffic is the true holdout.** Post-adoption ambient scores are
   unseen by the proposer by construction; the quasi-experimental
   confirmation (experimentation note, "Quasi-experimental designs from the
   lineage timeline") closes the loop. Scenario holdouts exist to catch
   gaming *before* adoption; ambient catches it after.
5. Small-workspace degeneracy: below the floor (a user with 2 pinned
   scenarios), the system says so honestly — campaigns run with an
   `unheld-out` tag on their evidence cards rather than fabricating rigor.

Resolved within the policy (2026-07-04): the 20% / floor-2 / k=3 defaults
stand as conventions, with the principled tuning path recorded — the
campaign record observes its own sealed-vs-open score gaps, so a workspace
whose gaps run large can learn stricter sealing from its own history.
**Anchor elicitation against a sealed scenario is allowed and immediately
retires it** from holdout duty (counted as a wear-out event): the label is
worth more than the seal, paid for honestly. **Sealing is per-campaign**,
with wear-out counters tracking *cumulative* gate exposure across
campaigns, so a repeatedly-consulted scenario retires regardless of which
campaign did the consulting.

**Progressive rigor, never entry rigor (standing principle, Jack
2026-07-04): this policy never gates starting.** A campaign with zero
pinned scenarios runs — on ambient and shadow evidence alone, tagged
`unheld-out`; sealing engages automatically as the scenario corpus grows,
and the tags retire themselves as rigor accrues. The same principle
already governs the parent surface (gauges need no bars, `settle` and
`--certify` are optional grades, "rigor is one verb away, never
required") — restated here because a holdout policy is exactly the kind of
machinery that drifts into being a prerequisite if nobody says otherwise.

## 9. Canary — the measurement/action boundary

**This section is the OWNER of the consent surface** (assigned 2026-07-03;
one surface governs *all* live-touching branches — improve canaries, the
quasi layer's deliberate reversals, any versioned-workspace branch
deliberately allowed to reach the world). **Settled in principle
2026-07-03**, on top of the scope-semantics postures (versioned-workspace
note: posture = grant vector per category × role; consent edits the
subject's row only; the instrument's row is never user-editable).

**Consent is exactly and only the re-grant of subject authority over
irrevocables.** The synthetic respondent and real-ask opt-ins are
experiment configuration, not consent — nothing irrevocable fires.

**The exposure ladder — with a consent-free rung this design discovered:**
pinned-scenario regeneration → **shadow** → canary → adopt. Shadow =
counterfactual posture + live ingress tee: the candidate processes a copy
of real traffic, its outputs divert, no irrevocable fires, **no consent
needed** — and by payloads-matter-delivery-doesn't, a diverted reply on a
real input is a nearly lossless observation. Honest limit: **shadow covers
single-turn behavior only** (the counterparty never saw the shadow reply,
so there is no second turn); interactional outcomes are exactly what only
a canary measures. Consequence: canaries get smaller and shorter, spent
only on what shadow structurally cannot see. The shadow *sampling
fraction* is EVSI's choice (sample allocation, budget-governed — shadowing
doubles inference spend on shadowed traffic); the canary *exposure
fraction* is the human's. That is the general principle: **the system
chooses samples; the human chooses risk.** Never-choose-N survives — a
traffic fraction is a risk tolerance, not a sample size; the grant grammar
speaks risk language exclusively.

**A canary is an RCT — the only true randomization on live traffic in the
whole architecture.** Hash-based sticky assignment (unit = counterparty /
conversation — required anyway: you cannot switch programs mid-
conversation, and it keeps within-counterparty interference out of the
estimate); `branch-ref` is the assignment record. Canary evidence carries
**`design: randomized`** — the top of the ambient ladder — and doubles as
the prime feed for the quasi layer's continuous LaLonde-style calibration
audit.

**The grant object** (semantics settled; surface syntax open):

- **Grantee**: a specific branch, or a candidate-class predicate in
  evidence-card vocabulary ("all bars met, in-radius gauges verified
  within noise, no `unheld-out` tag").
- **Provenance class**: *candidate* (never adopted — full grammar) vs.
  *previously-adopted* (deliberate reversal — the program was trusted; the
  risk is known regression, so cheap consent, small default bounds). One
  grammar, risk class as a field; human-authored edits on working branches
  never touch this surface at all.
- **Exposure bounds**, risk language only: traffic fraction, segment
  carve-outs (irreversible intents, named counterparty classes,
  internal-users-first), sticky unit, hard expiry (grants always expire;
  renewal explicit).
- **Rollback triggers**: e-process drift alarms, bar-violation posteriors,
  hard error rates — anytime-valid, so continuous monitoring has no
  peeking problem; rollback is a routing flip, instant. Asymmetry:
  **de-escalation is always autonomous; escalation always consented.**
- **Escalation ladder** (optional): pre-authorized steps or per-step asks.
- **Authorization form**: per-act through the ask door, or a **standing
  policy** (predicate + bounds + triggers, durable). Authority ebb maps
  exactly: approve each act, or write one policy.

Bookkeeping falls out of scope semantics: **grants are speech acts by the
human** — workspace-plane events, monotone, audited, revocable; every
canary egress carries its grant id; the guarantee report lists active
grants. Agents may *request* through the ask door, never self-grant.

**Open within the settled shape:** surface syntax for grants and standing
policies; the segment carve-out vocabulary (likely rides existing
facts/case features); shadow multi-turn residuals (shadow turn one of
every conversation; synthetic-respondent continuations, clearly tagged —
worth-it unproven).

## 10. Campaign records (settled) and improve-as-whip (leaning yes, look pending)

**Campaign record:** an append-only, event-sourced log in the workspace
store — the workspace's durable-artifact family (program lineage, run event
logs, scenarios, evidence ledger, certificates) gains one member. Contents:
candidates considered (already durable as lineage branches), the evidence
card *as it stood* at each decision, elicited preferences (training data for
the local utility model AND a governance record), adoption/rejection
decisions, spend accounting. Product payoff — **program archaeology**: every
adopted edit links to the campaign that produced it, the alternatives it
beat, and the human choice that resolved the tradeoff; "why is this prompt
phrased this way?" is answerable years later (`whip why --history`, the
natural extension of `why` from evidence provenance to program provenance).
Campaign events export via `std.telemetry`; the record is core, dashboards
stay behind the enterprise seam.

**Improve-as-whip:** the split to start from — **campaign orchestration is a
whip** (durable, resumable, budget-capped, event-sourced, escalates via the
ask door; the loop is shaped exactly like a workflow, and dogfooding it
pressure-tests the language), while **engine primitives** (dominance checks,
racing statistics, EVSI arbitration) are runtime built-ins the whip calls —
the standard std-construct-over-runtime split. Careful-look items before
committing: authority (a workflow that proposes program edits needs
owned-harness tool access with a real IFC story), self-reference (the
improve whip is not improvable by itself in v1 — a simple guard), version
coupling (engine upgrades vs. language stability). None look disqualifying.

**Default terminal state: propose, don't apply.** An evidence card per
undominated candidate ("A: +12% extraction, 3 gauges certified untouched, 2
verified within noise, +$0.0004/run; B: +18% but −2% tone — your call");
auto-apply is an explicit opt-in policy. Authority ebb as everywhere — and
the standing-contradiction machinery reopens the choice later if the
evidence turns.

## 11. Dependencies, sequencing, and the deliberately-undesigned issue

Downstream of the experimentation core: ledger, slicer (incl. the
dependency-closure prerequisite — experimentation note, "Soundness
hazards"), gauges, scenarios, `settle`, the measurement model's anchor
slot. Nothing here creates new language surface
beyond the `judge via` extension (§8) and whatever the campaign surface pass
decides (§3). Coherence note: campaigns over workflows whose regenerate
reach includes `unversioned_external` state (versioned-workspace note,
"Coherence and the package state surface") inherit the
`external-state-drift` tag on all their evidence — EVSI should price that
in when arbitrating regeneration against ambient designs.

**Dependency: regeneration side-effect containment — settled in principle
2026-07-03, deliberately designed elsewhere.** A regenerated suffix
re-executes effect sites; `coerce`/agent turns are harmless, but a replayed
`send`/`notify`/`exec` would fire real external effects from a
counterfactual run. The versioned workspace owns the answer
(`versioned-workspace-research-note.md`, "The boundary identity" and
"Per-door containment policy"): containment splits exactly at the IFC
egress boundary — storage-plane effects are contained *completely* by
branch semantics (a counterfactual run executes on a branch; its writes are
branch-local, discarded or adopted), while egress doors get
replay/divert/live verdicts under the counterfactual non-egress invariant.
What remains open there: five sub-forks and the store→world pump audit;
what remains open *here*: the §9 consent surface as the deliberate
exception. This note only records the dependency.

## 12. Settled vs. open

**Settled in principle (Jack, 2026-07-03):**
- Reject the GEPA port; keep reflective mutation as the proposer.
- The frame: dominance invariant + candidate-local probing + lazy preference
  elicitation; frontier never an object.
- Campaign = partition of the gauge vector (ascend / bound /
  free-within-band); bars are hard constraints, restored first.
- Cost/latency/tokens as built-in first-class gauges.
- Indifference bands: noise-floor default, user-overridable.
- Blast-radius factorization; disjoint-slice composition as licensed
  crossover; small-radius preference.
- Lineage-as-population; search-born flag; anytime-valid racing; no sample
  counts anywhere.
- Ground truth = anchors generalized; `judge via coerce|prompt|exec|labels`;
  thin dataset interface (files the user owns).
- Campaign record as an event-sourced durable artifact; program archaeology.
- Propose-don't-apply default.
- **Holdout policy (2026-07-04)**: 20% / floor-2 sealed per campaign;
  aggregates-only proposer visibility; k=3 wear-out with ambient refresh;
  ambient as true holdout; `unheld-out` tag below floor; anchor-ask
  retires the seal; per-campaign sealing with cumulative wear-out;
  defaults tunable from campaign-record sealed-vs-open gaps.
- **Campaign surface (2026-07-04)**: naming-is-the-partition;
  `std.spend`/`std.latency`/`std.tokens` built-ins; inline targets
  (`reach`); `then` stages with ratchet; the `campaign` declaration
  (same grammar, higher ceremony); bare `whip improve` = repair mode;
  `--sacrifice`/`--within`/`--spend-cap`.
- **Complex objectives = derived gauges (2026-07-04)**: `judge via exec`
  over an `inputs` gauge vector; delta-kernel, Goodhart-resistant;
  subsumes weights — no weights feature ever.

**Settled in the design passes with Jack (2026-07-11):**
- **Campaign scope** (§3): segments are gauge-layer partition refinements,
  never campaign-layer filters (complement auto-guarded via
  unnamed-means-guarded; campaign-layer filtering rejected as a dominance
  hole); case-feature predicates over facts as the naming destination,
  pin-time segment tags as the progressive-rigor rung, classifier-judge
  segments an idiom; the grant grammar shares the predicate form only;
  campaign *target* (workstream line vs. mainline) decoupled — mechanics
  riding the candidates-as-branches residual.
- **Proposer leakage policy tiers** (§7): tiers keyed off the reader-set
  delta (no delta → unrestricted unflagged; delta → unrestricted-with-flag
  default); stratified reflection is CAMPAIGN-ATTACHED (`proposer
  redacted` / `--redacted-view`, tighten-only) — built same day; cleared
  providers reclassified as reader-side machinery outside this policy; no
  v1 leakage budget, trigger observable via the campaign record.
- **The local utility model**: representation = a PRECEDENT SET, not a
  parameter blob — answered tradeoffs stored verbatim (full
  direction-adjusted delta vector, operating point at answer time,
  verdict, answerer) in the campaign-record family, where elicited
  preferences already live as governance records. Auto-resolution =
  **monotone precedent dominance, engaged by default**: a candidate
  Pareto-at-least-as-good (direction-adjusted, per gauge) as an approved
  tradeoff auto-approves; one Pareto-worse than a rejected tradeoff
  auto-rejects; everything between asks. Sole assumption is monotonicity
  in each gauge's better-direction (already baked into the surface).
  Every auto-resolution cites its precedent (program archaeology extends
  to preferences); precedents are speech acts — inspectable, revocable by
  deletion, reopened by the standing-contradiction machinery. Locality:
  a precedent applies only while the current operating point sits within
  the indifference-band neighborhood of its answer-time point; stale
  precedents become inapplicable, never silently deleted. A learned
  direction-vector posterior is an OPTIONAL component that carries no
  authority — permitted solely as the EVSI pricer for ask ordering and
  ask-worthiness. Cold start = the empty precedent set (always-ask is
  the degenerate case, so the v1 build is already conformant). Stronger
  delegation is written, never learned: an explicit standing policy in
  the §9 grant idiom.

**Open:**
- **Canary authorization surface** — shared with the quasi-experimental
  layer's deliberate-reversal question (experimentation note); narrowed
  2026-07-03 to the canary grant grammar (§9).
- **Improve-as-whip careful look** — authority/IFC, self-reference guard,
  version coupling.
- **Regeneration side-effect containment** — settled in principle 2026-07-03
  (versioned-workspace note, "Per-door containment policy"); residual = the
  consent surface (§9, owned here) plus that note's open sub-forks (§11).
- Formal-model scope when the effort builds: the acceptance rule ("never
  accept a dominated candidate, w.h.p.") is the invariant to model —
  coverage + bite per house discipline.
