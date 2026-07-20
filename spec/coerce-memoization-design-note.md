# Cross-run coerce memoization — design note (DRAFT for discussion)

**Status: DRAFT 2026-07-20 — for Jack's reaction; nothing here is decided or
built.** Origin: the v0.2 inference-cache thread (`spec/inference-cache-note.md`,
`spec/v0.2-milestone-tracker.md` Cluster C). This is the whip-side,
semantics-bearing half of caching; the provider-side (prefix-cache) half is
purely economic and proceeds independently (G1/G2).

## 1. The semantic split this note lives on

Provider prompt caching never changes results — it caches *prefill compute*;
the model still samples fresh output. Whip-side **response memoization** —
returning a stored result for an identical call — does change semantics. So
"recompute vs. reuse" is exclusively a whip-side, language-level question, and
it is a *per-declaration* property: some coerce decisions are conceptually pure
functions of their inputs ("classify this ticket"), others are deliberately
re-sampled ("propose a fresh idea").

## 2. What whip already has (the substrate is mostly built)

- **Within one instance:** effects are durable and exactly-once — a completed
  coerce is never re-executed on resume/replay. "Don't recompute within a run"
  already holds; this note is only about *across* instances.
- **Across instances, precedent:** the delta-kernel **exec result cache**
  already reuses deterministic `exec` validator results workspace-wide, keyed
  by content — so "cross-run result reuse, opt-in by construction" is an
  established pattern, not a new invention.
- **The right cache key exists:** the execution fingerprint is
  `H(effect.input_json, upstream ids)` and the coerce effect key folds in the
  **model id** and the schema (S2b, DR-0014 amendment). A memo key is this
  minus the run-scoped parts (program_version / revision_epoch / instance):
  `H(rendered prompt, output schema, provider, model)`. Template or schema
  edits change the key automatically; a model upgrade changes it automatically.

## 3. Proposed surface (strawman)

```whip
coerce classify(title string, body string) -> Classification {
  memo                     # ← opt-in: "this decision is a function of its inputs"
  prompt """..."""
}
```

- **Default stays recompute.** Memoization is the author's explicit assertion
  that the decision is input-determined — whip cannot verify purity, the same
  way it cannot verify an `exec` validator is deterministic; the declaration is
  the honesty boundary (progressive rigor: zero-setup default unchanged).
- Store: workspace-scoped (same scope as the exec result cache). Never crosses
  a workspace; a hit records `served_from_cache: true` in run metadata so
  provenance, `whip runs`, and evidence can tell replayed decisions from fresh.
- A miss executes normally (provider egress, IFC checks, spend); the result is
  stored under the memo key at completion.

## 4. Semantics that need to be right (the real content)

**IFC.** A cache *hit* performs no provider egress — the confidential-read →
model egress check applies at *miss* time (when the call actually leaves).
The stored value's label situation is clean because the memo key contains the
full rendered input: identical key ⇒ identical inputs ⇒ the label derivation
at the use site is the same as if computed. The one rule to enforce: the memo
store is inside the workspace trust boundary (same posture as facts), so no new
flow is introduced. The IFC checker should still treat a `memo` coerce as an
egress point statically (it *may* miss), i.e. no static-check relaxation.

**Evidence & improve.** Two options when a campaign evaluates candidates:
(a) memo applies — cheaper evals, but a candidate whose coerce inputs are
unchanged re-reads the *same* decision, masking model-side variance the judges
might care about; (b) campaigns bypass memo (regeneration means regeneration).
Leaning **(b) bypass during improve/settle/suppose regeneration** — those
subsystems exist to measure the distribution, and serving memoized responses
would quietly narrow it. Normal runs use memo; regeneration measures fresh.
(Open question 3.)

**Staleness.** Nothing expires by content: model id + template + schema are in
the key. Residual staleness is provider-behavior drift *behind the same model
id* (e.g. a hosted alias silently updated). Options: a declaration-level TTL
(`memo 7d`), a `whip coercion memo clear` operator command, or both. TTL adds
a time dependency to something declared input-pure — my lean: **no TTL in v1**,
just the operator clear command; add TTL only on demonstrated need.

**Failure results.** Memoize successes only. A failed/timeout coerce is not a
value; it never enters the store.

## 5. What this buys

- Cross-run: identical decisions (re-runs over unchanged corpora, cron-shaped
  workflows re-classifying stable inputs, fan-outs sharing sub-decisions) cost
  zero provider spend and zero latency after first computation.
- Composes with, not replaces, provider caching: memo eliminates the *call*,
  prefix caching cheapens the calls that remain.
- It is also the honest scope limit: agent *turns* (tool loops, workspace
  mutations) are NOT candidates — they are effectful by construction. Only
  `coerce`/`decide` (pure input → typed value) qualify. This note deliberately
  proposes nothing for `tell`.

## 6. Open questions for Jack

1. **Surface:** `memo` keyword on the coerce declaration (strawman above) vs. a
   store-level/config knob vs. both. The declaration feels right (it is a
   semantic property of the decision, not deployment config).
2. **Keyword name:** `memo` / `cached` / `pure`? (`pure` overclaims — the model
   isn't pure; the *policy* is reuse.)
3. **Improve interplay:** confirm bypass-during-regeneration (lean (b) above).
4. **Scope granularity:** workspace-wide store, or per-workflow namespacing
   within it? (Workspace-wide maximizes reuse; the key already contains the
   full prompt, so cross-workflow collisions are by-construction identical
   calls. Lean: workspace-wide, matching the exec cache.)
5. Is this v0.2 scope at all, or noted-and-parked until a use case demands it
   (it is demand-gated by nature — no user has yet asked to re-run identical
   decisions cheaply)?
