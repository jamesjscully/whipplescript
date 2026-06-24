# DR-0023: `action` blocks — static rule-body effect-chain templates

Status: accepted 2026-06-17 (design); slices 1–2 landed 2026-06-17. Implements
the Phase-4 ergonomic item "rule-template/action-block expansion for repeated
effect chains" (spec/language.md §"Repeated effect chains", plan Stage 10b).
Design-first per the standing "do it right" bar. See "Implementation status"
below for what shipped.

## Problem

Workflows repeat identical effect chains — e.g. `tell <agent> → after succeeds
coerce <review> → after succeeds record <Result>` — across rules and across
provider routes. Today the only reuse is copy-paste, which obscures the durable
graph and drifts. `pattern`/`apply` (DR-era, implemented) already abstracts
reuse, but at the **top-level declaration** layer: a pattern expands into whole
declarations (schemas, rules, agents) before type-checking. It cannot abstract a
chain of statements *inside one rule body*. That intra-rule effect-chain reuse is
the gap this DR fills.

## Decision

Introduce a top-level **`action`** declaration: a named, typed, parameterized
template over rule-body statements, expanded **statically and inline** at each
call site, fully visible in the compiled IR.

```whip
action run_language_task(agent AgentRef, task LanguageTask, provider string) {
  tell agent as turn """markdown
  Write {{ task.language }} text to {{ task.artifactPath }}.
  """
  after turn succeeds {
    coerce reviewLanguageArtifact(task.language, task.artifactPath, turn.summary) as review
  }
  after review succeeds {
    record LanguageE2EResult { provider provider  language task.language  status "reviewed" }
  }
}
```

Invoked as a **call statement** in a rule body:

```whip
rule route
  when LanguageTask as task
=> {
  run_language_task(reviewer, task, "codex")
}
```

### Semantics

