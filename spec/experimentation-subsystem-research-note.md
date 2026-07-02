# Experimentation & Evaluation Subsystem — Product Research Note

**Status: RESEARCH NOTE (pre-ADR).** This captures a design conversation about
how a developer refines and gains confidence in a stochastic WhippleScript
workflow. It defines *what good looks like* and the technological/mathematical
framework that would enable it. **Nothing here is decided, scoped, or committed
to v1** — it may be deferred, folded into gaugewright, or scrapped. It is written
down so the target is not lost.

## 1. The problem

Whip already has deterministic testing (`test`/`given`/`stub`/`run`/`expect`),
observability (`std.telemetry`), and model surfaces (`coerce`/`tell`/`decide`).
None of these answers the real question a developer of an LLM workflow has:
**"is this workflow doing what I want, and did my change make it better?"** —
where "the workflow" is *stochastic*, so a single run proves nothing and "better"
is a judgment, not a mechanical metric.

Two tempting non-answers, both rejected earlier in the conversation:
- **"Evals = coerce + a workflow."** A scorer is a coerce and an orchestration is
  a workflow, so if that were the whole story we would add nothing. Special
  support is only justified if there is a **semantic link** that plain composition
  cannot express ergonomically.
- **"Evals = observability."** `std.telemetry` measures *execution mechanics*
  (latency, throughput, error rates) and exports them to a backend that already
  does windowing and alerting. It fundamentally cannot judge *output quality* —
  that requires *computing* a judgment. But once a judgment (a score) exists,
  emitting/alerting on it is observability's job. So *live quality monitoring*
  reduces to `coerce`-score → `std.telemetry`, and needs no new construct.

What neither covers — and where the real value is — is **grounded, statistically
honest, fast iteration on a stochastic program**: freeze a real scenario,
re-run the uncertain part, judge it, and accumulate evidence over time as you
change the workflow.

## 2. The core reframe — it is a *subsystem*, not a language feature

The decisive realization: almost none of this belongs in `.whip` syntax.

- The **evidence** is living runtime state (accumulated posteriors over many
  runs), not a source declaration.
- **Experiments are anchored to runs, not code** — they reference a checkpoint in
  a real event log at a program version, which is a runtime artifact.
- **Interventions target anything** (a prompt, a rule, the whole workflow), so
  there is no natural syntactic home; an experiment is transverse.
- The loop is **interactive and cooperative** (human + root agent) — a tooling and
  session activity, not a static artifact authored once.

So this is an **experimentation subsystem = a store + a Bayesian/causal engine +
the root-agent development loop**, built on top of two substrates whip is already
heading toward:
- **Checkpoints / restorable context** (see `decision-records/restorable-context.md`):
  a checkpoint is a consistent cut across transcript + event log + file store. It
  is exactly the "frozen scenario" an experiment replays from.
- **The root/session agent** (DR-0026): the cooperative human+agent loop that
  drives refinement and narrates results.

The language contributes almost nothing new: the **scorer** is an ordinary
`coerce`/`prompt`; a durable "this workflow must always satisfy X" invariant is an
ordinary `test`. Everything else is runtime + tooling. This keeps the workflow
language clean and means the subsystem sits naturally beside **gaugewright**
(the dogfood/requirements engine) rather than inside the compiler. It also
explains why it is honestly deferrable: it depends on the checkpoint substrate
*and* the root agent, both post-v1.

## 3. What "good" looks like

A developer works on a workflow with the root agent, informally. They run it (a
test environment: signals fire, prompts trigger — the **event log is the run**).
They change things and rerun. The system quietly models every change as an
intervention and tells them, through a few honest numbers the agent explains in
plain words: *is it better, by how much, how sure, and what is confounding that
judgment* — with a ranked leaderboard when weighing competing options and a hint
about the most informative next experiment when they are stuck. When a judgment
is uncertain and they care, they step into a mode that runs the minimum
controlled experiments to settle it; when they want tuning, an optimization mode
searches for them. **They are never forced into a protocol.** Evidence
accumulates and self-corrects — a hasty N=1 call is revealed as more data
arrives — and the sophisticated statistical machinery stays invisible unless
asked.

