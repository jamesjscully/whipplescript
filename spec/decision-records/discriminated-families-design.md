# Discriminated Families in whipplescript — A Unified Design

**Status: DRAFT (research thread).** Decision-record style; decisive where groundings disagree, with assumptions stated inline and collected at the end.

> **Revision 2026-06-28 — anchor verification + Stage 3 reframe.** All code anchors were verified against the tree. Core factual claims confirmed TRUE: the latent `schema_for_ref` sum-type bug (coerce_native.rs:154-156), `after … fails as f` not binding a schema (lib.rs:6504 `fails => {}`), and the genuine 3-passes-plus-textual-scan duplication (dispatched at lib.rs:6524-6533). Drifted anchors corrected inline: `schema_for_ref`/`schema_ref_is_class`/`output_schema_envelope` live in `crates/whipplescript-kernel/src/coerce_native.rs` (not lib.rs); the IFC low-integrity sources are at ifc.rs:661-677 (not :619-628); fixture generators at main.rs:19430/19484. **Stage 3's "strict vs lenient admission" — flagged as the most contestable call — was reframed (§5.7):** strict admission is already the system-wide invariant (event-ingress.md:128; no lenient path exists), and narrowing soundness needs only *positive* conditional-presence, not rejection of inapplicable sibling fields. This makes Stage 3's net-new admission work small and removes a hard corner.

> **Revision 2026-06-28c — implementation findings (Stages 1a/2 in progress).** Landed + tested on `infoflow-design`: the **arm-after-wildcard redundancy** check (inv c second half; shared by rule + terminal coverage validators), the **Stage 2 `schema_for_ref` fix** (payload enums now emit `anyOf` of variant object schemas, not a string enum), and the model-first layer (`case-family`/`case-selector`/`discriminant-schema` Maude + `Narrowing`/`Refinement` Lean) wired into both gates. **§5.2 CORRECTION (design was wrong):** "narrow `after … fails as f` to a single `TerminalFailed`" would *regress* `exec … fails as f { f.message }` — failure payloads are **effect-kind-specific** (`exec` failure exposes `.message`; `coerce`/workflow failure exposes `.reason`; only the terminal-*case* `Failed` tag uses `TerminalFailed`). The current `"fails" => {}` (alias left untyped, field access unchecked) is the deliberate, correct conservative choice; a real fix needs **per-effect-kind failure schemas**, not one `TerminalFailed`. §5.2 below is superseded by this note and deferred pending that design. **§5.3 fixture-anchor CORRECTION:** coerce fixtures are NOT produced by `ingest_shape_json`/`fixture_value_for_shape` (that path is exec/parsing-effect JSON ingestion). Coerce outputs are injected by author-written **`stub coerce <fn> returns { … }`** clauses, evaluated by the kernel (main.rs ~10951–11488). So the remaining Stage 2 runtime piece is "can a `stub … returns { … }` block construct a sum-type enum variant value", not a shape-generator change. Verified that coerce→enum already **checks** end-to-end (examples/coerce-enum.whip, gated) with the schema fix; the runtime-stub piece is the open remainder. **Family B status (2026-06-28): foundation + static validation + admission DONE** (parse `<f> <T> when <disc> is "<lit>"` — `is` not `==`, tokenizer has no `==`; validate unknown-disc/literal-not-in-union/non-literal-disc; conditional required-presence at admission, positive-only). **Read-narrowing (restrict conditioned reads to their `case` arm) DONE** (7ca4df3): a first attempt that added the check to the shared line-based field-path validator was reverted (the general per-line body loop double-validated arm interiors with an empty allowed-set → false-positives); the landed version is a **dedicated AST pass** `validate_conditioned_field_reads` that walks the body AST with an allowed-set, extends it per `case <root>.<disc>` arm (`family_b_arm_allowed` over `SchemaIndex.presence`), and rejects conditioned reads in record/terminal/done values + branch conditions + case guards that are outside a matching arm. Tested outside/matching/wrong-arm. **So Family B (Stage 3) is COMPLETE end-to-end** (parse + validate + admission + read-narrowing). v1 gap: effect prompt/argument read positions (`BodyStmt::Effect` skipped) — documented follow-up.

> **Revision 2026-06-28b — IFC aligned to DR-0029/0030 + signed off.** §5.6 rewritten and §5.9 added: narrowing is **label-transparent** (granularity-agnostic, never lowers a label — forced by DR-0030 Decision 0's no-oracle dial), forward-compatible with the join-box refinement (DR-0030 Direction B per-field labels; Family A coerce-enum results stay single-label *permanently* as a decided non-goal). The selection channel splits: ordinary-effect arms are the pre-existing I-IFC5 control-flow scope (same as a `where` guard) **with a v1 divergent-sink lint**; crossing-in-arm cases are governed by **NMIF-on-the-selector** (DR-0030 A.4.3 — the discriminant is the selector), vacuous until crossings land. Zero new IFC syntax. Jack signed off on: label-transparent framing, the selector doctrine, and lint-in-v1.

> **Revision 2026-06-28d — implementation status.** SHIPPED on `infoflow-design` (per-piece, all gates green): **Stage 2 / Family A** end-to-end (coerce→enum: anyOf schema fix + check + runtime variant-dispatch + gated example); **Stage 3 / Family B** end-to-end (`when <disc> is "<lit>"` parse + static validation + conditional-presence admission + read-narrowing via a dedicated AST pass); **Stage 1a** user-facing checks (arm-after-wildcard redundancy; conflicting-effect-binding reuse §5.5); **Stage 1b** the binding-syntax break (`Tag as binding` required for terminal cases, space form removed, corpus/docs/goldens migrated); the model-first layer (`case-family`/`case-selector`/`discriminant-schema` Maude + `Narrowing`/`Refinement` Lean) gated. Surface note: Family B chose `when <disc> is "<lit>"` (`is`, not `==` — the declaration tokenizer has no `==`). REMAINING: Stage 1a pass-collapse (internal, no behavior change — deferred as low-value); Stage 4 / Family C (deep cross-instance runtime); the selector-doctrine IFC wiring (Task 4); a `whip fmt --upgrade-as-bindings` helper.

> **Revision 2026-06-28e — reflection-phase decisions (Jack).** After the implementation pass, two open calls resolved: (1) **Family C milestone runtime = poll** (parent invoke effect polls child milestones each step; push-via-notify rejected for v1 — see §7.3). (2) **Pass-collapse (Stage 1a 4-pass unification) is DEFERRED / dropped from the active tracker** — Families A and B shipped without it, so it buys no capability, only internal tidiness, at regression risk to working core machinery; revisit only if the duplication actually bites. Both remaining features (Family C, selector wiring §7.3/§7.4) now have grounded, de-risked designs ready to implement on the next pass.

> **Revision 2026-06-29 — Family C SHIPPED end-to-end.** Stage 4 landed on `infoflow-design` (all gates green). Surface: child `emit milestone "<name>" of <Class> { fields }` + parent `after p reaches "<name>" as m`. **Two scope-controlling design decisions vs the original §7.3 sketch:** (1) **milestone emission is a `BodyStmt::Milestone`, NOT a new `IrEffectKind`** — it is a *synchronous durable projection* (record-like), so the feared ~15-20-site `IrEffectKind` cascade never happened; it derives a `workflow.milestone:<name>` fact in the child's own base. (2) **the milestone is declared by emitting it** (`of <Class>` types the payload; the declared set is the union of names a workflow's rules emit, collected into `WorkflowInputSurface.milestones`) — no separate top-level `milestone` declaration construct, so **zero new `Item` variant and zero `.ir` golden churn** (body text + runtime-derived facts; the IR `DependencyPredicate` for `reaches` maps to `Completes`, provenance-only — real gating is text-keyed at runtime via `fact_matches_after_predicate("reaches:<name>")`). `AfterPredicate::Reaches` stays fieldless/`Copy` (name on `AfterBlock.milestone`, as prototyped). Delivery: the parent invoke effect polls the child's `workflow.milestone:*` facts after stepping it and re-derives idempotent `workflow.invoke.reached:<name>` facts keyed by the parent effect id; a sibling `after p reaches` reacts through the *unchanged* text-based after-machinery, binding `m` to the milestone payload. Static checks: **reject-undeclared** (a parent cannot observe a milestone the child never projects — the terminal-only observation invariant) + unknown-payload-class. Verified: parent observes BOTH a child milestone (binding `m.field`) AND the terminal. Acceptance target `revision-parent-child.whip` extended. Gates: parser 215 (+3 milestone unit tests), control_plane 155 (+1 e2e `dev_observes_child_milestone_via_reaches`), bin 284, soft_middle 52, `milestone-signal.maude` (2 Sol / 3 No-sol bites), fmt clean, docs-examples green. **v1 surface note:** `m`'s field reads are as-strict-as invoke-result reads (loose — consistent with how `after child succeeds as r` types `r`); tighter per-field milestone typing is Open Question 3. **The whole discriminated-families tracker (Stages 1a/1b/2/3/4 + selector capstone) is now SHIPPED.**

## Executive summary

whipplescript already implements one type-theoretic move — *match a tag, recover the payload type for that tag, narrow per-arm, check exhaustiveness, allow `_`, allow guards* — across four surface spellings and four compiler paths (three parallel `case` passes plus a textual `after … as` scanner that never enters the case machinery). This document names that idea once as a **discriminated family**, makes narrowing a single parameterized primitive, and re-expresses every existing spelling as an instance, then opens new families (LLM-result enums, discriminant-string schemas, child-milestone lifecycle). The payoff is a single headline guarantee — **Total Outcome Settlement**: every outcome of an LLM decision, effect, or external event provably reaches a terminal — assembled from exhaustiveness (lifted to a hard error), flow-liveness (warning), and kernel auto-fail (runtime backstop). The work is model-first and greenfield (no back-compat): Maude must-bite obligations and hermetic Lean 4 proofs precede each stage. The design is honest that it unifies *elimination* (case/narrow/exhaustiveness), not *production* (tag resolution remains a four-way dispatch behind one trait), and that two stages (discriminant-string refinement, child lifecycle) are genuinely new type-system work rather than consolidation.

