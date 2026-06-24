# Sum types: data-carrying enum variants

Status: spec drafted 2026-06-10 from decided design
([`language-ergonomics-tracker.md`](decision-records/language-ergonomics-tracker.md) C1).
Stage: spec -> modeling -> implementation + testing -> review.

## Framing

**A data-carrying variant is a tagged record; a sum type is a discriminated
union of variant records.**

- `enum` today = a closed set of bare variants (`Accept | Revise | Blocked`).
- `enum` with payloads = a closed set of variants, each optionally carrying
  typed fields.

The feature exists to make illegal states unrepresentable: an `Approved`
outcome that carries a `score` and a `Rejected` outcome that carries a
`reason` cannot be confused, and `case` over them is exhaustive. This matters
most for the typed outputs of model decisions (`coerce` and `decide`, the core
anonymous-coercion sugar that lowers to a generated `coerce` — see
[`language.md`](language.md)), which is
where the language's control flow actually branches — and it protects the
LLM-author audience from the largest bug class: forgetting that a payload is
present only for one outcome.

Sum types add no general computation. They are a more precise way to describe
typed workflow state and decision outcomes — the data-primitive side of the
philosophy, not the control-flow side.

## Surface

### Declaration

A variant is a name plus an optional brace body. The body reuses the class
grammar verbatim (`name type`, newline-separated, no commas):

```whip
enum ReviewOutcome {
  Approved {
    score float
  }
  Rejected {
    reason string
  }
  NeedsInfo {
    questions string[]
  }
}
```

A variant with no body is a bare variant. `enum ReviewStatus { Accept Revise
Blocked }` keeps its current meaning, so this feature is **purely additive
and backward-compatible**: every existing enum is the all-bare-variant case.

Brace bodies — not Gleam's `Approved(score float)` parens — because parens are
a new delimiter and multi-field payloads would force comma or in-paren-newline
conventions the language uses nowhere else. The brace body is the existing
class-field parser, reused.

v1 payload field types: scalars, class references, arrays. Excluded from v1:
generics on variants, recursive variants (a variant whose payload contains its
own enum), nested sum-type payload fields, and methods — all general
data-structure-language features outside the workflow-data remit.

### `case` and binding

```whip
case outcome {
  Approved as a => {
    complete result { score a.score }
  }
  Rejected as r => {
    fail error { reason r.reason }
  }
  NeedsInfo as n => {
    askHuman choices ["clarify"] "{{ n.questions }}"
  }
}
```

- `as <binding>` binds the matched variant's payload, typed as that variant's
  record; field access is normal (`a.score`). This reuses the binding/typing
  machinery the terminal-output union `case` already uses.
- A bare variant takes no `as` (`Blocked => { ... }`).
- Payload access is legal **only inside a matched branch**. Reading
  `outcome.score` outside `case` is a check error — this prohibition is what
  forces the exhaustive handling the feature exists to provide.
- `_` / `default` branches are allowed; exhaustiveness otherwise reuses
  `validate_case_coverage` over the variant set.

`Variant as binding` rather than Gleam's positional `Variant(score)` because
`as`-binding is how every other binding in the language works, and it avoids
positional-destructuring fragility.

### Construction

Building a variant in source (rare — most arrive from `coerce`) reuses
nested-record construction with the variant name as the record:

```whip
record Decision {
  outcome Approved {
    score 0.9
  }
}
```

A bare variant is just its name (`outcome Blocked`), resolved against the
field's declared enum type — identical to setting a bare enum field today. The
discriminant is filled automatically; the author never writes it.

## The discriminant

Synthesized from the variant name; never written in source.

- Each variant lowers to a class carrying a reserved literal field
  **`variant`** whose value is the variant name string (`variant "Approved"`).
- `variant` is reserved inside variant bodies: an author-declared field named
  `variant` in a variant body is a check error.
- The tag is **visible in the lowering** (`whip check` shows the generated
  classes and their literal `variant` field), exactly as flows show their
  generated state classes. It is a documented, inspectable convention, not
  hidden state.
- The variant name is the single source of truth: the author cannot typo or
  desynchronize the tag, and the WS↔coerce contract is mechanical (below).

Not `$variant`: a `$`-prefixed key risks reservation in coerce / JSON-schema
surfaces. A plain reserved identifier (`variant`) avoids that and reads
cleanly in raw fact JSON.

## Lowering

Each `enum` with at least one data-carrying variant lowers to:

- One generated class per variant, named `<Enum>.<Variant>` (the `<Enum>.`
  namespace is reserved; user classes cannot start with it), holding the
  literal `variant "<Variant>"` field plus the variant's payload fields.
