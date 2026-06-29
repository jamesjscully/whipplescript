# DR-0032 — Typed effect failures (the `EffectError` family)

Status: **proposed (2026-06-29).** Design decided in principle; staged model-first.
Generalizes the terminal-outcome `Failed` tag into a discriminated family over
effect kinds. Builds on the shipped narrowing core (discriminated-families-design.md
Stage 1a) — this record adds a *family*, it does **not** add narrowing. Intersects
the IFC track for the redaction boundary (DR-0027 confidentiality, the codex
error-surfacing redaction work). Supersedes discriminated-families-design.md §5.2
(the deferred "per-effect-kind failure schema" note), which is folded in here.
Formal model (planned): `models/maude/effect-error.maude` +
`models/lean/Whipple/Narrowing.lean` reuse. Durable status: this record.

## Problem

Failure is the least-typed part of whipplescript. Success payloads are declared
contracts (`output result R`); failure payloads *accreted*. Today:

- `after x fails as f` binds `f` **untyped** (lib.rs `"fails" => {}`), so `f.<any>`
  is unchecked — correct-but-unverified, not a lie, but no static help.
- The one typed failure is the generic builtin `TerminalFailed{reason, summary,
  effect_id, run_id}` (the `case x { Failed f }` payload).
- The on-the-wire failure base is **inconsistent**: `exec` failures expose
  `.message` where `coerce`/workflow failures expose `.reason` for the same
  human-facing role.

Two structural facts make this both tractable and urgent:

1. **No generics.** whipplescript deliberately excludes type parameters (the GADT
   thread). The clean cross-language answer — Gleam's `Result(value, error)` with a
   per-operation error *type parameter* — is unavailable to us by construction. So
   "type the failure" cannot be "make it generic"; it must be a **closed,
   finite-tag** construct. That is exactly a *discriminated family*, which we
   already have machinery for.
2. **The failing effect's kind is always statically known.** Every failure is
   observed at a site bound to a specific effect — `after <effect> fails as f`, or a
   `case` over a terminal union produced by a specific effect. There is **no site
   where the failing effect's kind is statically unknown.** So failure typing needs
   *static* narrowing by effect kind, not runtime tag dispatch.

We have **zero users / zero back-compat** today, which is the one-time window to fix
the inconsistent wire shape (a breaking change) for free.

## Decision 0 — The doctrine

Effect failure is a **closed `EffectError` discriminated family**: a common **base**
shared by every effect kind, plus per-kind **extras**, eliminated by *static*
narrowing on the effect's kind. We **commit to the base** (small, stable, safe) and
**defer the extras behind narrowing** — and because a per-kind extra is reachable
*only* through narrowing (which is per-effect-kind and additive), adding a variant's
extras later is **non-breaking by construction**. This is the Gleam factoring
(shared base + per-source variant) expressed without generics and without a runtime
union.

The dividing line for *what to do now vs later* is **breaking-cheaply-later vs
additive-later**:

- **Breaking-cheaply-later → now:** the runtime wire shape (field names / presence).
- **Additive-later → defer:** the entire type layer (variant catalogs, narrowing
  wiring, the redaction decision). Deferring it costs nothing because it can only
  *add* readable surface.

## Decision 1 — `EffectError` is an observer-origin family; v1 is static-narrow-only

- **Observer origin** (discriminated-families-design.md §5.4): user rules may
  *eliminate* an `EffectError`, never *construct* one. The kernel produces it.
- **Conceptual family, materialized as per-effect-kind built-in failure schemas
  sharing the base.** There is no runtime union value and no runtime kind tag in v1,
  because (Problem fact 2) the kind is always statically known at the read site.
  `after <effect> fails as f` statically resolves `f` to that effect kind's failure
  schema.
- **Relationship to the terminal family.** `TerminalFailed` *is* the base. The
  terminal-union `Failed` tag carries the base; at a site where the effect kind is
  known (always, today) it narrows to that kind's schema. No new terminal tag.
- **Rejected for v1: a runtime-caseable `EffectError` union with a kind
  discriminant.** It has no v1 use case (no statically-unknown-kind site) and would
  be speculative machinery. It is the natural *future* extension if a
  "store-a-failure-and-handle-it-generically" pattern ever arises; the family
  framing leaves room for it (a kind discriminant becomes the tag, runtime `case`
  becomes available) without rework.

## Decision 2 — The common base (the only thing built now)

The base is **`{reason, summary, effect_id, run_id}`** — minimal, already exposed,
already redaction-safe. The single time-sensitive action is making it **consistent
on the wire across all effect kinds**:

- **Additively** ensure every effect failure fact carries the base with consistent
  names. Concretely: `exec` failures gain `reason` (the human-facing field, matching
  coerce/workflow); they **keep** `message`/`exit_code`/`stderr` as untyped extras
  (no information is discarded — those are the raw material for a future
  `ExecFailure` variant).
- Migrate our own fixtures/goldens reading `f.message` → `f.reason` (the only thing
  the rename touches; we own all of it).