---

## 1. Motivation

### 1.1 One idea, four spellings, four code paths

The same elimination is spelled four ways today:

```whip
// (a) user enum — binds with `as` (spec/sum-types.md is normative)
case outcome {
  Approved as a => complete with a.score
  Rejected as r => log r.reason
  Blocked       => escalate
}

// (b) terminal union — binds WITHOUT `as` (space-separated; spec/sum-types.md §2.1)
case answer {
  Completed decided => decide_with decided.choice
  Failed failure    => log failure.reason
  TimedOut          => retry
  Cancelled         => abort
}

// (c) terminal via `after` — binary, `as`, textual scan (lib.rs:6462-6510)
after child succeeds as r { complete with r.value }
after child fails    as f { log f.reason }          // f currently does NOT narrow

// (d) coerce result — no binding, in-place
after classification succeeds { use classification }
```

These are backed by **three parallel case passes** (`validate_case_blocks` at lib.rs:9366, `collect_rule_case_metadata`, `collect_terminal_case_metadata` at lib.rs:7045-7115) plus a **fourth textual narrowing path** (`after … as`, lib.rs:6462-6510) that never enters the case machinery. Tag resolution forks five ways; payload-schema lookup forks (`case_branch_payload_binding` at lib.rs:7011 vs `terminal_payload_schema_for_tag` at lib.rs:7225); the exhaustiveness domain forks (`finite_case_domain` at lib.rs:9915 vs the hardcoded `terminal_case_tags()` at lib.rs:9734/9777, duplicated again in `validate_terminal_case_pattern` and `validate_terminal_case_coverage`); guard validation is split because only pass 3 holds `effect_payload_types`.

### 1.2 What unification buys

Beyond shrinking the formal model from four cases to one parameterized primitive, unification *enables* features that are blocked today:

- **LSP on case bindings.** `IrCasePattern::EnumVariant(String)` has no binding field (binding is recovered out-of-band by `case_branch_payload_binding`); `OptionalSome` *does*. A unified `{ tag, binding: Option<Ident>, guard: Option<Expr> }` puts the binding in the IR, unlocking hover/completion/rename.
- **The two dogfooding bug classes become unrepresentable.** *"failed invoke hung the parent"* (the `succeeds` predicate matched the terminal marker regardless of status) and *"on-timeout fired on success"* both reduce to "distinct tags with distinct payloads; an arm fires only for its own tag." See §4.

### 1.3 The headline guarantee

> **Total Outcome Settlement.** In a *self-terminating* flow, every tag of every discriminated family is either explicitly handled by an arm that provably settles (reaches a terminal or hands off a fact), or driven to a terminal by kernel auto-fail. No outcome of a decision, effect, or external event can leave an instance stalled.

This is the product of three layers that already exist but were never composed (exhaustiveness, flow-liveness, auto-fail). §4 states the composition honestly — one hard error, one warning, one runtime backstop — and bounds the claim to self-terminating flows.

---

## 2. The Core

### 2.1 Definition

A **discriminated family** `F` is:

```
F = (Tags, payload, Discriminant, origin)

  Tags          : finite, statically-known set of tag names      { t₁, …, tₙ }
  payload       : Tags → Schema⊥        each tag → its payload schema, or ⊥ (bare tag, no payload)
  Discriminant  : how a runtime value reveals its tag            (§2.4)
  origin        : producer | observer   may user code CONSTRUCT a value of F, or only ELIMINATE one?
```

`Schema⊥`: a tag carries either exactly one flat payload record (a class/schema) or nothing (a bare tag, e.g. `TimedOut`, enum `Blocked`). This respects no-nested-sum-payloads (spec/sum-types.md): a payload is a flat record, **never another family** — exhaustiveness stays decidable over a finite tag set.

`origin` is load-bearing for the kernel boundary and IFC: a user enum is `producer` (rules build `Approved {score: 0.9}`); the terminal and lifecycle families are `observer`-only — the kernel produces them, user rules may only eliminate them (enforced; see §5.4).

### 2.2 The single elimination form

```
case <scrutinee> {
  <tag> as <binding>  where <guard>  => <body>     // payload-carrying arm
  <tag>               where <guard>  => <body>     // bare arm
  <tag> as _                         => <body>     // discard a named payload
  _                                  => <body>     // fallthrough
}
```

`<scrutinee>` is an expression whose static type resolves to some family `F`. Everything below is family-agnostic; the four `Discriminant` cases differ only in *how `F` and the runtime tag are recovered* (§2.4).

### 2.3 The ONE narrowing rule

With scrutinee family `F = (Tags, payload, _, _)` and ambient scope `Γ`:

```
NARROW (payload arm  tᵢ as b where g => body):
  ── F.payload(tᵢ) = S   (S ≠ ⊥)
  Γ' = Γ[ b ↦ S ]                 -- b has the payload schema for THIS tag only
  Γ' ⊢ g : Bool                   -- guard checked in the narrowed scope
  Γ' ⊢ body                       -- body checked in the narrowed scope

BARE  (tᵢ where g => body, or F.payload(tᵢ) = ⊥):  Γ' = Γ
WILD  (_ => body):                                  Γ' = Γ
```

This is exactly what `branch_scope` already does (lib.rs:6831-6862, 7045-7115): clone `binding_types`, insert `(binding, schema)`, validate guard + field paths in the clone. Today it is spelled three times with three payload lookups; unification replaces them with one per-family `F.payload`, and leaves **exactly one `branch_scope` insertion site**.

**Guard scope for terminals (resolves blocker, lens 1).** Terminal-case guards reference payload bindings (`Completed as r where r.x > 0`). In the unified pass, guard validation is part of NARROW: the guard is checked in `Γ'`, which already contains `b ↦ S`. For terminal families, `S` for the `Completed` tag is sourced from `effect_payload_types[scrutinee]`; for `Failed/TimedOut/Cancelled` it is the fixed terminal schema. `effect_payload_types` is therefore **a precondition input to NARROW, not a separate pass** — it must be populated before the unified pass runs (§6.1, sequencing). It is re-derived per scrutinee from the effect map; it is never mutated by NARROW. There is one `branch_scope` for both enum and terminal arms; the only difference is the source of `S`, which is exactly what `F.payload` abstracts.

**Payload-access safety (the teeth).** `b` is in scope only inside arm `tᵢ`. Reading a payload field outside its matched arm is a check error — the existing sum-types.md rule ("reading `outcome.score` outside `case` is a check error") generalized to all families, enforced by `validate_known_field_paths` against `Γ'`, unchanged.

### 2.4 The only parameterized thing: `Discriminant`

The four mechanisms are *not* four narrowing rules — they are four ways to answer "what is `F`, and what is the runtime tag?" behind one trait:

```rust
trait Family {
    fn tags(&self) -> &[TagName];                        // exhaustiveness domain
    fn payload(&self, t: &TagName) -> Option<SchemaRef>; // NARROW's F.payload
    fn discriminant(&self) -> Discriminant;              // codegen + IFC
    fn origin(&self) -> Origin;                          // producer | observer (§5.4)
}

enum Discriminant {
    SynthesizedField(&'static str), // user enum: reserved `variant` field (lib.rs:5286)
    ImplicitKernel,                 // terminal / lifecycle: tag is a kernel projection, not a value field
    UserField(FieldName),           // discriminant-string schema (Family B)
    PredicateSugar,                 // `after … succeeds/fails as` desugars to ImplicitKernel (§3.2)
}
```

`finite_case_domain` → `F.tags()`. `case_branch_payload_binding` + `terminal_payload_schema_for_tag` → `F.payload`. `terminal_case_tags()` and its three duplicates → the terminal family's `tags()`, **one definition**.

**Honest scope of the abstraction (resolves major, lens 3).** This unifies *elimination* — case matching + narrowing + exhaustiveness — not *production*. `Discriminant` still has four variants; tag resolution inherently has four paths. We call this **"unified narrowing with parameterized discriminant dispatch,"** not full unification. The benefit is real (one NARROW rule, one `branch_scope`, one tag domain, one payload lookup, binding visible in IR); the boundary is explicit so future changes know where the seams are.

**Flow-namespace guard (resolves major, lens 1).** Any `ImplicitKernel` discriminant **must not** derive from a flow-owned fact (`FlowAwait_*`). This is documented in the Maude flow-namespace model, asserted in the `Family` trait contract (doc comment + a compiler assertion in family construction), and is why self-typestate is out of scope (§7).

### 2.5 Exhaustiveness, `_`, guards, redundancy — stated precisely

Given arms `A` over family `F`:

- **Covered tags** = `{ t | t appears in an *unguarded* arm of A }`. A guarded arm (`where g`) does **not** count — a guard can fail at runtime, so the tag is not provably handled. (Current code already filters guarded arms at lib.rs:9866; we pin it formally.)
- **Exhaustive** iff `Covered ⊇ F.tags()` **or** `A` contains an unguarded `_`.
- A non-exhaustive `case` is a **hard error** (error severity, not lint). Diagnostic must name the conditional case explicitly: *"`Completed` is handled only under a guard; exhaustiveness requires an unguarded arm or a `_` fallback."*
- **`_`** is the catch-all and binds nothing (there is no single payload across remaining tags). `Tag as _` discards a single named tag's payload.
- **Redundancy** (an unguarded arm for a tag already covered; any arm after an unguarded `_`) is a **hard error**. New, cheap, prevents dead arms. Implemented in the unified validator as a left-to-right reachability scan over arms (covered-set + post-`_` flag).