- The enum itself becomes a discriminated union of those generated classes.
- `case` lowers to dispatch on the `variant` field; each branch binds the
  matched variant class and runs its body.
- Generated classes and the union carry `provenance_class: "sum_type"` with
  source spans pointing at the variant in source; `check` groups them under
  their enum in the snapshot.

Runtime representation is internally tagged JSON:

```json
{ "variant": "Approved", "score": 0.9 }
```

WS's runtime `case` compares `variant` **exactly** — by the time a fact exists,
the schema-coercion boundary has already normalized or rejected the value, so no
fuzzy matching is needed on the WS side.

## Schema Coercion And coerce Backend Mapping

This is the integration crux. coerce is one backend for schema coercion, and the
coerce-specific behavior below is settled by evidence from coerce's Schema-Aligned
Parsing (SAP) source (`coerce-lib/jsonish`), not by assumption.

- A WS sum type corresponds to a **coerce union of classes**, each class carrying
  the literal `variant` field plus payload. In generated mode, WhippleScript
  declarations are the source of truth and the coerce artifact is emitted from
  them. In interop mode, a user may bind an existing `.coerce` union explicitly,
  and the checker must cross-validate it against the WS sum type. A WS
  data-carrying enum is WS's typed, case-dispatchable view of that coerced
  value.
- coerce union resolution is **deterministic scoring**, not best-effort JSON
  parsing: SAP coerces the model output against every variant, assigns a
  penalty score, and picks the lowest (`coerce_union.rs`). A literal
  discriminant is decisive — the matching arm scores 0 and short-circuits; a
  wrong arm either has its literal cast rejected outright or pays a large
  penalty (`DefaultFromNoValue = 100` for a missing required payload field vs.
  single-digit penalties for cosmetic noise). The right variant wins by a
  100+ margin.
- The discriminant **string match is forgiving** (`match_string.rs`: exact →
  strip-punctuation → case-insensitive → unaccented → substring), so a model
  emitting `"approved"`, `"APPROVED."`, or `" approved "` still resolves to
  the canonical `"Approved"` before WS sees the fact.
- coerce's own suite covers exactly this shape (`test_unions.rs`: discriminated
  picker unions, action-type unions), so the union-of-tagged-classes path is
  proven, not speculative — no live-model spike required.
- Graceful failure: if the model emits something so far off that no variant
  scores acceptably, coerce returns a coerce **failure**, which surfaces as
  `after <coerce> fails` — an existing, modeled branch. The feature degrades
  into a path the language already handles.
- `check` cross-validates the WS enum against the generated or referenced coerce
  type through schema hashes carried in the coerce request, so WS-side variants
  and the coerce union cannot silently diverge.

## Fixture

`fixture_coerce_value` returns one tagged variant deterministically (the first
declared variant) so local fixture runs exercise the success path; a
`--variant <name>` fixture knob selects another arm to exercise its `case`
branch without a real provider.

## Static checks

- An author field named `variant` in a variant body is a check error.
- A user class whose name starts with the reserved `<Enum>.` generated
  namespace is a check error.
- Payload field access outside a matched `case` branch is a check error.
- `case` over a sum type must be exhaustive (or carry `_`/`default`), reusing
  the existing coverage checker.
- A sum type used as a `coerce` output is cross-validated against the selected
  schema-coercion backend artifact.
- Bare-only enums are unaffected: no new diagnostics fire for them.

## Scope and staging

- v1: named sum types via `enum`, consumed primarily as `coerce` output and
  matched with `case`. Inline `decide` stays flat (anonymous field shapes);
  sum types are named.
- Held out: generics, recursion, nested sum-type payloads, methods.
- The construction surface (variant literals in `record`/`complete`/`fail`)
  ships with v1 but is expected to be rare; the dominant path is
  coerce-produces, case-consumes.

## Dependencies

Requires B1 (real body AST) for `case` payload binding and the unified
evaluator, and reuses the terminal-output union's binding/typing path and
`validate_case_coverage`. No new runtime concept: a sum type is classes plus a
discriminated union plus `case` dispatch, all visible to inspection.

## Modeling notes

- Dispatch determinism: a synthesized `variant` tag resolves to exactly one
  branch; no value matches two variants (property test over generated tags).
- Exhaustiveness soundness: a `case` accepted as exhaustive handles every
  variant a value can hold; a `_`-free `case` missing a variant is rejected.
- coerce round-trip: a value coerce produces for the referenced union parses back
  into exactly the WS variant whose `variant` tag it carries (golden tests
  against the coerce fixture).
- Failure path: a coerce whose output matches no variant lands `failed` and
  routes to `after <coerce> fails`, never an untagged or partial fact.