- **Static, inline expansion.** A call expands to the action's body with the
  arguments substituted for the parameters, spliced into the calling rule body
  *before type-checking and lowering* — the same expansion phase as `pattern`/
  `apply` and `flow`. There is no runtime call frame, dynamic dispatch, or
  recursion; it is syntactic reuse, **not** a general function system. A call to
  an undeclared action, an arity/type mismatch, or a (direct or indirect)
  recursive action is a compile-time error (mirrors pattern's on-stack check).
- **Parameter substitution.** Parameters are typed (`AgentRef`, a schema ref, a
  primitive). At expansion, parameter references are substituted: in prompt
  interpolation (`{{ task.x }}` — the existing mechanism), in `tell` targets
  (the `agent` param), in expression operands, and in `record`/effect payload
  field values. Substitution is by parameter name; the checker validates each
  argument's type against the parameter.
- **Hygiene.** The action's internal bindings (`turn`, `review` above) are
  **uniquified per call site** (suffixed with a deterministic call-site index,
  as `flow` does for synthetic step bindings) so two calls in one rule body — or
  a call plus a sibling effect — never collide. `after <binding>` references
  inside the action are rewritten to the uniquified names. Author-visible
  bindings introduced by the call are **not** exported to the surrounding rule
  unless the call itself is bound (see open question O1).
- **Preservation.** Expansion preserves source spans (diagnostics point at the
  action body and the call site), idempotency keys (derived from the expanded
  effect's rule + node position, stable across runs), effect dependencies, and
  effect/fact provenance — the same obligations `flow`/`pattern` already meet.
- **Lowering.** Expanded effects/facts carry the `rule_template` lowering-class
  identity already in the construct catalog
  (construct-lowering-preservation.md) — compiler-owned template nodes keyed by
  rule + call index — so the durable graph shows the expansion, not a hidden call.

### Relationship to `pattern`/`apply`

Complementary, not overlapping: `pattern`/`apply` generate **declarations**
(top-level); `action` inlines **rule-body statements** (effect chains). They
compose — a pattern may contain rules whose bodies call actions. No change to
`pattern`/`apply`.

## Open questions (resolve at implementation)

- **O1 — call result binding.** RESOLVED: **fire-and-forget in v0** (no `as`).
  A call cannot be bound; `name(args) as x` is a compile error. The action's
  purpose is to encapsulate the chain; exposing a terminal binding is an additive
  follow-up. Bounded surface, correct foundation.
- **O2 — control statements in an action body.** RESOLVED: v0 action bodies hold
  the **chain shape** — effect statements, `after` blocks, `record`, and `done`
  (consume/transform a fact; `done` was added to O2's original list because it is
  the natural chain-completion statement, e.g. `done item -> record Out { … }`,
  and the serializer already routes its binding through the renamer). `complete`/
  `fail`/`case`/`branch`/`cancel` are deferred (they entangle terminal/branching
  analysis with inlining) and rejected with a clear diagnostic. **Nested action
  calls inside an action body are also rejected in v0**, which keeps the call
  graph depth-1 — trivially acyclic, so the recursion case the design anticipates
  cannot arise yet (the Maude model still pins the general acyclic-gate invariant
  for when nesting lands).

## Implementation note (span consistency — learned 2026-06-17)

A first cut of slice 2 expanded calls by rewriting each rule's `BlockSource.text`
in the parser's `lower_program` (reusing `flow_expand::rename_text`). It works at
the parser/IR layer, but **panics the CLI**: `whip check`/`dev` (main.rs) re-process
rule bodies using **source byte spans** into the *original* file, so an
expanded-text byte offset applied to the un-rewritten source goes out of bounds.
`flow` avoids this because it emits **fresh** generated rules (new bodies whose
spans the downstream tolerates), whereas action expansion *mutates an existing
rule's body* while its span still points at the original source — desynchronizing
text and span.

So slice 2 must keep text and spans consistent for **both** consumers. Options to
weigh at implementation: (a) expand at the **raw-source** layer before any parse
(spans stay valid because everyone sees the same expanded source); (b) make the
mutated rule body carry a generated/safe span the CLI tolerates (the `flow`-style
path), and audit every main.rs site that slices source by a rule-body span to use
`BlockSource.text` instead; or (c) thread expansion through the body **AST** and
re-serialize, assigning generated spans. (a) is simplest and most robust; it is
the recommended approach. Slice 1 (the declaration) is unaffected and shipped.

## Gated implementation slices

1. **Parser/AST** — DONE. `Item::Action(ActionDecl { name, params, body })`;
   `parse_action`; reserved keyword `action`; formatter; round-trips and is inert.
2. **Expansion** — DONE. `action_expand::expand_action_calls`, a sibling pass to
   `expand_pattern_applications`/`expand_flow` run after flow expansion in
   `lower_program`. Each call is inlined with argument substitution + per-call-site
   binding hygiene + `after`-reference rewrite; diagnostics for undeclared action,
   arity mismatch, `as` binding, forbidden statement, and nested call. Modelled in
   `models/maude/tests/action-expansion.maude` (coverage + bite) per the
   model-first bar.
3. **Lowering/verify** — DONE for v0 surface. Expanded chains lower through the
   ordinary rule pipeline and appear in the durable graph (no hidden call); golden
   + negative unit fixtures in `action_expand.rs`; runnable example
   `examples/reusable-action-chain.whip` (+ committed `.ir` snapshot), in the
   all-examples `check` coverage. The `provider-language-e2e.whip` rewrite (the
   final-audit G-010 motivation) remains as additive follow-up.

## Implementation status

Slices 1–2 (and the v0 surface of slice 3) shipped 2026-06-17.

- **Expansion strategy (resolves the Implementation note below).** Approach (c):
  each rule body is scanned for call statements; the called action's body is
  parsed to AST, validated to the chain shape, has its internal bindings
  uniquified and parameters substituted, and is re-serialized via the (now
  shared) `flow_expand` serializer back into the rule body text. Re-serializing
  through the AST — rather than a flat text rename — is what makes substitution
  **position-aware**: a parameter named `provider` is substituted in value
  positions but the field *name* `provider` in `record R { provider provider }`
  is emitted verbatim. Binding *definitions* (`as turn`, `after turn`) are
  renamed in the AST; binding *uses* in value expressions (`{{ turn.summary }}`)
  ride the serializer's renamer closure.
- **Span safety.** Like `flow`, expansion produces rule-body text whose length
  need not match its original source span. To remove the prior panic *and*
  `flow`'s latent version of it, the CLI's `locate_span` now clamps to a valid
  char boundary, so a diagnostic on expanded text can never index past the
  source. Diagnostics raised *by* the expansion pass carry the calling rule's
  body span, which is always in range.

## Consequences

- Removes the last big copy-paste ergonomic gap (final-audit G-010) without a
  general function system or runtime call semantics — the durable, inspectable
  graph is preserved.
- Expansion is a third member of the static-expansion family
  (`pattern`/`flow`/`action`); they share the "expand before type-check, preserve
  spans/idempotency/provenance" contract.