---

## 3. Surface Syntax Decision

**One binding syntax everywhere: `Tag as binding`; bare `Tag` for payload-less arms; `_` for fallthrough; `as _` to discard; `where` for guards.**

```whip
case x {
  Tag1 as b  where b.f > 0  => …
  Tag2                      => …    // bare
  Tag3 as _                => …    // discard payload
  _                        => …    // fallthrough
}
```

Rationale (greenfield, no back-compat):

1. **Consistency.** Every other binding in whip uses `as` (`after X as y`, `tell worker as turn`, `invoke W {…} as result`, `coerce f() -> T … succeeds as r`). The terminal space-separated form (`Completed result`) is the odd one out; aligning makes binding sites greppable and IDE-resolvable.
2. **The IR can carry the binding** (§1.2), unlocking LSP.
3. **Spec already mandates `as`** for the largest family (user enums); only the terminal spec prescribes space.

**Cost — acknowledged as a migration lift, not cosmetic (resolves major, lens 3).** The terminal `Completed result` spelling is removed; spec/sum-types.md §2.1 and language.md §B.2.2 must be updated; the in-repo corpus (scheduled-escalation.whip, terminal-output-union.whip, revision-parent-child.whip, et al.) migrates. We ship a real migration tool, **`whip fmt --upgrade-as-bindings`**, not a hand-wave "mechanical rename." The grammar change is unambiguous: in a terminal/enum case head, `Ident Ident` (tag + bare binding) rewrites to `Ident as Ident`; `Ident` alone and `Ident as Ident` are left untouched, so `Completed result` and an already-`as` form never collide. `Completed result where …` rewrites to `Completed as result where …`. The migration is applied, the old space-form parser path is deleted, and a grep proves single-path.

`when` (rule trigger, prefix on a rule) and `where` (guard, infix in a case head) stay distinct — disjoint grammatical positions. Family B reuses `when` as a postfix field annotation (§5.2), also a disjoint position.

---

## 4. The Headline Guarantee — composition, stated honestly

> **Total Outcome Settlement (self-terminating flows).** Defense in depth, three layers:
> 1. **Exhaustiveness** — type-level, **error severity, always enforced**: every tag has an arm or a `_` (§2.5).
> 2. **Flow-liveness** — semantic-level, **warning severity**: each handled arm provably settles (`check_flow_branches`, flow_expand.rs:622-670). Can have false positives on legitimately non-self-terminating flows, hence advisory.
> 3. **Auto-fail** — **runtime backstop** (flow-autofail.maude): in a self-terminating flow, an unhandled `Failed`/`TimedOut` drives `fail_instance_internal` rather than stalling.

**Honest restatement (resolves minor, lens 3 + missing-consideration, lens 2).** The *static* guarantee is exactly exhaustiveness: every tag has an arm. The *settlement* guarantee is runtime (auto-fail). Liveness is the advisory middle layer. We do **not** claim static totality where only runtime fallback exists. The claim is scoped to self-terminating flows; for non-self-terminating flows liveness is informational only.

"Unhandled" for auto-fail (resolves missing-consideration, lens 2) means: no matching rule will ever fire on the terminal/failure fact. The flow-liveness checker determines this by the same rule-reachability it already uses for branch settlement — a failure with a downstream dependency rule that consumes its fact is *handled*; one with no consumer and no on-fails handler is *unhandled* and routes to auto-fail. The Maude composition obligation (§6) must show exhaustiveness + auto-fail jointly settle every tag.

This is the direct fix for the two bug classes:
- *failed invoke hung the parent* — `after child succeeds as r` desugars to a case whose `Completed` arm binds `r`; `Failed` is a **distinct arm** that exhaustiveness forces to be handled (or `_`, or auto-fail). The hang is unrepresentable.
- *on-timeout fired on success* — `TimedOut` and `Completed` are distinct tags; an arm fires only for its own tag.

---

## 5. Soundness & IFC

### 5.1 Boundary order for `after`-desugar (resolves blocker, lens 1)

The textual after-alias scan (lib.rs:6462-6510) today binds aliases into `binding_types` *before* `validate_case_blocks`, so they're in scope for guard/body validation. We **desugar `after X <pred> as b` to `case X { Tag as b => … _ => … }` during AST lowering, before the unified case pass**, and the desugared `case` produces the binding through the *same* `F.payload` path the case machinery uses. Concretely: desugaring runs first; it emits an `IrCasePattern { tag: Completed, binding: Some(b), guard }`; the unified pass then binds `b` via `F.payload(Completed)` (= `effect_payload_types[X]`). The alias no longer flows through a separate textual mechanism — it flows through `effect_payload_types` exactly like an inline `case`. The textual scanner is deleted; a grep proves no second narrowing path remains. **Span safety (resolves minor, lens 1):** the desugared pattern reuses the source span of the original `after … as` text via the existing `locate_span` clamp, so diagnostics point at the author's `after` line, not a synthetic location (regression test required).

### 5.2 `after … fails as f` now narrows (deliberate, resolves major, lens 2)

Today lib.rs:6504 returns no schema for `fails` (documented limitation). Under the family, `Failed` has payload `TerminalFailed { reason, summary, effect_id, run_id }`, so `f.reason` becomes legal. This is an **improvement, not a regression**: rules that bound `f` and never read it are unaffected; rules that couldn't read `f.reason` now can. No silent breakage. Reviewers confirm `TerminalFailed`'s field set is the intended exposed surface (hard corner, §7). This also closes the `f.message`/`f.reason` confusion in memory.

### 5.3 Coerce/decide failure vs case-exhaustiveness (resolves blocker design, lens 2)

For `coerce f() -> Triage` where `Triage` is a user enum, **the enclosing terminal `after` carries the failure, not the inner `case`.** Layering:
- Enum-exhaustiveness over `Triage` is a **success-path** property: SAP (schema-aligned parsing) guarantees the success payload is one of the declared variants, so the inner `case c { … }` need not have a `_` for "model returned garbage."
- Coerce **failure** (no variant matched / provider error) is the **terminal family's `Failed` tag**, handled by the enclosing `after … fails` (or auto-fail).

So the success/failure split lands on two different families at two different scopes — which is exactly right.

**Coerce → enum return-type resolution (resolves blocker, both lenses; anchors verified against tree 2026-06-28).** When `coerce f() -> EnumWithPayloads`, the compiler **SHALL** resolve the return type to the **union of the generated `<Enum>.<Variant>` class types**, not the bare string enum. Today `schema_for_ref` (`crates/whipplescript-kernel/src/coerce_native.rs:151-166`, the bug is at :154-156) emits `{type:"string", enum:[names]}` for *all* enums (verified TRUE), which is correct only for bare enums and wrong for payload-carrying sum types (the coerce backend then receives a string-only schema and cannot construct payloads). Required changes:
- `terminal_completed_payload_type` (parser lib.rs:6789-6820): if the enum has generated classes, build `IrType::Union` over `IrType::Ref` to those classes.
- `schema_for_ref` (coerce_native.rs:151): for a sum-type enum, emit `{ anyOf: [ {type:object, properties:{variant:"Approved", score:…}}, … ] }` over the generated classes; for a **bare** enum, keep `{type:"string", enum:[…]}`. Note the per-type strict-schema builder `json_schema_for_type` (coerce_native.rs:79-186) already emits `additionalProperties:false` for objects and `const`/`enum` for literal/union types — the `anyOf` branch slots into that existing strict machinery.
- `schema_ref_is_class` (coerce_native.rs:219): return true for enum refs with generated classes, so `output_schema_envelope` (coerce_native.rs:199) does **not** double-wrap an already-object-shaped union (bare-enum wrapping stays, documented).
- Fixture generator (`ingest_shape_json`, main.rs:19430 / `fixture_value_for_shape`, main.rs:19484): for a sum-type enum, emit a full object `{ variant:"Approved", score:0.5, … }`, honoring `--variant <name>` (default: first declared variant). A golden-fixture test confirms `--variant Approved` → the Approved object and the generated schema matches the backend's expectation.

### 5.4 Observer-only construction (resolves blocker, lens 1)

`origin = observer` families (terminal, lifecycle) **may not be constructed by user rules**. We store a producer-context flag during statement lowering and reject construction of `TerminalFailed`/`TerminalTimedOut`/`TerminalCancelled` (and lifecycle schemas) in user-rule contexts with a clear diagnostic. Test: a rule that tries `record TerminalFailed {…}` is rejected.

### 5.5 Duplicate binding names within a rule scope (resolves blocker, both lenses)

`effect_payload_types` is keyed by binding name; nested `after`/`case` could collide (`after f1 succeeds as r {…} after f2 succeeds as r {…}`), making the keying ambiguous. We add **`duplicate_binding_in_scope`**: collect all binding names in a rule scope (effect/`after` aliases, `case` bindings, rule `when` bindings) and reject duplicates **before** any use of `binding_types` as a map key. Test: nested `after` blocks with identical binding names are rejected.

### 5.6 IFC: narrowing is label-transparent (resolves blocker + major, lens 1; REWRITTEN 2026-06-28 for DR-0029/0030 alignment, signed off by Jack)

