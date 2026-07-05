# Experimentation & Evaluation Subsystem — Product Research Note

**Status: RESEARCH NOTE (pre-ADR).** Captures a design conversation
(2026-07-01, developed 2026-07-02/03) about how a developer refines and gains
confidence in a stochastic WhippleScript workflow. Fourth pass added the
**product surface** (§4), settled in principle by Jack 2026-07-03 — the seam is
**internal** (a whip-native surface, not an external product), the language
gains two small declarations (`gauge`, `mark`), and the verb set encodes the
ambient/sequential evidence model. Fifth pass (2026-07-03) added the
**identification layer** (§12): the do-*calculus* — not just the do-operator —
run over whip's certified causal graph. Sixth pass (2026-07-03) expanded
**quasi-experimental designs from the lineage timeline** (§13): the
production-time mirror of the dev-time do-machine, with certified control
series. **Not scoped or committed to v1** — build timing is open; it depends
on post-v1 substrate plus one analysis prerequisite (§9.1). An earlier
direction that externalized the system behind a product-to-product seam was
considered and **rejected** (2026-07-03): the surface must feel native
alongside the existing whip surface.

## 1. The problem

Whip already has deterministic testing (`test`/`given`/`stub`/`run`/`expect`),
observability (`std.telemetry`), and model surfaces (`coerce`/`tell`/`decide`).
None answers the real question a developer of an LLM workflow has: **"is this
workflow doing what I want, and did my change make it better?"** — where the
workflow is *stochastic*, so a single run proves nothing and "better" is a
judgment, not a mechanical metric.