The feel: **fast, creative, and scientific** — the scientific method without the
formality.

## 4. The capture mechanism

An **experiment** is:
1. a **checkpoint cut** — a frozen prefix of a real run's event log (the scenario,
   deterministically replayed from *recorded* outcomes);
2. a **regenerate set** — the stochastic step(s) to re-execute fresh (e.g. a
   specific `tell`/`coerce`), everything else replayed;
3. a **scorer** — a `coerce`/`prompt` judge, a deterministic check, or a human/
   agent judgment;
4. an **assertion / question** — the statistical claim being evaluated.

Properties that fall out of anchoring to the event log:
- **Zero-ceremony capture.** The experiment is *inferred from natural activity*:
  edit a prompt + rerun is recognized as an intervention and modeled — no forms,
  no "declare a hypothesis." The agent may merely confirm ("looks like you're
  testing whether that tweak helps; I'll track it").
- **Auto-expiry.** The frozen prefix references program elements at a version. On
  a new version whip tries to replay it; if the referenced parts are gone, the
  experiment auto-invalidates. Evals expire when the program part they measured
  does.
- **Optimizers get an objective for free.** Regenerate with a *candidate* prompt
  instead of the original and score: the suite becomes a fitness function a
  GEPA-style optimizer maximizes over. Auto-optimization is "the loop, automated,
  over a parameterized intervention."

## 5. The loop and its modes

The unit is an **experiment (an intervention + a measured effect)**, not a
"label." The loop is the scientific method: **observe → hypothesize → intervene
→ measure → accumulate.** Because an intervention *changes the program*, each
experiment carries a version, and the accumulated evidence is a **DAG over
revisions** (before/after *is* the experiment) — tied to whip's `revision_epoch`
lineage.

The system is a **modeler first, protocol never.** Rigor is a mode you step into,
not the default:
- **Exploratory (default) — observational.** Infer the best statistical/causal
  model from whatever the user + agent naturally do. Fast, N=1, confounded, and
  *honest about it* (§7).
- **Hardening — interventional.** Run *designed*, controlled, replicated
  experiments until an evidence bar. Clean causal signal, on demand. Where
  confounds get eliminated and competing hypotheses get discriminated (§6).
- **Optimization.** Bayesian optimization / GEPA over a parameterized intervention
  (a prompt), with the accumulated suite as the objective.

Same Bayesian/causal engine; two ways of feeding it (observe cheaply vs
experiment deliberately). This is what lets it be fluid by default and rigorous
on request.

## 6. The statistical / causal framework

Implementation is **Bayesian** — it is the exact fit for the desiderata:
- **N=1 is informative.** Update from a prior; one observation yields a belief
  with wide uncertainty that narrows as evidence accrues.
- **Peek anytime, no sin.** Posteriors are valid after every observation — no
  alpha-spending correction — so continuous checking never corrupts the stats.
  This is *why* the loop can feel fast.
- **Run-until-evidence, not run-until-N.** Stop at an evidence bar
  (`P(θ ≥ t) ≥ c`, or interval width < ε), sampled adaptively, budget-capped.
  Not a fixed N, not a data-science ritual.
- **Self-correction.** A hasty N=1 call slides as the likelihood swamps the prior;
  the system can flag when accumulated evidence contradicts an earlier judgment.
- **Strong inference (not binary).** The interesting question is usually *which*
  of several changes/explanations is right, not "did it pass." → **Bayesian model
  comparison** (posterior odds over competing hypotheses), and optimal
  experimental design aimed at **discrimination** (design the experiment that best
  rules out alternatives, Platt-style), not just parameter precision.
- **Optimal experimental design / active learning.** The suggestion engine: pick
  the most informative next experiment by expected information gain / expected
  discrimination — offered when the user wants direction, never required.
- **Judge reliability is modeled, not surfaced.** A `coerce`-judge scorer is itself
  stochastic; its reliability is a nuisance parameter in the model, hidden from
  the normal workflow.
- **Hierarchical pooling (optional, later).** Multiple experiments over one
  workflow share structure; partial pooling lets each borrow strength, reducing
  regenerations per experiment.

## 7. Confounds as a first-class, actionable output

The north star for the observational default: **represent the confounds and feed
them modeler → agent → user**, so a confounded estimate is *informed*, not blind.
This is tractable because the system sees everything: when comparing two
observations, the confounds are **the diff of their conditions minus the
intervention of interest** ("you changed the prompt, but the cases also differed
and the model version bumped"), enumerable from the event log + version lineage,
with the causal model quantifying how much each muddies the estimate.

So the evidence unit is **an estimate + its confounds**, e.g.
`+15% · P(improvement)=78% · N=6 · confounded by {cases, model-version}`. The
user then decides: iterate now accepting the caveat (fast), or step into an
interventional experiment that holds the confounds fixed and isolates the change.
**The system makes confounds visible and lets the user choose — it never forces
de-confounding.** This is what makes trusting a fast, observational, N=1 estimate
reasonable rather than reckless.

## 8. The surface — established scalars, agent as interpreter

Showing a non-expert a posterior distribution is the wrong UX; inventing bespoke
plain-language "confidence bands" would be uncalibrated and vibey. The primary
surface is a small set of **established scalar measures** (nothing invented), and
the **agent translates them to plain language** on request:

- **Did my change help?** → **P(improvement)** (0–100%) + **effect size** (% lift
  or standardized) + **N**.
- **Which of several is best?** → **ELO / TrueSkill ratings** — a scalar per
  variant, built from noisy pairwise regenerated comparisons, incrementally
  updated; everyone already reads "higher = better." The natural home for strong
  inference across competing variants: a leaderboard.
- **What should I test next?** → **expected value of information** — the OED
  objective as a scalar per candidate experiment.
- **Overall on a suite** → **win-rate / mean score + N**.

Each scalar **carries its confound tags** (§7). Three layers: **scalars are the
surface; the Bayesian/causal model is the backend; the root agent is the
interpreter** ("that TrueSkill gap means A is clearly ahead; P(improvement) is
only 60%, so I'd run a few more"). Power users get **progressive disclosure** —
read the scalars, or open the full posterior. The scalars are legible because
they are comparable numbers and calibrated because they are direct summaries of
the posterior, with no lossy translation layer in between.

## 9. Architecture, dependencies, relationships

- **Substrate:** checkpoints / restorable context (`decision-records/restorable-context.md`)
  provides freeze-and-regenerate; the root/session agent (DR-0026) provides the
  cooperative loop and the legibility surface.
- **Language surface: ~none.** Scorers are `coerce`/`prompt`; durable invariants
  are `test`. No new workflow syntax is implied by this note.
- **gaugewright.** This experimentation loop is a strong candidate to *be*
  gaugewright's core (the dogfood/requirements engine), or to sit adjacent to it.
- **Open-core seam.** An experiment *store* + dashboards are enterprise-shaped;
  the *engine* + agent loop are core. Kept behind the standard seam; no licensing
  artifacts implied here.

## 10. Non-goals / explicitly out of scope

- **Not an eval platform.** No dataset-as-a-service, no dashboards, no
  experiment-tracking product — those are external / gaugewright / enterprise.
- **Not reinventing evals.** Reuse established statistics (Bayesian updating, model
  comparison, ELO/TrueSkill, VoI); do not invent a QC methodology.
- **Not a mandated methodology.** The user drives; the system models whatever they
  do and *offers* optimal experiments; rigor is opt-in via modes.
- **Not a v1 language decision.** Depends on post-v1 substrate; may defer or be
  scrapped. This note defines the target only.

## 11. Open questions (for a future ADR, if pursued)

- The exact statistical starting point (Beta-Bernoulli + `P(θ≥t)≥c` sequential
  stopping first; OED, model comparison, hierarchical pooling as later layers?).
- How the observational model represents and quantifies confound impact concretely
  (causal graph bookkeeping over condition-diffs — how far to go).
- The precise scalar set and how the agent's plain-language mapping stays
  calibrated to the posterior.
- Where the experiment store lives and how it keys to runs + `revision_epoch`.
- Relationship boundary with gaugewright (is it gaugewright, or a substrate
  gaugewright uses?).
- Cost governance for regeneration (adaptive-N budgets, caching, sampling).