Grounded in the IFC system as it stands (Waves 0–6) **and aligned with its decided future** (DR-0029 cross-package, DR-0030 join-box refinement). Today's relevant facts: labels are **resource-grain** (per-field labels deferred — information-flow-surface.md:241); the checker keys on **(source-resource → sink-resource) pairs at the sink** (`injects` ifc.rs:357, `leaks` ifc.rs:335); `message from <channel>` (ifc.rs:668) and `human answered` (ifc.rs:675) are low-integrity sources; `endorse`/`declassify` are audited governance grants over `(resource, role)`, **not user-writable in `.whip` yet** (E4 deferred); and there is **no PC-label / implicit-flow tracking by deliberate scope** (DR-0027:172-174 — control-flow channels are excluded from I-IFC5).

**The decision: narrowing is LABEL-TRANSPARENT (not label-*preserving* in a resource-fixed sense).** It moves no data across a resource boundary, creates no source→sink pair, strips no label (carriage, W6), and — critically — **never lowers a label**. It propagates whatever label the projected value already carries *at the prevailing granularity*: the opaque-join-box label today (DR-0029 X2 v1), a per-field/per-variant label under DR-0030 Direction B. `case` contributes no granularity and no crossing of its own. (Stating it granularity-agnostically now is the DR-0030 "keyed from day one so v2 is non-breaking" discipline: when the join box refines, narrowing already does the right thing with zero change.)

Why transparency is *doctrine*, not convenience: an automatic, repeatable label-*lowering* on `case` would be exactly the **oracle** DR-0030 Decision 0 forbids (the "automatic-and-repeatable" half of the one dial). So "narrowing never lowers a label" is forced. The two channels:

1. **Value channel (data-flow) — protected automatically.** For `case x { tᵢ as b => … }`, the binding `b` and the narrowed payload carry exactly `x`'s label (whatever its granularity). Writing a narrowed low-integrity value into a higher-integrity sink fires the *existing* sink-keyed `injects` and demands an audited crossing — narrowing is invisible to this check, so it **cannot launder a value**. *Admission is not endorsement:* Stage-3 strict admission validates **shape, not trust** — a well-formed payload from an untrusted channel keeps its untrusted resource label through admission (carriage strips nothing). Treating "we validated it" as "it's clean" is the bug class IFC exists to prevent.
2. **Selection channel (control-flow) — splits by what the arm does.**
   - *Arm → ordinary governed effect* (record/emit/… with trusted payloads): a pure control-flow channel — the pre-existing, deliberate I-IFC5 out-of-scope boundary, **identical to a `where` guard on the same discriminant, on both axes** (a secret discriminant driving divergent public writes is the confidentiality dual). Not new to discriminated families. **A v1 lint (warning, never error, no soundness claim)** flags when a low-integrity/secret discriminant selects among arms with *divergent governed-sink effects*, as authoring awareness. *(Jack: lint IN v1.)*
   - *Arm → a crossing* (`declassify`/`endorse` in the arm body, once E4/DR-0030 Direction C land): **the discriminant IS the selector, so NMIF-on-the-selector applies (DR-0030 A.4 property 3 / Direction C).** A crossing selected by an attacker-influenced discriminant is **rejected** unless the discriminant's integrity acts-for the crossing's release authority. This is the existing robust-declassification doctrine (NMIF.lean `untrusted_declassify_only_public`) with "selector = the `case` discriminant" identified — not new policy. **Vacuous today** (no in-arm crossings exist yet); stated now so discriminated families compose correctly the *moment* crossings exist (the DR-0030 "model the risky interaction before lock-in" move).
3. **`case` is the mandated discipline, not an endorsement.** You cannot read a conditioned/narrowed field without first matching the tag, so `case` makes the low-integrity dependency syntactically visible at every access. **Narrowing is a label no-op *and* a visibility control point** — coupled, not separable. Auto-endorsing on `case` would silently launder attacker data into a governed effect; transparency forbids it.

**Zero new IFC syntax** (DR-0030 Direction B principle: IFC syntax lives only at source-labels and crossings). `case`/narrowing is type/scoping machinery; Family B's `when kind == "x"` is a *presence/type* annotation, not a label.

### 5.7 Runtime obligations for string-literal / open-tag discriminants (Family B) — REWRITTEN 2026-06-28 after grounding

The original draft treated "strict vs lenient admission" as the most contestable call in the whole design. **Grounding against the tree dissolves most of that contestability**, and a soundness observation shrinks the new admission obligation to something small. Two findings:

**Finding 1 — strict admission is already the system-wide law, not a new policy.** Every ingress path already rejects extra and missing-required fields against the declared schema: `validate_json_for_object` (main.rs:19394) for signals/notify, `validate_ingest_value` (main.rs:19539) for `exec -> Schema`, and the same validators for table fixtures. Out-of-range discriminant values are *already* rejected today — a `"ok" | "failed"`-typed field with value `"weird"` fails the union-range check (~main.rs:19314-19328). `spec/event-ingress.md:128` makes this a normative promise: *"A malformed payload is rejected before any fact is recorded."* There is **no lenient path anywhere in the codebase** (no `drop_unknown`, `ignore_extra`, `lenient_mode`). So choosing strict for Family B is *consistency with an existing invariant*; choosing lenient would be the precedent-breaking move that violates event-ingress.md. The fork is settled by the status quo.

**Finding 2 — narrowing soundness needs POSITIVE presence only, not negative absence.** This is the key simplification. Family B's static checker *forbids reading a `when`-conditioned field outside its matched arm* (§2.3 payload-access safety, generalized). Therefore the runtime presence-or-absence of a *wrong-arm* field is **statically unobservable** — no well-typed program can read `e.rollbackReason` in the `kind == "deploy"` arm. Soundness of narrowing requires exactly one thing: *in the arm where `kind == "deploy"`, the deploy-conditioned fields are present and correctly typed.* It does **not** require that a deploy event *lack* rollback fields. Consequence: admission must enforce **conditional required-presence** (if the discriminant holds, the conditioned fields are present + typed) — a natural gating of the existing "required field present" check on the discriminant value — and need **not** reject "contradictory" payloads that carry inapplicable sibling fields. This matters in practice: many real webhook providers send *all* keys on every event (with `null` for inapplicable ones); the soundness-minimal rule accepts those, where the draft's "reject contradictory shapes" rule would have rejected every one of them.

**The v1 admission rule for Family B (net-new work, gating the already-strict validator):**
- If discriminant `d` has value `"t"`, then for each field annotated `T when d == "t"`: the field MUST be present and admit as `T` (else reject — this is the existing required-field check, newly gated on `d`).
- Fields annotated for *other* tags (`when d == "u"`, u≠t) are **not** required and **not** rejected if present (a present-but-unreadable sibling is admitted and simply never narrowed into scope). `null`≡absent for the purpose of "is the field present," so all-keys-with-nulls webhooks pass.
- An **out-of-range discriminant value** is rejected — already true today, inherited for free.

The static exhaustiveness domain is exactly the declared literal set; an unknown future tag is a rejected payload, not a silent fallthrough. (Forward-compat with unknown tags is a future opt-in via an explicit open variant — out of scope for v1.)

### 5.8 Formal soundness, honest about what's proved vs runtime-checked

- **Narrowing payload-access soundness** is a *static* property, proved in Lean (§6): a binding `b:S` introduced in arm `tᵢ` is not in the type environment for any arm `tⱼ` (i≠j) or outside the case.
- **Family B field-presence** is a *hybrid*: statically, narrowing on the discriminant yields the conditioned-present field set (proved in Lean as a finite tag→fieldset map, no dependent types); at runtime, the **conditional required-presence admission check** (§5.7) is what makes the static map *true of the actual value*. The static guarantee is exactly: *in the arm `d == "t"`, the `when d == "t"` fields are present and typed.* That holds iff admission rejects a payload that asserts `d == "t"` but omits a `when d == "t"` field. This is a strictly *positive* obligation (right-arm fields present), a gating of the existing required-field check — not a rejection of inapplicable sibling fields (§5.7 Finding 2). The static narrowing is a lie about runtime only if admission fails to enforce conditional required-presence — that single check is the load-bearing one (hard corner, §7).

### 5.9 Alignment with DR-0029 / DR-0030 (the join-box future)

Discriminated families are designed to be *correct under the present resource-grain join box and forward-compatible with its decided refinement* (DR-0029 X2 → DR-0030). The alignment, point by point:

- **Label-transparent ⇒ granularity-agnostic (DR-0030 A.2).** Narrowing reads the prevailing label granularity; it neither assumes nor requires resource-grain. When the join box refines from opaque (X2 v1) to whole-result/per-field signatures (DR-0030 A.2 v1→v2), narrowing carries the finer labels with no change. This mirrors DR-0030 keying the flow contract per-output-port "from day one so v2 is non-breaking."
- **No automatic label-lowering ⇒ no oracle (DR-0030 Decision 0).** `case` is automatic and repeatable, so it must never lower a label; it makes no flow-edge-removal/independence claim, so it is automatically fail-closed and never a serialized "departure" (DR-0030 A.3). Any future "the matched arm rules out variant V" refinement would be a flow-edge removal and would have to go through A.3 (departure, fail-closed to `direct`, proof obligation) — explicitly **not** done here.
- **Family A is fixed by a decided non-goal (DR-0030 Direction B).** A `coerce -> enum` result is a *single model output*; per-field/per-variant labels within a single model output are the **rejected-as-unsound** case (B trusts the model to keep the secret out of a "public" field). So a coerce-enum result carries one turn label permanently, and `Approved as a` gives `a` that same label — future-settled, not merely tolerated.
- **Finer per-variant labels come only from whip assembly (DR-0030 Direction B), and compose.** Where per-variant label precision is genuinely wanted, it is produced by Direction B's commit-then-fill (public phase commits the tag/discriminant, private phase fills the payload), assembled in trusted whip; label-transparent narrowing then reads those per-provenance labels out. Family A/B are the *elimination* form; Direction B is how finer labels get *into* the value. No conflict.
- **The selector doctrine is DR-0030's, instantiated (A.4.3 / Direction C).** "Integrity on the selector" is DR-0030's lever against adaptive/oracle leaks; discriminated families instantiate it by identifying the `case` discriminant as the selector for any in-arm crossing (§5.6 channel 2). Pinning it now, before crossings exist, is the DR-0030 "model the riskiest interaction first" discipline.
- **Cross-package (DR-0029): narrowing is a consumer-side elimination.** A discriminated value that crosses a package boundary is one output port carrying one (join-box, X2 v1) label fixed at the seam; `case` runs consumer-side on a value whose label the boundary already set. Discriminated families require none of the X1–X8 surface machinery — they ride on whatever label the boundary assigned, and stay label-transparent over it (forward-compatible with a future per-output-field signature, DR-0030 A.2 v2).
- **Zero IFC syntax (DR-0030 Direction B).** Confirmed: discriminated families add nothing to the IFC surface (source-labels + crossings only).