Two tempting non-answers, both rejected: **"evals = coerce + a workflow"**
(special support is only justified by a semantic link plain composition cannot
express — there are two, §4.2) and **"evals = observability"**
(`std.telemetry` measures execution mechanics; it cannot *compute* a judgment,
though once a score exists, alerting on it is observability's job). What
neither covers is **grounded, statistically honest, fast iteration on a
stochastic program**: freeze a real scenario, re-run the uncertain part, judge
it, and accumulate evidence as the workflow changes.

## 2. The core reframe — a whip-native subsystem

Almost all of this is runtime + tooling, not syntax: the evidence is living
runtime state; experiments anchor to runs (checkpoints in real event logs);
interventions target anything, so an experiment is transverse; the loop is
interactive (human + root agent). So this is a **subsystem — an evidence
ledger + a slicer + pluggable statistics + the root-agent loop** — built on
two substrates whip is already heading toward: **checkpoints / restorable
context** (`decision-records/restorable-context.md`) and **the root/session
agent** (DR-0026).

The seam is **internal**: the evidence ledger lives in the workspace store
exactly as the coordination store does; the verbs are `whip` subcommands
beside `dev`/`test`/`status`; the reports read like the guarantee report. The
language contributes exactly **two small declarations** — `gauge` and `mark`
(§4.2) — and nothing else; scorers are ordinary effects
(`coerce`/`prompt`/`exec`/label files), durable deterministic invariants are
ordinary `test`. gaugewright is a consumer of
this surface (the dogfood/requirements engine), not its home.

## 3. What "good" looks like

A developer works on a workflow with the root agent, informally. They run it
(the **event log is the run**), change things, rerun. The system quietly
models every change as an intervention and tells them, through a few honest
numbers the agent explains in plain words: *is it better, by how much, how
sure, and what is muddying that judgment* — with a leaderboard when weighing
options and a hint about the most informative next step when stuck. When a
judgment matters they settle it; when they want tuning, an optimization mode
searches for them. **Never forced into a protocol.** Evidence accumulates and
self-corrects — a hasty N=1 call is revealed as more data arrives — and the
machinery stays invisible unless asked. The feel: **fast, creative, and
scientific** — the scientific method without the formality.

## 4. The surface (settled in principle, 2026-07-03)

Designed from the operating seat: the root agent (or a technical user) running
whips on behalf of a user, translating between real-world intuition and the
program, under an **ebbing of authority** — sometimes the user follows
guidance, sometimes they are opinionated. Authority ebb maps to *verb choice*,
never to modes or configuration.

### 4.1 Two principles the surface must encode

1. **Measurement is ambient.** A `gauge` does not wait to be invoked — it
   makes *every* run an observation: dev runs, test runs, production runs.
   (Judging costs money, so the gauge scores adaptively — densely when beliefs
   are loose or an edit just landed, sparsely when they are tight — the same
   value-of-information logic applied to whether scoring is worth it.)
   Consequence: **evidence usually exists before anyone asks.** Explicit
   regeneration supplements the ambient stream when it is too confounded or
   too slow; it is never the primary source. The evidence screens make this
   visible: `N=64 (7 regen · 57 live)` — the corpus is a byproduct of use.
2. **The operator never chooses N.** They ask what we believe, name a decision
   to settle, or cap spend (in currency). The system allocates samples — which
   scenarios, which sites, how many — by expected value of information, and
   stops itself. **No surface anywhere accepts a sample count.** A `-n` flag
   is a design failure: it regresses to the run-until-N data-science ritual
   this design exists to kill.

The two evidence engines cover opposite regimes: the do-machine dominates
**dev-time** (dense edits, thin traffic, pinned scenarios), the §13
quasi-experimental designs dominate **production-time** (sparse edits, clean
windows, high ambient volume). Opposite data economics, one ledger.

### 4.2 The nouns

Users speak in **examples** ("that email from accounting") and **qualities**
("it keeps missing the due date"); the nouns match, so translation is
near-mechanical.

- **`scenario`** — a pinned, named checkpoint: a specific run's frozen prefix
  kept because it exemplifies something. Runtime-side identity (which run).
  The scenario library *is* the regression corpus, grown out of conversation
  rather than authored.
- **`mark`** (language) — a named cut point declared in `.whip`:

  ```whip
  mark "triaged" after classify
  mark "drafted" after summarize
  ```

  The runtime cuts a checkpoint at every mark on every run, automatically.
  Marks name *where* the meaningful moments are (program structure, versioned,
  identity riding the slice hash); scenarios name *which* run. Pinning
  composes them: `whip pin run-0142 at triaged --as subject-line-duedate`.
  Three properties: names are stable across edits (event offsets shift, marks
  don't); capture is retroactively ambient (any past run can become a scenario
  at any declared mark — the cuts were already taken); marks give gauges an
  optional segment-scope vocabulary later, uncommitted now. Deliberately a
  **separate declaration from `milestone`** (child→parent lifecycle signaling
  vs. event-log position — genuinely different mechanisms; keep them
  separate). Naming: `mark` over `cut` (reads better in source; `cut` stays
  the DR term for the consistency property).
- **`gauge`** (language) — a named quality dimension: a site, a judge,
  optionally a bar. The sibling of `test` — deterministic expectation vs.
  stochastic expectation, one family:

  ```whip
  gauge extract_quality on summarize.extract {
    judge via coerce DueDateJudge          # an ordinary coerce declared elsewhere
    expect P(due_date_correct) >= 0.9      # optional bar; the default decision
                                           # bar for settle, checked by release
                                           # gating in hardening grade
  }
  ```

  This revises the earlier "language surface: ~none" stance, with the
  justification the reframe demands: the *binding* of judge to site must ride
  the site's identity (slice hash) and version with the program — a scorer
  floating in CLI config or an external database is the un-whip thing. A
  `gauge` versions, formats, lints, and diffs like everything else. Bars may
  be quantile-shaped (`expect p10 >= 0.7`) — reliability is a tail property
  (§14). The judge slot is generalized (settled 2026-07-03, in the improve
  effort): **`judge via coerce | prompt | exec | labels <source>`** —
  deterministic validators and user-owned label files fill the same slot as
  LLM judges, with no new import machinery; rationale and the
  Goodhart-resistance consequences live in `improve-design-note.md`
  ("Ground truth, Goodhart, and the holdout policy"). **Derived gauges**
  (settled 2026-07-04): a gauge may take an `inputs` clause naming other
  gauges — its exec judge receives the score vector, enabling
  composite/business objectives without whip growing math capabilities;
  deterministic → delta-kernel identity, Goodhart-resistant; the `inputs`
  clause hands the slicer its edges. Subsumes any weighted-objective
  feature.
- **`evidence`** — the accumulated asset (workspace store). A noun, not a
  verb, deliberately: consulting it performs nothing (peek-anytime, §11).
- **`certificate`** — an anytime-valid guarantee produced by settling in
  certify grade; attaches to a gauge **and rides slice hashes like any
  evidence** — certify a gauge, then make an edit that provably cannot touch
  that site, and the certificate survives *with proof*.

### 4.3 The verbs

- **(ambient)** — gauges score every run adaptively; the primary evidence
  source has no verb at all.
- **`whip pin <run> [at <mark>] --as <name>`** — name a scenario at the moment
  a case matters. One command, no ceremony.
- **`whip suppose <scenario>`** — one what-if regeneration from the frozen
  scenario under the current program; the recorded outcomes are the paired
  control; the belief updates as a side effect (every observation lands
  immediately). Named `suppose` (not `try`: try-catch collision, and
  `suppose` teaches the interventional semantics — "suppose we'd had this
  prompt when Tuesday's email came in" is literally what the do-operator
  computes). Also the everyday debugging what-if tool, independent of any
  statistics. With the §12 counterfactual bounds, `suppose` can answer the
  question the user actually asked — "that failure would have been avoided
  with probability ≥ 0.82" — rather than only the interventional dodge
  ("under the new prompt: 6/6 correct").
- **`whip settle <gauge> [--compare <dim>] [--certify] [--anchor]
  [--spend-cap <$>]`** — name the decision; the system **attempts
  identification first** (§12: can the ambient stream answer this via
  adjustment or a §13 design over the certified graph?), then allocates
  regenerations only for the residual that identification provably cannot
  reach, by information value, and **stops itself**: at the gauge's bar, when
  EVSI collapses below the cost of one more regeneration, or because **the
  question has no answer of the requested shape** (e.g. low rankability: "no
  total order — friendly wins complaints, terse wins status-checks"), which is
  run-until-evidence in its strongest form. When a question is unidentifiable
  and regeneration is expensive, settle may instead suggest a one-line
  **instrumentation edit** (§12) that makes future ambient data answer it for
  free. `--certify` upgrades the stopping rule to an anytime-valid sequential
  test and records a certificate (there is no separate `harden` verb —
  certification is settling to a stricter grade). `--spend-cap` is a guardrail
  in currency, never a sample size.
- **`whip evidence [<gauge>] [--compare <dim>]`** — peek anytime (always
  valid): estimate + honesty tags (§14) + the N decomposition (regen vs live)
  + **standing-contradiction flags** — the ledger reopening earlier calls on
  its own as ambient data accumulates ("⚠ contradicts 07-03 user-directed
  call: P(worse)=81% and tightening"). Named `evidence`, not `eval`
  (rejected: drags in dataset/benchmark-batch baggage the design defines
  itself against; is an action word for a deliberate non-action; interpreter
  connotations). Everything speaks `--json`; the human-readable lines are
  deliberately quotable so the agent's translation is often verbatim.
- **`whip why <gauge>`** — provenance as a first-class answer: what pooled,
  what carried (through which edits, by hash), what was adjusted for and by
  what licensing (§12), what design produced an estimate and how it was
  audited (§13), what was excluded and why (e.g. an unanchored judge
  generation). Nothing is "trust me."
- **`whip evidence carry --from <hash> --to <hash> --because "…"`** — assert
  transfer the slicer cannot certify (a semantics-preserving refactor):
  audited, honesty-tagged, revocable by drift monitors (declassify's twin,
  §6).
- **`whip improve <gauge>`** — **reserved, undesigned** (§15).

Ambient integration: `whip check`/`whip dev` grow one evidence line — the
edit's blast radius plus what carried (`edit touched summarize.extract · 2
gauges unaffected (carried) · extract_quality warm-started`) — the transfer
layer surfacing at exactly the moment it acts, with zero ceremony. The same
seam carries the §13 **attribution notice** at ship time: `this ships 2h
after edit e46 — co-located changes degrade effect attribution; hold for a
clean window?` — the timeline as something the system helps keep legible, not
just something it reads.

### 4.4 The feel — three vignettes (the spec for how it must behave)

**Delegated authority.** User: *"Triage keeps missing due-dates in subject
lines."* The first move is not to reproduce — the evidence already exists:

```
$ whip evidence extract_quality
P(due_date correct) = 0.58 · N=41 (all live) · worst slice: subject-line 0.31 (n=13)
```

The complaint is confirmed and quantified before anything is touched. Pin the
user's example, edit the prompt, `whip check` shows the carry, then:

```
$ whip suppose subject-line-duedate
due_date = 2026-07-11 ✓ (was: missing)
extract_quality: P(better)=71% · N=1 + warm prior
  undetermined for your bar (≥0.9) · est. to settle ~$0.19 → whip settle extract_quality
$ whip settle extract_quality
  stopped at N=7 (bar cleared): P(better)=93% (+31%) · tags: none (paired)
```

The system chose seven. Agent to user: *"Fixed on your example and it holds
across the other pinned cases — no caveats."* (`tags: none` is what licenses
that sentence.)

**Opinionated user.** *"Prove yours is better"* → `whip settle extract_quality
--certify --spend-cap $2` → "crossed at N=11: better at 95%, certificate
ct-3f21 · spent $0.34". Or the user overrules the numbers ("use the formal
wording — compliance wants it"): no command at all; the ambient stream keeps
flowing, and weeks later `whip evidence` raises the standing-contradiction
flag by itself — the system giving the agent permission to *respectfully
reopen* a decision, with evidence rather than opinion. And *"why should I
believe any of this"* has `whip why`: 36 obs, 24 this version, 12 carried
through edits e41/e44 by hash; judge anchored (9 human labels, agreement
0.91); 8 obs excluded (unanchored judge generation).

**The fork in the road.** User: *"Try three tones: formal, friendly, terse."*
Variants are kernels; evidence accrues whenever they are exercised:

```
$ whip settle reply_quality --compare tone
  stopped: no total order (rankability low, curl 0.41)
  friendly wins complaints · terse wins status-checks
```

The honest statistic changes the *program design* — "want me to branch on
message kind instead of picking one?" — which is the point of all of it.

### 4.5 Why this is the balance

The verbs are the operator's actual questions: what did my edit touch
(`check`)? does it work on the case that failed (`suppose`)? what do we
believe (`evidence`)? are we sure enough (`settle`)? why should anyone trust
it (`why`)? Authority ebb maps to verb choice: delegated → suppose quietly and
translate; challenged → settle/why; overruled → record, and evidence
self-corrects out loud later. Nothing is ceremony (`pin` is one command;
interventions are inferred from edit + run; the only authored artifacts are a
five-line `gauge` and a one-line `mark`). And every piece earns its place
without the statistics: `suppose` is the debugging what-if, scenarios are the
regression corpus, `gauge` is documented quality intent, `why` is audit.

## 5. Foundation — the workflow as a factorized probability kernel

A workflow at version *v* is a Markov kernel from inputs to trace
distributions, **factorized at its stochastic choice points**: each
`tell`/`coerce`/`decide`/agent-turn effect site is a **sub-kernel** — a
conditional distribution P(output | arriving context) — and everything between
choice points is deterministic given the record. The factorization is not
imposed; it is what the event log already witnesses.

- **Checkpoint-and-regenerate is a do-operator.** Because the runtime owns the
  event log, "freeze a prefix, replay recorded outcomes, resample one
  sub-kernel under a modified generator" is a *literal intervention* (kernel
  surgery), executed by machinery rather than estimated by statistics —
  changing the problem class from causal inference to designed
  experimentation. Precision: interventional, not counterfactual (LLM noise
  cannot be held fixed) — but prefix-blocking recovers most of the variance
  reduction: **paired regeneration on a frozen prefix is a matched-pairs
  design for free** (routinely 10–100× fewer samples). This is what `suppose`
  runs. (Bounded rung-3 statements are nonetheless reachable — §12.)
- **An experiment is a partial evaluation.** Frozen prefix = early-bound;
  regenerate set = late-bound; the experiment is the residual program of
  specializing the workflow at the checkpoint. Capture stays zero-ceremony —
  edit-a-prompt-and-rerun is recognized as an intervention.

## 6. The duality, unpacked

The organizing insight: **evidence transfer is non-interference one meta-level
up.** Non-interference quantifies over inputs (runs differing only in the
secret look the same to the observer); evidence transfer quantifies over
program versions (programs differing only in edit Δ have the same distribution
at site s). Treat the program source as an input to an interpreter and the
second statement *is* the first: the edit is the secret, the choice point is
the observer.

This lands in known territory: the *Core Calculus of Dependency* (Abadi,
Banerjee, Heintze, Riecke 1999) — information-flow security, program slicing,
binding-time analysis, and incremental computation are **one dependency
calculus** under different lattices. Evidence transfer is a fifth face, and
each classical face maps to a component:

- **Slicing ↔ transfer** — the backward slice decides which evidence survives
  which edit (§7).
- **Binding-time ↔ experiments** — checkpoint=static, regenerate=dynamic
  (§5); the replay frontier (first Δ-affected site) prices new evidence:
  regeneration cost ∝ the edit's blast radius.
- **Incremental computation ↔ the whole subsystem** — the ledger is a memo
  table with **distributions as the cached artifacts**: transfer = cache hit;
  an edited kernel = cache miss with a warm-start prior; drift revocation =
  cache expiry. A **build system whose artifacts are posteriors** — whip's
  fourth content-addressed mechanism, after effect idempotency keys,
  restorable-context manifests, and the DO storage plane. *Posteriors are
  build artifacts; edits invalidate exactly what they touch.*
- **Security ↔ the sign flip** — IFC's cost is where flows exist; transfer's
  benefit is where flows don't. Every analyzer precision investment
  (per-field X2, signal edges, anti-dependences) **pays twice**.
- **Integrity is the specific dual — and an implementation.** Evidence
  validity is an integrity property; taint = Δ. **Mark every diffed IR element
  as a low-integrity source and run the existing integrity propagation**
  (the `derived_signal_integrity` style of fold); evidence at s transfers iff
  s's arriving context is untainted. No new analyzer — a new client with a
  synthetic taint assignment.

Three consequences: **assert-transfer is declassification's twin** (`whip
evidence carry`: explicit, audited, honesty-tagged, revocable per NMIF
robustness — careless edits cannot launder stale evidence). **The
probabilistic theorem is cheap via coupling** — all stochasticity sits at
oracle sites and the glue is deterministic given the record, so couple the two
versions' runs on identical oracle draws: Δ outside the slice ⇒ coupled runs
identical at s ⇒ distributions equal; the Lean work is the existing relational
style plus a coupling wrapper (independent confirmation the sub-kernel is the
right unit). **The duality is predictive** — IFC's hard cases
(declassification, timing) predicted transfer's (asserted transfer, §9.2
timing), found independently before the mapping was drawn; DR-0029's
composition results transport to cross-package evidence transfer for imported
tools. Research horizon: QIF dualizes to *graded* transfer — data-processing
bounds on distributional shift, contracting through intervening kernels
(Dobrushin coefficients). Recorded, not proposed.

## 7. Evidence identity — the slice hash

One identity rule. Per choice point s:

- **generator hash** — content hash of s's local generator closure: prompt
  template, output schema, resolved model + provider profile, granted tools,
  temperature — everything parameterizing the conditional (type-level; the
  effect-idempotency-key analogue without instance inputs). For exec sites,
  the closure includes the **script content and the environment/image hash**
  (versioned-workspace note, materialization + evidence-grade boundary
  sections). Execution mode (clock semantics + door
  posture) is ambient config too, entering the hash only for slices that
  depend on it (§9.6);
- **slice hash** — Merkle root over the generator closure **plus every
  program element in s's closed backward slice** (flow *and* anti-dependences,
  §9) **plus ambient config**. The existing reach primitive
  (`reach_reads_from`, ifc.rs — whose complement the X2 code already calls "a
  proven non-interference, `noReach`") computes the slice; §6's taint check is
  the same verdict on demand, the hash is it precomputed.

Transfer is a **lookup**:

| comparison | meaning | evidence status |
|---|---|---|
| slice hash equal | kernel *and* arriving-context distribution provably unchanged | pools verbatim (hash equality *is* the certificate) |
| generator equal, slice differs | same conditional, shifted marginal | valid **per scenario/checkpoint** it was regenerated from (observations key by checkpoint-ref anyway); the upgrade path is a **transportability problem** (§12) — same mechanism, different population; the do-machine *measures* the shift by regenerating the upstream slice under both versions — never importance weighting, whose weights degenerate exactly when the shift matters. External-state reads get the identical treatment via per-effect **fingerprints** — the world's `revision_epoch` (versioned-workspace note, "Coherence and the package state surface") |
| generator differs | edited kernel | no sample transfer; **lineage prior** — θ_new around θ_parent with variance τ²(edit), τ learned per edit-type from lineage history (cold start: a hand-set taxonomy; trained at scale by the §13 retrospective estimates) |

Free properties: **revert reunification** (revert an edit → hashes match the
ancestor → evidence rejoins; evidence follows content, not history);
**cross-variant sharing** (branches differing in one prompt share every other
kernel's evidence with no branch modeling); **the revision DAG demoted**
(`revision_epoch` lineage only supplies edit-type priors); **fail-closed by
construction** (a coarser slicer hashes more, so imprecision only *denies*
pooling — the degenerate whole-program slicer is trivially sound and yields
per-version evidence islands; precision is the single quality knob, rule-level
to per-field X2 v2); **auto-expiry, principled** (evidence pools where nothing
moved, narrows to scenario scope where only the population moved, warm-starts
where the kernel changed); **scorers are kernels** (a judge has a generator
hash; a judge-model bump splits the scale instead of silently polluting it —
§10 for re-bridging).

## 8. The ledger — store facts, derive everything

Append-only raw observations in the workspace store; posteriors are derived
views, never the stored object:

```
(revision_epoch, branch-ref, execution-mode, site-id, checkpoint-ref,
 output-ref, scorer-site, score, cost, timestamp)
```

`branch-ref` and `execution-mode` are provenance columns (added 2026-07-03):
the former serves the versioned workspace's marking condition ("What is
*not* a door" — inspection surfaces must distinguish branch-scoped
content), the latter serves hazard §9.6. Identity remains entirely
content-hash-derived; these columns never key pooling directly.

Generator/slice hashes are **derived lazily** from pinned program versions at
query time (and cached); partition by hash agreement, fold per §7. Three
justifications: **whip's ethos applied to itself** (event-sourced
observations, derived beliefs); **retroactive precision** (when the slicer
sharpens, old observations re-key and become more transferable); **model
upgrades are free** (any future aggregator re-consumes raw samples; scores are
cheap to store — the regenerations that produced them were the expensive
part).

## 9. Soundness hazards — each a way to mint a false hash match

1. **The dependency graph is fact-edges only — PREREQUISITE.**
   `rule_dependencies` captures producer→consumer *fact* edges; rules also
   influence each other through signals, coordination writes (counters,
   leases, queues, ledgers), and channels. The IFC checker sees those as read
   *resources*; the slice needs them as *edges* (the H8
   `derived_signal_integrity` emit-port machinery already computes the signal
   case). **Fourth edge kind (added 2026-07-03): file-handle
   producer→consumer edges** — workflow behavior flowing through
   scripts/file I/O must be in the closure, and content-hash handles make
   these the most precise edge kind (the handle literally names the artifact
   flowing between sites); see `versioned-workspace-research-note.md`, "The
   evidence-grade boundary", for the exec taxonomy (deterministic delta
   kernels vs. stochastic sidecar turns, empirical hermeticity checking,
   fail-closed demotion). Until the closure includes all four kinds, every
   slice hash is unsound and verdicts must degrade toward whole-program.
   **First work item whenever this effort starts; a prerequisite, not a
   choice.**
2. **Anti-dependence.** A `consume` can *prevent* a fact from reaching s — an
   edited rule merely competing for a resource in the slice's read set changes
   what arrives with no producer edge. The slice needs flow, anti, and output
   dependences; fail-closed v1: if Δ's write-or-consume footprint intersects
   the slice's read footprint, no pooling. Timing likewise: edits touching
   timeouts/timer structure that could race with the slice are global until
   proven local.
3. **Ambient config is part of the generator.** Model bumps, provider-profile
   changes, temperature defaults appear in no IR diff yet change every kernel;
   the generator hash folds in the resolved provider profile. A provider bump
   then visibly warm-starts every touched kernel — with its own learned τ,
   which over time answers "how much does a model upgrade actually move my
   workflows."
4. **The world drifts under an unchanged program.** Web results change under
   byte-identical kernels; providers shift within a version. No hash sees
   this. Backstop: a per-kernel **drift monitor** — an anytime-valid
   changepoint detector (e-process, §11) over the ambient score stream — that
   *empirically revokes* pooling the hash granted. **Hashes cover program
   change; monitors cover world change.** A revocation surfaces as an honesty
   tag.
5. **Scorer drift** is the same story on the score axis: scorer hashes split
   the scale; human anchors re-bridge it (§10).
6. **Execution mode is an intervention the hash must see (added 2026-07-03,
   second review pass).** A counterfactual run executes under a different
   *mode* than a live one: virtual clock, diverted egress doors, possibly
   truncated needs-human paths (versioned-workspace note, "Per-door
   containment policy"). Most of the mode difference is provably harmless —
   diversion changes delivery, not payloads (the measurand); truncation and
   the synthetic respondent already carry their own tags and source
   identities. The un-handled residual is the **virtual clock**: a
   timer-sensitive suffix sampled under compressed time is a different
   distribution at the same slice hash. Fix, in-house: execution mode is
   **ambient config**, and like any config it enters the hash of exactly the
   slices that depend on it — the slicer already knows whether a slice reads
   the clock or timer structure. Clock-independent slices (most LLM kernels)
   hash identically under either mode, so regen and live evidence pool
   freely there — the core loop is untouched; clock-dependent slices
   partition by mode (or surface an honesty tag), which is the fail-closed
   verdict exactly where the hole exists. The §4.1 regen/live N
   decomposition is this distinction surfacing; matched pairs are unaffected
   (both arms of a `suppose` share the mode). The subject/instrument split
   (versioned-workspace note, "Per-door containment policy") sharpens the
   hazard's boundary: instrument-side differences — e.g. denser judging on
   counterfactual runs — never perturb the subject's distribution, and the
   §10 observer-effect certificate is what makes that a theorem rather than
   an assumption.

**The central bet (named once, here).** Everything above is fail-closed —
but fail-closed protects against *imprecision within known influence
channels*, not against a **missing channel class**. The completeness of the
dependency-closure enumeration (facts + signals/coordination/channels +
consume anti-dependences + file handles) is the single soundness bet that
transfer, certified merge, improve's licensed crossover, and the
identification layer all ride simultaneously: a fifth un-modeled way rules
influence each other would mint false certificates in four systems at once.
Two mitigations: the formal models carry a bite fixture *per channel kind*
(§16), and — applying the corpus's own pattern (drift monitors backstop
hashes; the LaLonde audit backstops the quasi layer) — the engine
**spot-audits Tier-A transfers**: occasionally regenerate at a site whose
evidence pooled "verbatim by certificate" and check fresh samples against
the pooled distribution. A certificate that fails its spot-audit is evidence
of a missing edge kind — the closure gets an empirical tripwire rather than
resting purely on enumeration.

## 10. Measurement — scores are instrument readings

Judge *noise* averages out with N; judge *bias* (verbosity preference,
position bias, self-model sycophancy) does not. The v1 stance is structural,
not statistical: **never pool across scorer hashes** (crude, fail-closed, zero
machinery). The upgrade path is psychometrics: scorers as instruments with
learned bias/discrimination/drift (Dawid-Skene / IRT-style latent-variable
models, jointly inferred), **sparse human labels as anchors** that identify
the scale and — via test equating — bridge scorer generations, so evidence
scored by a dead judge becomes usable again instead of expiring.

**Anchors arrive through the existing human-ask machinery** (pending-asks, the
I-IFC8 door, the escalation channel): when judge unreliability becomes the
bottleneck on a decision, the system routes a tiny "which of these two is
better?" comparison through the ask surface — **EVSI prices the
interruption**, so it only asks when a human label is worth more than the
annoyance. The scarcest resource in the loop (user judgment) is allocated by
the same value-of-information logic as everything else, through a door whose
IFC semantics already exist. Surfaces as a tag with a route-to-fix: `judge
unanchored — 2 quick comparisons would fix → whip settle --anchor`.

**Measurement must not disturb the system — and here that is checkable
(added 2026-07-03).** The instrument — gauge-declared judges, the ledger
writer, the anchor surface (versioned-workspace note, "Per-door containment
policy") — reads traces after the fact and writes nothing into the
subject's arriving context. Instrument–subject non-interference is a reach
property the slicer certifies like any other: no edge from instrument
outputs into subject inputs within a run. The failure mode is real: a
workflow that wires gauge scores back into its own logic builds a feedback
loop in which every intervention on the judge is an intervention on the
subject — the certificate fails and the evidence carries a
**measurement-feedback** tag (§14). Whether strict mode refuses outright is
open (§20).

## 11. Statistics are plugins, not architecture

The system's irreplaceable job is knowing **which observations are still about
the thing being asked** (§7–§9, §12) — a bibliography, not a calculator.
Aggregation over a valid partition is commodity and pluggable; sophistication
is a monotone upgrade path over the same raw ledger:

- **v1 floor:** empirical distributions / Beta-Bernoulli per
  (slice-hash, scorer-hash) partition; raw pairwise win matrices; suggestion =
  cheapest experiment touching the highest-uncertainty kernel.
- **Distributional comparison.** Reliability is a tail property (a +5%-mean
  variant that fails badly 2% of the time is worse — SPC's original concern):
  quantile treatment effects, stochastic dominance, CVaR; "mean up, tail
  worse" becomes representable and flagged rather than averaged away. Gauge
  bars may be quantile-shaped.
- **E-processes for guarantees.** "Bayesian, so peeking is no sin" holds for
  beliefs, not guarantees — under misspecification (an LLM-judge likelihood
  always is) a peeking Bayesian still fools itself. Game-theoretic probability
  (Shafer–Vovk; Ramdas et al.) gives **anytime-valid certificates** — valid at
  every stopping time. `settle --certify` emits them; e-values multiply across
  independent experiments (certificates compose); the same object is the §9.4
  drift monitor watching the ambient stream. Conformal prediction sits beside
  for distribution-free deployment guarantees.
- **Hodge decomposition for variants.** LLM-judge pairwise preferences are
  measurably intransitive; Bradley-Terry launders cycles into a confident
  linear ranking. HodgeRank: the **gradient** part is the leaderboard; the
  **curl** norm answers what ELO cannot ask — *are these variants rankable at
  all?* Surfaces as a stopping reason in `settle --compare` (arguably its best
  possible UX). ELO-style ratings survive as display only.
- **Quasi-experimental aggregator (§13).** A single Bayesian
  structural-time-series plugin (CausalImpact-style: local trend + seasonality
  + certified-control regression → posterior over the counterfactual series)
  covers most of the §13 design menu in one state-space model, emitting
  P(improvement) in the same currency; e-process variants give the
  anytime-valid version for monitoring a deployed change.
- **Hierarchical pooling** across the lineage DAG (the §7 edit-type priors)
  and the §10 measurement model, jointly inferred.
- **EVSI, not EIG, everywhere and invisible** — never a knob, always the
  allocator: sample allocation and stopping inside `settle` (bits that cannot
  change the decision are worth zero; stop when EVSI < the cost of one more
  regeneration, priced via the replay frontier), the `est. to settle` hints,
  ambient judging density, and when to interrupt a human for an anchor.

Sequential updating is why every single observation lands immediately
(`suppose` prints an updated belief at N=1); transfer is why the ambient
stream *compounds* across edits instead of resetting. Those two together are
the design's center: **evidence as a continuously accreting asset the user
never has to tend.**

## 12. The identification layer — the do-calculus over a certified graph

Fifth-pass addition (2026-07-03), answering "are we fully exploiting the
do-calculus framing?" — we were not. §5 uses do() as a *mechanism* (we can
literally intervene). The do-*calculus* — Pearl's rewrite rules,
backdoor/frontdoor adjustment, the identification machinery — exists for the
opposite move: **answering interventional questions from observational data
without intervening**, when the graph licenses it. Our economics demand it:
ambient observations are free and abundant; regenerations cost money. The
calculus converts the free thing into answers that normally require the
expensive one.

**Why whip can run it when almost nobody else can.** Applied causal inference
dies on two assumptions: you must *assume* the causal DAG, and you rarely
observe the mediating mechanism. Whip eliminates both: the rule-dependency
graph + slice structure **is** the causal graph — statically certified,
fail-closed — and the event log records **every intermediate variable** in it.
Same certificate-authority story as transfer, pointed at a second target.

What it yields:

- **Backdoor adjustment, mechanically.** "Did the prompt edit help?" asked
  against ambient data is confounded by whatever else changed — but if the
  confounder (case mix, recorded as facts) satisfies the backdoor criterion
  in the certified graph, the engine conditions on it and the confound is not
  a caveat, it is a *solved problem*. Adjustment sets are computed from the
  graph; no human specifies covariates. This splits the confound tag (§14):
  **`adjusted for {cases}`** (backdoor held — the estimate is causal) vs.
  **`unadjustable: {model-version}`** (open path or no overlap — costs money
  to resolve).
- **Frontdoor is unusually strong here.** Frontdoor rescues identification
  when confounding is unobserved but the *mechanism* is fully observed —
  normally a textbook curiosity because nobody observes full mechanisms. Whip
  observes the mechanism **by construction**: a prompt edit affects the
  outcome only through the facts its site produces, and those are in the
  event log. Even contaminated ambient stretches can identify through
  recorded mediators.
- **`settle` = identify first, regenerate the residual.** EVSI arbitrates
  **four currencies**: the free adjusted estimate; a one-line instrumentation
  edit; paid regeneration; a human anchor. The do-machine becomes the court
  of last resort; the calculus keeps you out of court. User-visible change:
  the `est. to settle` numbers get dramatically cheaper.
- **Transport calculus puts the §7 middle row on formal footing.** "Generator
  equal, slice differs" — same mechanism, shifted context distribution — *is*
  a transportability problem (Pearl–Bareinboim selection diagrams: what
  reweighting is licensed). The same machinery answers a question not posed
  before: whether evidence transports **across environments** — dev to prod,
  one deployment's traffic to another's — where most ambient evidence will
  live in any multi-tenant future.
- **Bounded counterfactuals (rung 3) are partially reachable after all.**
  Point counterfactuals stay out (LLM noise can't be held fixed), but
  Tian–Pearl-style bounds on probability of necessity/sufficiency are
  computable from *combined* observational + interventional data — exactly
  what the ledger holds. Users ask counterfactual questions ("would the new
  prompt have prevented Tuesday's incident?"); `suppose` can answer them with
  an honest interval ("avoided with probability ≥ 0.82") instead of only the
  interventional statement.
- **Identifiability-driven instrumentation.** When identification fails, the
  calculus names the variable that would fix it — and in whip, observing a
  variable is a one-line edit (`record` a fact, add a `mark`). The system's
  response to an unidentifiable question is a route-to-fix in guarantee-report
  style: *"record `case_kind` at triage (one line) and the ambient stream will
  answer this class of question for free from now on."* The calculus's
  failure modes become concrete program-edit suggestions.
- **Quasi-experimental designs from the lineage timeline** — developed fully
  in §13.

**Honest limits.** The certified DAG is *program-internal*: causal sufficiency
at the world level is not certified (time-of-day, upstream data drift can
confound inputs and outcomes invisibly to the program graph). Identification
claims are scoped — "causal relative to the program graph + recorded
environment facts" — residual external confounding stays tagged, and the §9.4
drift monitors remain the backstop (certificates for program structure,
monitors for the world). And **positivity/overlap is not guaranteed**: ambient
data comes from a user changing five things at once; the engine must check
overlap and fail closed to regeneration — identification can only *save*
money, never mint an invalid answer. Interference between runs (SUTVA)
through the shared knowledge plane is handled structurally
(versioned-workspace note, "Scope semantics"): counterfactual subjects
cannot write the plane — contamination is blocked at the source exactly
where sample counts are high — and live runs' plane reads are recorded
ingress events in the dependency closure, so cross-run influence is visible
causal structure to condition on, never an invisible violation.

The unifying statement: **transfer recycles evidence across program change;
identification recycles evidence across assignment bias.** Two orthogonal
recyclers over the same raw ledger, powered by the same certified structure,
both fail-closed, both existing so the expensive things (regeneration, human
attention) are spent only on what cheap data provably cannot answer.

## 13. Quasi-experimental designs from the lineage timeline

Sixth-pass expansion (2026-07-03). The lineage timeline is *better* ground for
the quasi-experimental toolbox (interrupted time series, difference-in-
differences, regression discontinuity, event studies) than the domains those
designs were invented for, because their four standing enemies are all
structurally weakened here:

- **Interventions are stamped, byte-exactly** — every edit at a known instant
  with a content diff; provider bumps stamped by generator-hash changes;
  judge changes by scorer-hash changes. The classical "were there
  co-interventions?" checklist stops being an assumption and becomes a
  *query* (clean-window detection is mechanical).
- **Subjects cannot anticipate.** Inbound traffic doesn't know a deploy is
  coming: anticipation, Hawthorne effects, testing effects, and subject
  maturation simply do not exist for program traffic. What remains is exactly
  the stamped set plus world drift, which already has its backstop (§9.4).
- **Boundary selection is checkable** — RDiT's density/covariate-balance
  check runs over *recorded facts*.
- **Controls are certified** — the piece nobody else can have (below).

### 13.1 The design menu, mapped

- **Within-run difference-in-differences with certified controls — the
  star.** For an edit Δ, every gauge whose slice hash is untouched is
  *provably* unaffected — the non-interference certificate **is** the "no
  treatment leakage to controls" assumption, unverifiable everywhere else, a
  theorem here. Those control gauges are still exposed to everything worth
  differencing out (provider drift, seasonality, input-mix shift) and ride
  **the very same runs** as the treated gauge: ordinary DiD prays for
  parallel trends between different populations; within-run DiD has
  *identical traffic on both arms by construction*. The slicer performs the
  treated/control classification mechanically (downstream-of-Δ gauges are
  treated too). Residual weakness: scale comparability across gauges with
  different judges — the §10 measurement model, or rank-based variants
  (changes-in-changes) that need no shared cardinal scale.
- **Regression discontinuity in time.** Runs just before vs. just after a
  deploy are locally as-if randomized. Mechanical guards: the clean-window
  check (no other stamped change within the bandwidth) and the boundary
  density test on recorded case features. Cheap, sharp, honest about when it
  doesn't apply.
- **Reverts are withdrawal designs.** The single-case experimental design
  literature (n-of-1 trials, ABAB) demands the effect appear–disappear–
  reappear; users revert naturally, and §7 revert reunification already does
  the identity bookkeeping. Better: the system can *suggest* a deliberate
  reversal — "flip back for a day to confirm" is an ABA design on live
  traffic costing **zero regenerations**, slotting into the EVSI currency
  list between free ambient designs and paid regeneration.
- **Staggered adoption across deployments.** A package upgrade adopted at
  different times by different workspaces is a staggered event study — with
  a guarantee staggered designs never get: the treatment is
  **content-hash-identical across every adopter**. Modern
  heterogeneity-robust estimators (Callaway–Sant'Anna style) apply directly;
  transport (§12) governs which adopters may pool. Product capability that
  falls out: **package authors get causal quality evidence across their
  adopter base** ("v2.1 improved downstream gauges across 14 adopters") from
  ambient data alone.
- **Implementation umbrella:** the §11 BSTS plugin — one state-space model
  covers ITS, comparative ITS, and synthetic control; e-process variants for
  anytime-valid monitoring of a deployed change.

### 13.2 Three structural connections

1. **Lineage topology selects the design.** A lone edit in a quiet window →
   RDiT. An edit with certified-untouched siblings → within-run DiD. Edit
   then revert → withdrawal design. Parallel branches over one period →
   concurrent cohorts (the ledger's `branch-ref` column, added for
   provenance marking, is the assignment record that makes this design
   computable). Staggered package adoption → event study. The engine
   pattern-matches the lineage shape against the design library — the
   modeler-first principle extended to design selection; the user never
   chooses a methodology.
2. **This layer is the τ-learning engine.** §7's Tier-C priors need the
   empirical distribution of quality shifts per edit type; the specification
   is: quasi-experimental effect estimates computed *retrospectively across
   the entire lineage* — every edit ever made, each with its ITS/DiD
   estimate — are the training data for the edit-impact hyperprior.
   Warm-start widths get grounded in the workspace's actual history and
   improve for free as it accumulates.
3. **The do-machine continuously audits the quasi layer.** LaLonde's critique
   (observational methods, benchmarked against experiments, are often wrong)
   becomes a *continuous, automatic* benchmark: whenever `settle` regenerates
   (interventional ground truth) for an effect the quasi layer also
   estimated, the discrepancy is a measured bias sample for that design in
   this workspace. Quasi-estimates carry a calibration tag backed by actual
   audits — the do-machine doesn't just answer questions, it keeps the cheap
   layer honest.

### 13.3 Placement, regime, limits

On the evidence ladder this is its own rung — above §12 covariate adjustment
(design-based, not just adjustment-based), below regeneration — and it is
**design-tagged, never silently pooled**: `since edit e47: +11% · design:
within-run DiD · controls: 3 certified-untouched · clean window ✓ ·
calibration: audited ±3%`. The regime split worth headlining: **this layer is
the production-time mirror of the dev-time do-machine.** During rapid
iteration, edits are dense, windows dirty, traffic thin — the do-machine's
regime (scenarios, `suppose`, `settle`). In production, edits are sparse,
windows clean, ambient volume high — this layer's regime, where regeneration
is wasteful. Opposite data economics, complementary coverage, one ledger. The
surface touch it adds is the §4.3 **attribution notice** at ship time
(co-located changes degrade attribution; the system helps keep the timeline
legible). Limits: needs traffic (sparse workspaces get wide honest
intervals); scale comparability across control gauges is real work;
heavy-iteration periods yield few clean windows (fine — that's the other
regime); unstamped world change remains the monitors' job.

Two rungs added 2026-07-03 (consent design — improve note, "Canary"): a
**canary is an RCT** — hash-based sticky assignment on live traffic, the
only true randomization in the architecture; its evidence carries
**`design: randomized`**, sits above every quasi design, and is the prime
feed for connection 3's calibration audit. Below it, **shadow mode**
(counterfactual posture + a live ingress tee) yields
real-input-distribution evidence with diverted delivery, consent-free and
honest about its single-turn limit.

## 14. Honesty tags

The evidence unit is **an estimate + its honesty tags**, enumerable because
the system sees everything (conditions-diff minus the intervention of
interest): **adjusted-for** (a confound closed by graph-licensed adjustment,
§12 — informative, not a caveat); **confounds/unadjustable** ("the cases also
differed and the model version bumped" — an open backdoor path or no overlap;
costs regeneration to resolve); **design** (a §13 quasi-experimental estimate
names its design, its certified controls, its window check, and its audit
calibration — never silently pooled with interventional evidence);
**judge-anchoring** (judge-relative, unanchored scale); **tail flags** (mean
improved, a quantile/CVaR worsened); **rankability (curl)** (no total order);
**asserted transfer** (pooling rests on a user claim, revocable); **drift
revocation** (a monitor emptied a hash-granted pool); **measurement-feedback**
(instrument outputs reach the subject's own inputs — the observer-effect
certificate failed; the scores are part of the system, not just readings of
it); **redacted-view** (the judge saw a projection, not the full output —
the bar and any certificate are scoped to the projection, §18.2). e.g. `+15% ·
P(improvement)=78% · N=6 · adjusted for {cases} · unadjustable:
{model-version} · judge unanchored`. The user chooses: iterate accepting the
caveats, or settle. **Caveats are visible; de-confounding is never forced** —
this is what makes trusting a fast N=1 estimate reasonable rather than
reckless. `tags: none` is what licenses the agent to say "no caveats."

## 15. `whip improve` — reserved, needs its own effort

The optimization mode (GEPA-style / Bayesian search over a parameterized
intervention, typically a prompt, with the accumulated gauge suite as the
objective and the same ledger/EVSI machinery underneath) gets the reserved
verb **`whip improve <gauge>`** — decided 2026-07-03 that it will exist, and
that **it needs a full design effort of its own**, not a paragraph here. Open
design space includes: the search-space declaration (what is parameterized and
how), budget/stopping for search rather than inference, how candidates enter
the lineage DAG without polluting edit-type priors, guardrails against
objective gaming of the judge, and whether improved candidates are proposed or
auto-applied (authority ebb again). Out of scope for this note beyond
reserving the verb. **Design effort opened 2026-07-03: see
`improve-design-note.md`** (frame + core decisions settled in principle;
surface syntax and several policies open there).

## 16. Formal-model plan (model-first)

- **Maude — `evidence-transfer.maude`.** Rule graphs + an edit + slice/taint
  computation + pooling verdict; executable semantics runs both versions and
  compares the trace at s. Coverage: one fixture per §7 row. **Bite:** a
  consume-interference edit (§9.2) that diverges the arriving context while a
  naive fact-edge-only slice claims a hash match — the model must reject the
  naive verdict. (No-solution targets need the `RESIDUAL:Cfg` soup variable to
  actually bite.)
- **Lean.** The distributional non-interference theorem via coupling (§6):
  deterministic rule steps + oracle choice points; Δ outside the
  (flow+anti)-closed slice ⇒ coupled runs identical at s ⇒ trace measures
  equal. Fits the hermetic Mathlib-free layer.
- **Identification (later, if §12 is built):** the adjustment-set computation
  over the certified graph is itself a graph algorithm with a soundness
  obligation (a claimed backdoor set must actually block every backdoor
  path) — same coverage+bite discipline, likely Maude over small graphs. The
  §13 control-classification (treated vs. certified-control gauges for an
  edit) carries the same obligation and reuses the same slice model.

## 17. Decisions recorded, and remaining forks

**Settled in principle (Jack, 2026-07-03):** internal seam (rejected: external
product-to-product seam); `gauge` and `mark` enter the language (revising
"language ~none"); verb set `pin` / `suppose` / `settle` / `evidence` / `why`
/ `evidence carry`, with `improve` reserved pending its own effort; naming —
`suppose` not `try`, `evidence` not `eval`, `mark` separate from `milestone`;
the two surface principles (§4.1) including **no sample counts anywhere**;
anchors via the human-ask door; certification is a grade of `settle`, not a
separate verb; the identification layer (§12) and the quasi-experimental
layer (§13) folded in as engine design.

**Remaining forks (for the ADR):**

1. **Generator-hash contents** — include the resolved provider profile
   (recommended), accepting that provider bumps visibly warm-start kernels;
   the alternative is silent staleness.
2. **Slicer granularity** — rule-granularity first (recommended); per-field
   (X2 v2) is a pure-precision upgrade the lazy ledger applies retroactively.
3. **Verdict implementation** — taint-on-demand (reuses the integrity
   propagation directly) vs. precomputed Merkle keys (O(1) lookup, free
   reunification). Recommended: both faces of one check — the propagation
   *defines* the slice, the hash *keys* the ledger.
4. **Identity canonicalization** — alpha-equivalence so pure renames don't
   orphan evidence (IR normalization already absorbs formatting/comments);
   how far to normalize is an ADR question.
5. **Identification/quasi sequencing** — §12 and §13 are *engine* refinements
   invisible at the surface; whether they land in v1-of-the-build or as the
   first upgrades is a cost/complexity call for the ADR (the v1 floor is
   honest without them — everything unidentified is simply tagged and priced
   at regeneration cost).

Not a fork: **closing the dependency graph over signal/coordination/consume
edges (§9.1) is a prerequisite to any sound verdict.**

## 18. Architecture, dependencies, relationships

Four pieces — **ledger** (§8, workspace store) + **slicer** (§7, reusing the
IFC reach/integrity machinery) + **checkpoint substrate** (restorable context;
marks ride it) + **pluggable aggregators** (§11) — with the **identification
layer** (§12) and the **quasi-experimental layer** (§13) as engine logic over
the slicer's graph and the lineage timeline, fronted by the §4 surface. The
irreducible floor — essential, not accidental complexity: (a) the sound
dependency closure (§9.1); (b) the consistent checkpoint cut *including
coordination state* (restorable-context DR; mirrored in the durable-object
tracker's downstream-customer note — without it "regenerate from c" is not
well-defined); (c) the drift backstop (§9.4).

- **Language surface:** exactly `gauge` + `mark`. Scorers are ordinary
  effects (`judge via coerce | prompt | exec | labels`); deterministic
  invariants are `test`; release gating gets `--gauges` (statistical bars
  checked at certify grade, budget-capped).
- **Root agent (DR-0026):** the primary consumer of the surface (`--json`
  everywhere) and the interpreter of the scalars; gaugewright consumes the
  same surface as the dogfood/requirements engine.
- **Open-core seam:** engine + surface are core-shaped; enterprise dashboards
  and hosted experiment stores, if ever, sit behind the standard seam. No
  licensing artifacts implied here.

### 18.1 Build spine, and the minimum lovable increment

No single document previously named the build order or the smallest slice
that delivers value. The spine, in dependency order: **(0)** the
dependency-closure prerequisite (§9.1, four edge kinds) — soundness floor
for everything certified; **(1)** `gauge` + the ledger v1 floor + ambient
scoring + `whip evidence` — **the minimum lovable increment**: no
do-machine, no transfer, no branches, no statistics beyond
Beta-Bernoulli-per-partition, yet already a product ("your workflow's
quality, quantified from the runs you were doing anyway, visible before
anyone asks"); **(2)** `mark` + scenarios + `suppose` (needs consistent
cuts from the storage plane); **(3)** `settle` (racing + stopping);
**(4)** slice-hash transfer (evidence compounds across edits); **(5)** the
engine layers (§12 identification, §13 quasi) and measurement upgrades, in
whatever order evidence of need dictates; **(6)** `improve`
(`improve-design-note.md`) and the versioned-workspace dependencies as they
land. Everything after step 1 makes the asset *compound*; step 1 alone
creates it. The ADR should preserve this shape — the cathedral has a chapel
worth shipping first.

### 18.2 The evidence plane's own IFC (settled in principle, 2026-07-03)

The corpus uses IFC machinery *for* evidence (identity, certificates);
this subsection answers what labels the evidence itself carries — the gap
registered in §20, now designed.

**The frame: a third orthogonal axis.** Scope semantics established
scope ⊥ isolation (versioned-workspace note, "Scope semantics"); the
evidence plane adds **scope ⊥ clearance**. The workspace plane is global in
*scope* — branch-agnostic, monotone, written greedily — but global scope
never meant label-free: "workspace-visible" answers which timeline a write
lands on; labels answer who may read it back out. The knowledge plane is
not a declassification device.

**Ledger rows are telemetry-shaped: references, cheap to hold, checked to
open.** The row (mechanics + content-addressed refs) carries a low label;
confidentiality lives behind the derefs (output-ref → content,
checkpoint-ref → trace), where the reader-set check fires — the
mechanics-vs-content split already solved for telemetry, one plane up.
(Noted, not built for: hash *equality* is a one-bit covert channel.)

**The engine is inside the membrane; the IFC problem lives entirely at the
surfaces.** Warm-starting, pooling, EVSI arbitration, drift monitors — the
runtime reads the ledger as trusted computing base. Every place evidence
*leaves* the engine is an enumerable door: judge calls, proposer turns, CLI
display, the ask surface, exports. One refinement to the instrument
concept: the instrument's *bookkeeping* is TCB; its *judges are not* — a
judge call is a provider flow like any other, governed by the rule below.

**The no-new-readers rule.** Reader sets are **derived from actual
flows**: data that flowed to provider P during the workflow's own
inference has P in its reader set, so a judge re-sending the same content
to the same provider adds no reader and needs no configuration — the check
passes silently in the common case, which ambient measurement requires.
It flags exactly the enlargement cases: **first provider contact** (the
gauged site is exec/deterministic over confidential data whose output
never touched an LLM); **cross-provider judging** (often *desirable* —
§10's self-model-sycophancy concern — and legal via a one-line grant);
**composite views** (pairwise comparisons and worst-slice bundles show a
provider data combined across runs) — v1 treats same-provider as
same-reader without distinguishing purposes or view shapes; the
purpose-granularity refinement (live processing vs. offline corpora) is
noted as available if a compliance regime ever demands it, not designed.
Grant syntax and home (provider profile vs. capability registry) is an
ADR detail inside existing session-root machinery.

**Redaction needs no new grammar.** Judging less than a site's full output
is already expressible: `redact … keep […]` at the site, gauge the
redacted element. What is *not* optional is the honesty consequence: a
judge that saw a projection measured the projection — the
**redacted-view** tag (§14), with bars and certificates scoped to the
projection.

**Aggregation is declassification; the pooling shape is settled:** the
engine **pools everything internally** (it is TCB; statistics never
fragment), and every *surface* computes the reader's maximal visible
sub-pool, displayed with an honest deficit — `N=64 (12 hidden from your
view)`. Estimates become clearance-relative ("over your visible
partition" — an honesty-tag matter); the thin inference channel through
EVSI hints priced on hidden data is noted and accepted (adversary-free,
internal). Upgrade path: **aggregator flow signatures** — aggregators are
kernels, and the X2 producer-side signature pattern lets a Beta-posterior
plugin declare "reveals counts and a rate, not contents," capping its
output label below the join. Differential privacy is that road's
principled endpoint; horizon, not proposal.

**Reader census, settled by existing machinery:** `whip evidence` shows
aggregates and slice names (low-label under signatures); `whip why`
drill-down to examples is a deref door; anchor asks show content through
the I-IFC8 ask door, which checks content clearance, not just delivery;
scenarios are *references* — labels ride the underlying content, and
scenario/package *export* is egress through the state-surface/`redact`
machinery; campaign records and telemetry export carry refs + mechanics.
The integrity axis was already built under other names — the honesty tags,
the synthetic respondent's low-integrity source, trusted-by-declaration
dataset labels, asserted transfer — an integrity lattice wearing a product
hat; only confidentiality was the gap.

**The proposer is the hard residual — owned by the improve note** ("The
proposer"): improve makes the program a derived artifact of the data, and
the response is stratified by threat model (statistical control
in-workspace, hard checks at source-egress doors). Policy tiers there are
**deliberately open**.

**Registered undesigned door: cross-workspace evidence sharing.** §13's
staggered-adoption payoff ("v2.1 improved gauges across 14 adopters") is
the corpus's only flow where evidence *leaves* a workspace, promised by a
product bullet and designed by nobody. Predictable shape: per-workspace
opt-in; aggregate-only under aggregator signatures (never rows or
traces); a k-adopters threshold so the author never sees one adopter's
numbers; and a rendezvous service that is naturally *hosted* — open-core
seam territory. Registered here; designed when packages-with-gauges
become real.

## 19. Non-goals

- **Not an eval platform** — no dataset-as-a-service, dashboards, or
  experiment-tracking product (part of why `eval` was rejected as a name).
- **Not invented methodology** — established mathematics imported from
  adjacent fields: dependency calculi and partial evaluation (DCC, Futamura),
  self-adjusting computation, psychometrics (IRT, Dawid-Skene, test equating),
  causal inference (do-calculus identification, transport, counterfactual
  bounds, blocking, the quasi-experimental design canon and its modern
  estimators), game-theoretic probability (e-values), conformal prediction,
  combinatorial Hodge theory, Bayesian decision theory (EVSI). Precision by
  importing better tools, not inventing a QC methodology.
- **Not a mandated methodology** — the user drives; the system models whatever
  they do; rigor is one verb away, never required.
- **Not run-until-N** — no fixed sample sizes, no batch rituals, no surface
  that accepts a sample count.
- **Not a v1 language decision** — depends on post-v1 substrate (checkpoints,
  root agent) plus the §9.1 prerequisite; build timing open.

## 20. Open questions

- Slice-hash canonicalization details (per-element hashing,
  alpha-equivalence depth, stability across hosts — shared concern with DO
  Decision 4).
- Ambient judging economics: the adaptive scoring policy (when is a live run
  worth judging) and its interaction with provider budgets.
- Gauge grammar details: site designation (`on summarize.extract`), segment
  scopes via marks, multi-gauge sites, `expect` forms, the derived-gauge
  `inputs` clause's exact I/O contract. (The `judge via` forms and derived
  gauges themselves are settled — §4.2.)
- Identification in practice: overlap/positivity diagnostics (when does the
  engine trust ambient adjustment), how instrumentation suggestions are
  surfaced without nagging, and how far confound quantification goes
  (causal-graph bookkeeping vs. enumerate-and-tag).
- Quasi-experimental practice (§13): scale alignment across certified control
  gauges; minimum-traffic thresholds below which designs decline to run;
  attribution-notice ergonomics (helpful nudge vs. nag). The
  deliberate-reversal **consent surface is owned by
  `improve-design-note.md` ("Canary" section)** — one surface governs all
  live-touching branches; **settled in principle there 2026-07-03** (grant
  object, exposure ladder incl. shadow, `design: randomized` canaries);
  residual = grant surface syntax + carve-out vocabulary.
- **The evidence plane's own IFC story — SETTLED in principle 2026-07-03
  (§18.2).** The registered gap's design pass happened: third axis
  (scope ⊥ clearance), refs-not-content ledger rows, engine-inside-the-
  membrane, the no-new-readers rule for judges, pooling shape
  (pool-internally / view-per-reader with honest deficit), redaction via
  the shipped construct + `redacted-view` tag, the cross-workspace
  evidence door registered. Residual opens: **proposer leakage policy
  tiers** (deliberately open — improve note, "The proposer"); grant
  syntax/home for reader-set extensions (ADR detail); the
  purpose-granularity refinement (noted, undesigned).
- The edit-type taxonomy for lineage priors: seeding, and keeping learned τ
  honest with few observations per type (mitigated by §13.2's retrospective
  training).
- The `whip improve` design effort (§15) — everything about it.
- Retention: how long scenarios/checkpoints stay regenerable, and what expiry
  does to scenario-scoped evidence. **Owned by
  `versioned-workspace-research-note.md` ("Open questions" — branch GC roots
  subsume scenario/evidence retention);** this note only records the
  requirement.
- The graded-transfer horizon (§6): whether path-capacity/contraction bounds
  become estimable enough to use.
- **Measurement-feedback handling (§10):** tag-and-warn (lean) vs.
  strict-mode refusal when the observer-effect certificate fails; and how
  the instrument boundary is drawn for custom scorers beyond gauge
  declarations (shared with the versioned-workspace note's
  instrument-boundary question).
- **Regeneration side-effect containment** — a counterfactual run's replayed
  effects must not touch real project state or the outside world. **Settled
  in principle 2026-07-03** by the versioned workspace
  (`versioned-workspace-research-note.md`, "The boundary identity" and
  "Per-door containment policy"): storage-plane effects are contained
  completely by branch semantics; egress doors get replay/divert/live
  verdicts under the counterfactual non-egress invariant. Rebased 2026-07-03
  on that note's scope semantics: posture = grants per category × role
  (subject vs. instrument). Five sub-forks and the store→world pump audit
  remain open there; the consent surface — narrowed to the canary grant
  grammar — is owned by `improve-design-note.md` ("Canary").