- Keep the base **minimal** to minimize premature commitment; any richer common
  field (a structured error code, etc.) is a *variant* concern, not the base.

This is independently good (telemetry, auto-fail, diagnostics all benefit from a
consistent failure base) and is the foundation honest base-typing sits on.

## Decision 3 — `fails as f` types to the base; extras are demand-driven and additive

- v1 replaces the `"fails" => {}` no-op: `after <effect> fails as f` types `f` to the
  common base (`TerminalFailed`). With Decision 2, that type is *honest* (the base
  fields are really present on every failure fact).
- **Per-kind extras (e.g. `ExecFailure{exit_code, stderr}`) are NOT built in v1.**
  They are added demand-driven, one effect kind at a time, as the kind's failure
  schema extends the base. Because access requires the per-kind static narrowing,
  each addition is additive and cannot break an existing base read.
- **Each variant's exposed field set is an IFC/redaction decision** (Decision 4),
  so a variant lands only with IFC-track review, not unilaterally.

## Decision 4 — Variant field sets are an IFC redaction co-design

Failure payloads carry the most sensitive incidental detail in the system (stderr,
provider control-plane errors, paths, traces) and there is already a redaction
boundary there (the codex error-surfacing work). A typed `f.stderr` is a **sanctioned
read channel** for whatever that field contains. Therefore: **each `EffectError`
variant's field set is, in part, a confidentiality decision** about what failure
detail whip code may observe, and is co-designed with the IFC track — never added as
"just more fields." The base is exempt (`reason`/`summary` are already
redaction-safe); the *extras* are where this bites, which is another reason they are
deferred behind narrowing.

## Non-goals / deferred

- **Generics / a generic `Result(T, E)`.** Excluded by language design; this whole
  record is the no-generics translation.
- **Runtime-caseable `EffectError` union + kind tag.** No v1 site needs it; future
  extension only.
- **Per-effect variant catalogs.** Demand-driven, additive, IFC-reviewed (Decision
  3/4). v1 ships only the base.
- **Cross-revision failure-shape checking.** Same disposition as DR-0029
  boundary-contract work (a non-issue in the single-bundle model).

## Formal-model plan (model-first, per the per-piece gate)

`models/maude/effect-error.maude` (ASCII comments; `RESIDUAL:Cfg` soup var on
No-solution targets so bites are non-vacuous; no leading-paren comments):

- **base-present:** every effect kind's failure carries the base fields — a payload
  missing a base field is rejected (the wire-consistency invariant).
- **static-narrow:** `after <effect> fails as f` resolves `f` to that kind's schema
  (base now; base+extras once a variant exists) — the kind is taken from the effect,
  not a runtime tag.
- **extras-behind-narrowing (the additivity invariant, the load-bearing bite):**
  reading a per-kind extra is possible **only** under the matching effect-kind
  narrowing; an un-narrowed (base) read of an extra is rejected — proving that adding
  a variant cannot retroactively change base-read semantics (non-breaking by
  construction).
- **observer-only:** a rule that tries to *construct* an `EffectError` is rejected
  (§5.4 generalized).

Lean: reuse `Whipple/Narrowing.lean` (static narrowing is the same payload-access
judgment); add the **base ⊆ every variant** lemma (a base read is well-typed in
every narrowed scope), which is what makes the un-narrowed base read total.

## Sequencing (steps map to the prior discussion's 1/2/3)

Model-first, each piece through the per-piece review gate:

1. **This ADR** (the design + deferred scope). ← first.
2. **Formal models** (`effect-error.maude` + the Lean lemma), wired into
   `scripts/check-formal-models.sh` / `check-lean-models.sh`.
3. **Wire base-unification (step 1/2 of the prior framing):** additive base on every
   effect failure fact + fixture/golden migration. The breaking-now-free move; can
   land early since it is independently good.
4. **Base typing (step 3):** `fails as f` → the base; delete the `"fails" => {}`
   no-op; corpus migration of `f.message` → `f.reason`.
5. **Docs** (language-reference failure section) + per-piece gate green.

Deferred slices (each its own future gate, demand-driven): per-effect variant field
sets (with IFC review), and — only if a statically-unknown-kind failure value ever
arises — the runtime-caseable `EffectError` union.

## Resolved questions (Jack, 2026-06-29)

1. **Base field set — RESOLVED: include `kind: string` in the base now.** Base is
   `{reason, summary, effect_id, run_id, kind}`. `kind` is harmless future-proofing
   (forward-compatible with a future runtime union; useful for telemetry) even
   though static narrowing does not require it. Nothing else is promoted to the base.
2. **First variant — RESOLVED: base-only v1 + a *modeled* stub variant.** v1 ships
   strictly base-only (no real per-effect extras). The extras-behind-narrowing path
   is exercised in the **formal model** by a stub variant (a kind with one extra
   field) so the additive path is *modeled* before it is *built*. The first real
   variant lands later, demand-driven, with IFC review (Decision 4).
3. **Static-narrow-only for v1 — BLESSED.** No runtime `EffectError` union / kind
   tag in v1 (no site has a statically-unknown failing-effect kind); future
   extension only.