---

## 6. Formal-Model Plan

Model-first per stage: Maude must-bite (with a `RESIDUAL:Cfg` soup variable on NoSolution targets, per memory — bare `=>*` is vacuously No-solution and provides no bite) and a Lean obligation under `scripts/check-lean-models.sh` (hermetic, Mathlib-free). Maude comments stay plain ASCII.

### 6.1 Stage 1 — `case-family.maude` + `Narrowing.lean`

**Maude must-bite** (`case-family.maude`):
- (a) reading a payload field of `tⱼ` inside arm `tᵢ` (i≠j) — rejected;
- (b) a case missing tag `tₖ` with no `_` — rejected;
- (c) a redundant arm (duplicate unguarded tag, or any arm after unguarded `_`) — rejected;
- (d) **guarded-arm-non-coverage**: a tag handled *only* under `where`, no unguarded arm, no `_` — still non-exhaustive, rejected;
- plus a **positive** fixture: an exhaustive case (all tags, or a valid `_`) — accepts;
- plus the **terminal-guard** model: guard validation receives branch scope *and* `effect_payload_types` for terminal `Completed`; a nested `after`/`case` with guards over both a terminal and an enum scrutinee validates correctly.

**Lean obligation** (`Whipple/Narrowing.lean`): (a) a per-arm scope judgment defining exactly which bindings are in scope in each `body`; (b) a lemma that `b` from arm `tᵢ` is not in scope for arm `tⱼ` (i≠j) or outside; (c) a theorem that `b.field` is checked against the correct narrowed type; (d) **exhaustiveness ⇒ totality** over the value domain. Mirrors the landed Boundary.lean "constructible only with a proof" pattern; non-dependent (finite tag enum + flat payloads).

**Auto-fail composition** (in `case-family.maude` referencing flow-autofail.maude): a self-terminating flow where exhaustiveness holds but a `Failed`/`TimedOut` arm has no body — auto-fail settles it; a negative fixture where neither exhaustiveness nor auto-fail applies (non-self-terminating, unhandled) — stalls (no settlement), pinning that the guarantee is scoped.

### 6.2 Stage 2 — coerce → enum (extend `case-family.maude`)

Bite: the inner `case` over the enum need not handle effect-failure, but the outer `after` *must* handle `fails` (or auto-fail) — negative fixture where neither inner-exhaustive nor outer-fails settles, ⇒ stall. No new Lean (reuses Stage 1 totality; enum is another finite `tags()`).

### 6.3 Stage 3 — `discriminant-schema.maude` + `Refinement.lean`

**Maude bite:** (a) accessing a `when`-conditioned sibling outside its tag arm — rejected; (b) a `when` literal not in the discriminant's union — rejected; (c) a `when` referencing a non-discriminant field, or a cycle — rejected; (d) **conditional required-presence**: a signal/JSON payload asserting `d == "t"` but omitting a `when d == "t"` field — rejected at admission (the positive obligation, §5.7 Finding 2); (d') **positive control fixture**: a payload asserting `d == "t"` that carries an *inapplicable* `when d == "u"` sibling (or `null` for it) — **accepted** (proves we do NOT over-reject; this is the all-keys-with-nulls webhook shape); (e) **out-of-range discriminant value** at admission — rejected (already true today; the model pins it stays true under Family B).

**Lean** (`Whipple/Refinement.lean`): the field-presence map is consistent (a field is present in a well-formed value of tag `t` iff its `when` holds), and narrowing on the discriminant yields exactly the conditioned-present set. **Non-dependent**: presence modeled as a finite tag→fieldset function (no value-indexed types). Fallback if review finds a case needing value-indexed types (e.g. computed discriminants): that case is pushed out of v1 — literal-union discriminant fields only, no computed discriminants (hard corner, §7).

### 6.4 Stage 4 — `milestone-signal.maude`

Bite: a parent `after p reaches "x"` for an undeclared milestone — rejected; the terminal-only observation invariant preserved (parent cannot observe a state the child did not project); milestones are durable facts with an admission/idempotency contract. No new Lean (reuses terminal-family proofs; milestone set is an extended `tags()`).

### 6.5 Cross-cutting — IFC label-transparency obligation (Stage 1)

Narrowing is an IFC *no-op*, but model-first means we pin that rather than assume it. **Maude bite** (extend the existing `infoflow-*` models, not a new family model): a narrowed value `b` from `case x { tᵢ as b => … }` reaching a governed sink produces the **same** `injects`/`leaks` verdict as the un-narrowed scrutinee `x` reaching that sink — i.e., narrowing introduces no flow edge and lowers no label (carriage unaffected). Negative control: a narrowed low-integrity value into a higher-integrity sink is still rejected (proves narrowing did not launder). The **selector doctrine** needs *no new model* — it is the existing NMIF machinery (`NMIF.lean` `untrusted_declassify_only_public`, `infoflow-signature.maude` attacker-steered-crossing bite) with the discriminant supplied as the selector; it activates when in-arm crossings land (E4/DR-0030 Direction C) and is vacuous until then. The **divergent-sink lint** is heuristic (no soundness claim) and carries no model obligation.

---

## 7. Staged Implementation Plan

Cheapest/most-certain → most speculative. Each stage gets review + verify + docs before its box is checked (per the per-piece review gate).

### Stage 1 — Unify the four spellings (SEAM 1). Foundational; do first.

**Recosted honestly (resolves major, lens 3):** ~500–600 LOC compiler (collapse `case_branch_payload_binding` + `terminal_payload_schema_for_tag`, `finite_case_domain` + `terminal_case_tags` and its duplicates, and the ~225-line `validate_case_blocks` into one pass; add `binding`+`guard` to `IrCasePattern`; delete the textual scanner) + ~150–200 LOC Maude/Lean + full example migration + tooling. We split it:

- [ ] **Stage 1a — narrowing-rule unification, no syntax break.** One unified pass `validate_and_lower_cases` running *after* `effect_payload_types` is populated; `Family::payload`/`Family::tags`; single `branch_scope` site; `after`-desugar to `case` (§5.1); delete textual scanner; add `duplicate_binding_in_scope` (§5.5) and observer-only construction check (§5.4); exhaustiveness lifted to **error** with the conditional-coverage diagnostic; redundancy check. The space-form terminal syntax still parses (adapter into the unified IR) so safe work lands first.
- [ ] **Stage 1b — syntax break to `Tag as binding`.** Ship `whip fmt --upgrade-as-bindings`; migrate the corpus; delete the space-form parser path; update spec/sum-types.md §2.1 and language.md §B.2.2. Deferrable/reconsiderable independently of 1a.

**Tooling:** `fmt` — one case-block formatter; **add a `case_nesting_depth` lint and confirm idempotency on nested `case`-in-`after`-in-`case` first** (memory flags nested-block non-idempotency). `lsp` — hover/completion/rename on case bindings (now in IR). `lint` — `mixed_case_binding_style` becomes obsolete.

**Acceptance:** corpus compiles after migration; three passes are one; `after succeeds/fails as` gone as a distinct path (grep proves it); `fails as f` narrows to `TerminalFailed`; all Maude bites fail-closed; `Narrowing.lean` builds under `check-lean-models.sh`; desugar spans point at original `after` lines; full gate green incl. `cargo fmt --all --check`.

### Stage 2 — Family A: coerce/decide → user enum (SEAM 2). Cheapest *feature*.

**Code surface:** the sum-type schema/fixture/return-type resolution of §5.3 (this is more than "tests + docs" — the latent path emits a string-only schema and string-only fixtures, which are wrong for payload-carrying enums). Fixture provider returns first variant or `--variant`; guard against double-wrapping (`schema_ref_is_class`).

**Tooling:** `lint` — `unused_coerce_result` fires when an enum result is never `case`d; `lsp` completes enum variants inside the inner `case`.

**Acceptance:** e2e `coerce f() -> Enum … after f succeeds as c { case c {…} }` parses, lowers to a union schema, synthesizes `anyOf` of object schemas, runs deterministically under fixtures, dispatches per variant; golden-fixture test for `--variant`; docs/example added (extends memory's "coerce → Class" e2e to "→ Enum"). Real-example target: **`coerce-branch`**.

### Stage 3 — Family B: discriminant-string schemas (SEAM 3). Genuinely new type-system step.

**Code surface:** add `presence_condition: Option<(FieldName, String)>` to `IrClassField` (parser lib.rs:1054 — today just `{name, ty, is_key, span}`; the grounding confirms this is a small, isolated IR change) to carry `field Type when discriminant == "literal"`; a derived `present: Tag → Set<Field>` per discriminant schema; extend `validate_known_field_paths` (lib.rs:11057) to consult it under `branch_scope`; **conditional required-presence admission** wired into the *already-strict* validators `validate_json_for_object` (main.rs:19394) and `validate_ingest_value` (main.rs:19539) — the net-new bit is gating the existing required-field check on the discriminant value (§5.7); out-of-range discriminant rejection is inherited for free (already at ~main.rs:19314-19328); scrutinee form `case e.kind { "literal" => … }` (literal-union family; `payload = ⊥`; narrowing augments root binding `e`, no separate `Event.deploy` registry classes). Side-conditions enforced statically: discriminant must be literal-union-typed (not bare `string`); `when` references only the same schema's discriminant (no cross-field cycles, no nested discriminants in v1); every `when` literal ∈ the union.

**Scope honesty (resolves major, lens 3):** Family B is a **lite refinement layer for literal-union discriminants only**. Computed/multiple discriminants are out of scope and will require redesign; a v2 design-review gate is the commitment if they're requested. `when kind=="x"` is **not** sugar for `Optional` — it gives a *presence guarantee* inside the matched arm (the field is `T`, not `Optional<T>`); `Optional<T>` means "may exist independent of anything." Negation/other-arm access forbidden in v1.

**Boundary admission (REWRITTEN 2026-06-28 — was "the most contestable call"; grounding settled it).** The system is *already* strict everywhere (no lenient path exists; event-ingress.md:128 promises rejection of malformed payloads), so strict is consistency, not a new policy. The net-new obligation is narrow — **conditional required-presence** (§5.7 Finding 2): reject a payload that asserts `d == "t"` but omits a `when d == "t"` field, with a precise diagnostic naming the field and the discriminant value. We deliberately do **not** reject payloads that carry *inapplicable* sibling fields (those are statically unreadable, so their presence is irrelevant to soundness, and rejecting them would break the common all-keys-with-nulls webhook shape). `null` ≡ absent for presence. The earlier "reject contradictory shapes" framing is dropped as both unnecessary and harmful. This is no longer a hard corner; the only residual micro-question is whether to *lint* (warn, not error) an inapplicable sibling as a possible authoring mistake — see Open Question 1.

**Tooling:** `lint` — `unconditioned_discriminant`, `unreachable_when`; `lsp` hover on a `when`-field shows its presence condition; `fmt` handles postfix `when`.

**Acceptance:** **`event-bridge.whip`** rewritten to declare `Event` with `when`-conditioned fields and `case e.kind`, replacing the rule-level `where deployed.status == "ok"` workaround; a malformed inbound payload is rejected at admission with a clear diagnostic; all Maude bites fail-closed; `Refinement.lean` builds.

### Stage 4 — Family C: child-milestone lifecycle (SEAM 4-realistic). Speculative but bounded.

The terminal family generalizes to a lifecycle family **only for observing a child workflow**, and **only over states the child explicitly projects as durable facts** — not `running`/`paused` (ephemeral control-plane states, control-plane.md:84-94).

```whip
invoke Pipeline { … } as p

after p reaches "canary_live" as m { notify_dashboard with m.region }  // child-emitted milestone
after p succeeds as r { … }   // terminal family, unchanged
after p fails    as f { … }
```

**Code surface:** child-side `emit milestone "<name>" {…}` (additive signal projection with admission/idempotency); parent-side `after p reaches "<name>" as m` desugaring into the lifecycle family; a per-child milestone registry feeding `Family::tags`. `reaches` is `ImplicitKernel` with `tags()` = the child's declared milestone names ∪ terminals. This is *additive signaling* — the child chooses what to project — so the terminal-only observation invariant and flow-namespace discipline hold.

**Tooling:** `lint` — `undeclared_milestone`; `lsp` completes a child's declared milestones at `reaches`.

**Acceptance:** **`revision-parent-child.whip`** extended so the parent observes a child milestone *and* its terminal, both via the unified case/narrowing core; terminal-only invariant holds; flow-namespace facts untouched. **— DONE 2026-06-29 (see the Revision 2026-06-29 header for the as-shipped design + the two scope-controlling decisions that avoided the `IrEffectKind` cascade and all golden churn).**

### Stage 4-self — Self-instance typestate: OUT OF SCOPE for v1.

Matching the *current instance's own* live state (`Running`/`Paused`/…) is out of scope. Reasons:
1. **Flow-namespace collision.** Exposing self-state as a matchable fact (some `FlowAwait_instance_*`) violates the ownership invariant `validate_flow_namespace_access` enforces (lib.rs:6020-6051): flow/instance progression state is owned by generated rules, unreadable by user rules.
2. **`running`/`paused` aren't durable facts.** They are ephemeral control-plane states; making them matchable needs continuous control-plane→workflow signaling on every transition — a different mechanism with its own race surface.
3. **Races.** A rule reading "I am paused" races the control-plane pause/resume; the flow-namespace discipline exists to prevent exactly this.

Softened per review (lens 3): this is not architecturally impossible — if revisited it is a **separate research thread requiring its own ownership regime orthogonal to flow-namespace discipline**, not an extension of discriminated families.

### 7.1 Cross-stage sequencing note

Stage 1 must move `effect_payload_types` population **before** the unified case pass: terminal guard validation depends on it, and the unified pass makes that dependency explicit (a stated benefit). Lazy/post-unification payload collection is forbidden (would silently empty guard types) — assert non-emptiness.

### 7.2 Sequencing across the IFC leftover (added 2026-06-28)

Discriminated families and the deferred IFC items (`information-flow-audit-findings.md`) are **two largely independent tracks** — narrowing is IFC-independent by the label-transparent design (§5.9). **No deferred IFC item is a soundness hole** (Waves 0–6 closed the bug class; check-time + runtime admission are live), so the IFC leftover is prioritized by value, not forced. **Recommendation: lead with discriminated families** (higher marginal value, lower risk, fixes the latent `schema_for_ref` bug); the IFC substrate follows. Only **three synchronization points** couple the tracks:

1. **E4 ↔ selector doctrine.** **E4 LANDED 2026-06-28** — both `endorsed` and `declassified` ship as trailing markers on `coerce` (audit-findings E4: leaf flags on `BodyEffectKind::Coerce`/`IrEffectNode`, zero golden churn). So source crossings now exist; the discriminant-is-the-selector rule (§5.6 channel 2) was the *only* dormant half and is now activatable the moment `case`/narrowing lands — the combined NMIF test at `case ∩ (endorsed/declassified)` just needs `case`. **Action:** make the selector-doctrine wiring (discriminant → NMIF selector for an in-arm crossing) + its Maude bite a **Phase-1 capstone** (right after Stage 1a/2), turning §5.6/5.9 into a covered test rather than dormant prose. (`declassified`-on-coerce is the sharp instance — it pairs directly with Family A's coerce→enum.)
2. **Stage 4 milestones ↔ boundary modeling.** Milestone signals are new cross-instance egress/ingress; model them as labeled boundaries — but they are ordinary signals, so they work at **resource grain today** (reuse the five-doors/emit machinery). No hard block.
3. **DR-0030 Direction B ↔ transparency re-verify.** When value-flow / per-field labels land (DR-0030 B), **re-run the §6.5 narrowing-transparency Maude bite at field grain**. Verification, not a blocker.

Recommended phase order (full rationale in the chat thread; durable IFC side tracked in `information-flow-audit-findings.md`):

- [ ] **Phase 1 — Foundation.** Stage 1a (unify, no syntax break) → Stage 2 (Family A) → selector-doctrine capstone (wire the `case` discriminant as the NMIF selector for an in-arm `endorsed`/`declassified` crossing — E4 already landed — + Maude bite).
- [ ] **Phase 2 — Discriminated heavies.** Stage 1b (syntax break + migration) → Stage 3 (Family B) → Stage 4 (Family C, milestone-as-boundary at resource grain).
- [ ] **Phase 3 — IFC substrate.** E5 (typed `kind:address` ids + port-level reach) + E6 (reader/writer SETS in the checker) — brings the checker up to the Wave-4-proven algebra; prerequisite for all finer governance.
- [ ] **Phase 4 — Join-box refinement.** DR-0030 Direction A v1 (flow_signature schema → producer reach-vector → consumer derive/gate; folds in X3/X4 hardening) → Direction B (then sync point 3). v2/conditional-discount and Direction C stay demand-gated.

**Priority flip signal:** if the *read-secret/emit-benign* false positive (DR-0030's canonical pain case) is actively blocking real whips, pull Phase 3 → Phase 4 **ahead of** Phase 2 — it's the only deferred IFC item with user-facing bite. **Demand-gated / operational** (independent of both tracks): E7 (account binding), D4 (envelope versioning). **Free now:** adopt M4 ("negative bite per consumer per trusted artifact") as a standing review rule.

### 7.3 Family C runtime design — GROUNDED (research 2026-06-28, de-risks the open runtime gap)

The earlier worry was that Family C needs deep, net-new cross-instance runtime. Grounding in the actual runtime shows the opposite: it's **~90% reuse of the path that already delivers a child's terminal to its parent.** The runtime is **poll-based + fact-driven** — `whip dev --until idle` / `whip step` re-evaluate an instance's rules against its fact-base each tick (`ready_contexts`), and the parent's invoke effect **already derives facts into the parent's fact-base** from child state (`run_workflow_invoke_effect`, main.rs ~25631-25788: it reads the child's terminal and derives `workflow.invoke.succeeded`/`.failed` facts carrying the parent's `effect_id`). Milestones extend that exact mechanism mid-flight:

- **Child side** — `emit milestone "<name>" { … }` records a durable **milestone fact** in the child instance's own base (reuse `derive_fact` / the `emit`/notify path). It is *additive signaling*: the child chooses what to project, so the terminal-only observation invariant holds (the parent can only see milestones the child declared/emitted).
- **Parent side** — the parent's invoke effect, on each step, reads the child's emitted milestones (alongside the terminal it already reads) and **derives a `workflow.invoke.reached:<name>` fact** (keyed by the parent `effect_id` + milestone name) into the parent's fact-base. Idempotent via the existing `fact_id` keying (instance+rule+schema+fact_key, with the milestone name in the key) → each milestone delivered exactly once, no double-fire.
- **`after p reaches "<name>" as m`** lowers to a reaction on that `reached` fact (the same shape as `after p succeeds`, which already matches `workflow.invoke.succeeded`); `m` binds the milestone payload. The lifecycle family's `tags()` = the child's declared milestone names ∪ the four terminals.

**Net-new vs reuse:** reuse = `derive_fact`, the invoke-effect parent-fact-derivation path, `ready_contexts` matching, fact-id idempotency. Net-new = (1) parser for `emit milestone` (child) + `after … reaches` (parent), (2) the child-records-milestone-fact lowering, (3) the invoke effect reading child milestones and deriving `reached` facts, (4) a per-child milestone registry for `tags()`/exhaustiveness, (5) `milestone-signal.maude` (the §6.4 model). **Observation latency** is one parent step (the parent sees a milestone on its next tick) — consistent with the poll model and fine under `--until idle`. **DECIDED 2026-06-28 (Jack): poll.** The parent's invoke effect polls the child's milestones each step (mirrors terminal propagation exactly, needs no child→parent addressing); push-via-notify is rejected for v1. Latency-sensitive push can be revisited later if the one-step delay ever matters.

**Implementation-scope finding (2026-06-28, from starting the build) — SUPERSEDED by the as-shipped design (2026-06-29).** The first build attempt modeled `emit milestone` as a new `BodyEffectKind` → new `IrEffectKind::Milestone`, which DID cascade into ~15-20 exhaustive match sites and was reverted to keep the tree green. **The shipped design sidesteps that entirely:** a milestone is a *synchronous projection*, not an async effect, so it is a `BodyStmt::Milestone` that derives a `workflow.milestone:<name>` fact directly (record-like) — **no new `IrEffectKind`, no cascade.** `after p reaches "<name>"` is `AfterPredicate::Reaches` (fieldless/`Copy`, name on `AfterBlock.milestone`) whose IR `DependencyPredicate` maps to `Completes` (provenance-only); the real gating is text-keyed at runtime (`parse_after_header` → `reaches:<name>` → `fact_matches_after_predicate`). The parent invoke effect extension was small (~30 lines: `list_facts` the child, re-derive idempotent `workflow.invoke.reached:<name>`). The "per-child milestone registry" became `WorkflowInputSurface.milestones`, collected by scanning rule bodies for `emit milestone` (the emit IS the declaration; `of <Class>` types the payload) — no new `Item` variant. Net: it was a one-pass vertical (parser → validation/typing → runtime → e2e) with **zero `.ir` golden churn**. The `milestone-signal.maude` model gates the contract; see the Revision 2026-06-29 header for the full as-shipped account.

### 7.4 Selector-doctrine IR design — GROUNDED (research 2026-06-28, de-risks Task 4)

The blocker was that `ifc.rs` can't link a crossing-bearing effect to its enclosing case-arm discriminant. The clean fix is small and mirrors existing machinery:

- **Lowering** — the effect walk `walk_effects` (lib.rs ~7672-7809) already threads an `after_stack` through nested blocks but **drops case context** when it descends into a `case` arm (~7764). Add a parallel **`case_stack: Vec<(scrutinee, literal)>`**, pushed/popped around each arm exactly like `after_stack`, and stamp the innermost entry onto each effect.
- **IR** — add **`selected_by: Option<(String, String)>`** (scrutinee root + arm literal) to `IrEffectNode` (lib.rs ~1194-1224, beside the `endorsed`/`declassified` flags). Make it **non-serialized** (like those E4 flags) → **zero `.ir` golden/hash churn**.
- **IFC** — in `check_with_envelope`, for an effect with `endorsed || declassified` AND `selected_by = Some((root, _))`, derive the discriminant's integrity from the root binding's source (reuse the existing low-integrity-source detection: `when message from <ch>` / `when human answered`, ifc.rs ~771-788, + `integrity_authority` ~442). If the discriminant is low-integrity, **reject** (NMIF-on-the-selector). This is the §5.6 channel-2 crossing rule, now implementable.

**Net-new vs reuse:** reuse = the `after_stack` threading pattern, the low-integrity-source detection, `integrity_authority`. Net-new = the `case_stack` + `selected_by` field + the one IFC check + a `determine_binding_integrity(rule, root)` helper. The `case-selector.maude` bite already models the property. **Confirm before building:** the `selected_by` shape (scrutinee+literal vs a richer boundary) and that non-serialization is acceptable (it is, per the E4 precedent).

**IMPLEMENTED 2026-06-28 — with a reachability finding for review.** Landed: the `case_stack` in `walk_effects`, a non-serialized `selected_by: Option<(String,String)>` on `IrEffectNode` (zero `.ir` churn — verified by the snapshot test), and the NMIF-on-the-selector check in `check_with_envelope` (rejects a crossing whose discriminant root is a low-integrity `when` binding). IR linkage is tested (`case_arm_effect_records_its_selector`); the IFC check builds and breaks nothing. **BUT the check is currently unreachable at source level, exposing a real gap:** the only low-integrity sources today are `message from <channel>` and `human answered`, whose bindings (`Message`/`HumanAnswer`) carry **only `string` fields — none caseable** (you can't `case e.text`). The natural *caseable* low-integrity discriminant is a **Family B signal** (`when <signal> as e; case e.kind { … }`), but a signal-trigger binding is **not currently flagged low-integrity** (only the channel `message from` form is, H3). So the selector doctrine only bites once **signals are treated as a low-integrity source** — which is a genuine IFC-scope decision (signals are inbound/attacker-influenceable, so flagging them is consistent with H3, but it would newly subject existing signal-triggered rules, e.g. `event-bridge`, to injection checks under a governed envelope). **Recommendation:** make "signal triggers are low-integrity" an explicit IFC decision (own small slice), after which the selector check becomes live and a Family-B-signal + crossing test exercises it end-to-end. Until then the check is correct-but-dormant (harmless).

---

## 8. Non-Goals & Hard Corners

**Non-goals:**
- **Self-instance typestate** (§7) — flow-namespace collision.
- **Generics / nested-sum payloads / recursion** — a family payload is always a flat record, never another family; exhaustiveness stays decidable over a finite tag set.
- **Full PC-label / implicit-flow tracking through case selection** (§5.6) — narrowing is label-*transparent*; the selection channel is handled by the existing I-IFC5 scope (ordinary effects, same as a `where` guard) + NMIF-on-the-selector for in-arm crossings (DR-0030 A.4.3) + a v1 divergent-sink lint. No PC-label machinery.
- **Multiple/nested/computed discriminants, negation/other-arm refinement** (Family B) — one top-level literal-union discriminant, no `if kind != x` narrowing in v1.
- **User-constructible terminal/lifecycle values** — observer-only.
- **Open-ended terminal tag set** — `Completed|Failed|TimedOut|Cancelled` fixed by control-plane.md; new task-local states belong to milestone signals (Stage 4), not the terminal family.

**Hard corners (for adversarial review):**
1. **Conditional required-presence for Family B (§5.7, Stage 3)** is the one load-bearing admission check: narrowing soundness needs the *positive* guarantee that right-arm fields are present, nothing more. ~~Most contestable call~~ — DOWNGRADED 2026-06-28: grounding showed strict admission is the existing system-wide invariant (event-ingress.md:128, no lenient path exists), and soundness needs only positive presence, not rejection of inapplicable siblings. The corner is now small and consistent with precedent.
2. **Guarded-arm non-coverage (§2.5)** — conservative; `case x { Completed as r where r.ok => … _ => … }` needs the `_`. Correct (guards fail at runtime); the diagnostic explains it.
3. **`after fails as f` now narrows to `TerminalFailed` (§5.2)** — behavior change; confirm the exposed field set (`reason/summary/effect_id/run_id`).
4. **Binding-name uniqueness (§5.5)** — `duplicate_binding_in_scope` enforces it; without it `effect_payload_types` keying is ambiguous in nested scopes.
5. **Fixture determinism for enum coerce (§5.3)** — "first declared variant" must be stable; the coerce idempotency key includes the output-schema hash, so adding/removing/reordering variants correctly invalidates prior results (by design; may surprise shared-library users — documented). Variant *removal* leaves no gap: the hash changes, invalidating transparently; no separate versioning needed.
6. **Lean refinement staging (Stage 3)** — Family B claimed provable without dependent types via a finite tag→fieldset map; if review finds a value-indexed case, it's pushed out of v1 (literal-union, no computed discriminants).
7. **`after`-desugar visibility (resolves major, lens 3)** — desugaring `after X succeeds as r` to a `case` with synthesized `_ => {}` could surprise authors who read `after` as a built-in. Decision: **`after X succeeds/fails/times out as` remains a built-in surface form** that *desugars in the IR* to `case`; language.md documents it as sugar and explains the `_` fallthrough. The syntactic distinction between "wait for an effect terminal" and "switch on a typed value" is preserved at the surface; only the IR is unified. This keeps the single-narrowing-path benefit without the cognitive cost.

---

## 9. Open Questions

1. **Inapplicable-sibling hygiene (the residual Family B micro-question).** v1 *accepts* a payload that carries a `when d == "u"` field on a `d == "t"` event (soundness-irrelevant; supports all-keys-with-nulls webhooks). Open: should we additionally emit a **lint warning** (never an error) when a non-null inapplicable sibling appears, as a possible authoring/upstream mistake? Leaning yes-as-opt-in-lint, no by default — but it's the only live call left in admission. (Note: the broader "lenient ingestion / drop-unknown-fields" question is *not* Family-B-specific — undeclared fields are already rejected system-wide by event-ingress.md:128; if partner webhooks need that relaxed it's a pre-existing platform decision, not part of this design.)
2. **Computed/multiple discriminants** — the v2 gate (§7, Stage 3) if requested; would force a real refinement-type rearchitecture.
3. **Milestone payload typing** (Stage 4) — whether child milestones may carry typed payloads beyond `{region}`-style flat records, and how the parent's `tags()` stays in sync across child revisions (likely via the program-version/revision-epoch already in effect keys, per memory).
4. **Liveness false-positive surface** — whether to add a `flow is non-self-terminating` annotation so liveness can be enforced (error) where the author asserts self-termination, tightening the headline guarantee from "warning" to "error" for opted-in flows.
5. **IFC selection-channel diagnostics** — *resolved 2026-06-28:* the divergent-governed-sink **lint is IN v1** (warning, no soundness claim, §5.6 channel 2). Residual: when in-arm crossings land (E4/DR-0030 Direction C), whether the NMIF-on-selector rejection (discriminant must act-for the release authority) wants a dedicated diagnostic beyond the generic crossing-rejected one. Vacuous until crossings exist.

---

## Appendix A — Review dispositions

**Lens 1 (soundness/implementability):**
- *[blocker] three-pass guard/`effect_payload_types`* → §2.3, §6.1, §7.1: guard validation is part of NARROW in `Γ'`; `effect_payload_types` is a re-derived precondition input, populated before the unified pass; one `branch_scope` site; Maude terminal-guard bite (§6.1).
- *[blocker] binding-name uniqueness* → §5.5: `duplicate_binding_in_scope`, runs before any map-key use; test specified.
- *[blocker] observer-only construction* → §5.4: producer-context flag + rejection check + test.
- *[blocker] Family B boundary strictness* → §5.7, §6.3, Stage 3: **reframed 2026-06-28 after grounding** — strict admission is the pre-existing system-wide invariant (event-ingress.md:128), and soundness needs only *conditional required-presence* (positive), not rejection of inapplicable siblings; Maude bites (d) positive obligation + (d') accept-control + (e) out-of-range; Lean hybrid statement (§5.8). The blocker is resolved more cheaply than the original draft assumed.
- *[blocker] after-alias binding order* → §5.1: desugar before unified pass, binding flows through `effect_payload_types` exactly like inline `case`; scanner deleted; span preserved.
- *[major] exhaustiveness severity* → §2.5, §4: lifted to hard error with conditional-coverage diagnostic; acceptance test.
- *[major] out-of-range discriminant* → §5.7: strict reject at admission.
- *[major] flow-namespace Family C enforcement* → §2.4: trait contract + compiler assertion + Maude model that `ImplicitKernel` discriminants may not reference `FlowAwait_*`.
- *[major] IFC low-integrity discriminant* → §5.6 (rewritten 2026-06-28 for DR-0029/0030 alignment, Jack-signed): narrowing is **label-transparent** (granularity-agnostic, never lowers a label — DR-0030 Decision 0); value channel protected by existing sink-keyed `injects`/`leaks`; admission ≠ endorsement; selection channel = I-IFC5 scope for ordinary effects (+ v1 divergent-sink lint) and NMIF-on-the-selector for in-arm crossings (DR-0030 A.4.3, vacuous until crossings land); zero new IFC syntax. Full alignment in §5.9.
- *[minor] span-safety* → §5.1: reuse original `after` span via `locate_span`; regression test.

**Lens 2 (formal-model + structured-output reality):**
- *[blocker] coerce→enum union schema* / *[blocker] fixture for sum types* / *[major] return-type resolution under-specified* / *[minor] envelope wrapping* → §5.3: return type resolves to union of generated classes; `anyOf` object schemas; fixtures emit full objects with `--variant`; `schema_ref_is_class` true for payload enums to avoid double-wrap; golden test.
- *[major] `after fails as` narrowing* → §5.2: deliberately implemented; `TerminalFailed` binding.
- *[major] Maude NARROW bites* / *[major] Narrowing.lean* → §6.1: four negative + one positive fixture with `RESIDUAL:Cfg`; Lean scope judgment + non-interference lemma + totality theorem.
- *[minor] after→case desugar incomplete* → §5.1, §7.1: implemented in lowering before the unified pass; scanner deleted (grep-verified).
- *[minor] Family B implementation gap* → §6.3, Stage 3: greenfield with Maude spec first.
- *Missing — guarantee overstated* → §4: restated as three honest layers, scoped to self-terminating flows.
- *Missing — auto-fail/exhaustiveness composition proof* → §6.1: explicit Maude composition obligation + "unhandled" definition.
- *Missing — redundancy detection unimplemented* → §2.5: left-to-right reachability scan in the unified validator; Maude bite (c).
- *Missing — fixture determinism / variant removal* → §7 corner 5: hash-based invalidation handles add/remove/reorder transparently; documented.

**Lens 3 (ergonomics/cost/abstraction):**
- *[major] syntax break understated* → §3: acknowledged migration lift; `whip fmt --upgrade-as-bindings`; spec updates listed; Stage 1b separable.
- *[major] Stage 1 cost accounting* → §7 Stage 1: recosted ~500–600 LOC + 150–200 LOC modeling + tooling itemized; split 1a/1b.
- *[major] Family trait over-claims unification* → §2.4: reframed "unified narrowing with parameterized discriminant dispatch"; elimination unified, production not.
- *[major] Family B refinement deferral* → §7 Stage 3: explicitly "lite refinement, literal-union only"; v2 gate; `when` vs `Optional` distinction stated.
- *[major] after-desugar loses distinction* → §7 corner 7: `after … as` kept as built-in surface, desugars only in IR; language.md documents the sugar.
- *[minor] guarantee composition specifics* → §4: honest three-layer restatement.
- *[minor] IFC "no-op" understated* → §5.6: narrowing is label no-op *and* visibility control point.
- *[minor] Stage 4-self exclusion overstated* → §7: softened to separate research thread, not impossible.
- *Missing — fmt idempotency on nested case* → §7 Stage 1 tooling: `case_nesting_depth` lint + idempotency confirmation required first.
- *Missing — Family B error messages / lenient path* → §5.7, Stage 3: precise conditional-required-presence diagnostics specified; "lenient ingestion" recognized as not Family-B-specific (undeclared fields already rejected system-wide); residual micro-question is an opt-in inapplicable-sibling lint (Open Question 1).

---

## Appendix B — Assumptions (collected)

- Greenfield rebuild is sanctioned (model-first, then greenfield; no back-compat); the corpus migrates mechanically via `whip fmt --upgrade-as-bindings` and old paths are deleted.
- `TerminalFailed/TimedOut/Cancelled` and the reserved `variant` enum field are stable and become single sources of truth once de-duplicated.
- Coerce/decide → enum infra needs the schema/fixture/return-type work of §5.3 (the latent path emits string-only schemas/fixtures, wrong for payload enums); Stage 2 absorbs this and still holds its "cheapest feature" ordering.
- The hermetic Lean 4 layer (acts-for/Decide/Boundary landed) hosts `Narrowing.lean` and `Refinement.lean` under `scripts/check-lean-models.sh` without Mathlib.

**Key code anchors (all in `crates/whipplescript-parser/src/lib.rs` unless noted; verified against tree 2026-06-28):** unified pass replaces `validate_case_blocks` lib.rs:9366 / `collect_rule_case_metadata` lib.rs:6823 / `collect_terminal_case_metadata` lib.rs:7045-7115 (the three passes are dispatched sequentially at lib.rs:6524-6533); payload lookup unifies `case_branch_payload_binding` lib.rs:7011 + `terminal_payload_schema_for_tag` lib.rs:7225; tag domain unifies `finite_case_domain` lib.rs:9915 + `terminal_case_tags` lib.rs:9734 (consumed by `validate_terminal_case_pattern` lib.rs:9738 and `validate_terminal_case_coverage` lib.rs:9777); guarded-arm filter lib.rs:9866; `branch_scope` lib.rs:6831-6862; `IrCasePattern` enum lib.rs:1277-1284 (`EnumVariant(String)` carries NO binding; only `OptionalSome{binding}` does — verified); `effect_payload_types: BTreeMap<String, IrType>` populated by `collect_effect_payload_types` lib.rs:6770-6787; reserved `variant` synthesis lib.rs:5286; delete textual after-alias lib.rs:6462-6510 (the `fails => {}` no-bind at :6504 — verified); coerce return-type at lib.rs:6789-6820; coerce schema/strict in `crates/whipplescript-kernel/src/coerce_native.rs` (`schema_for_ref` :151-166, `json_schema_for_type` :79-186, `output_schema_envelope` :199, `schema_ref_is_class` :219, `is_strict_compatible` :388); fixture gen `crates/whipplescript-cli/src/main.rs` (`ingest_shape_json` :19430, `fixture_value_for_shape` :19484); admission validators in main.rs (`validate_json_for_object` :19394, `validate_ingest_value` :19539, union-range check ~:19314-19328); IFC rule at ifc.rs:661-677 (label-preserving, no narrowing-endorse); flow-liveness composition at flow_expand.rs:622-670 + flow-autofail.maude; flow-namespace ownership at lib.rs:6020-6051.
