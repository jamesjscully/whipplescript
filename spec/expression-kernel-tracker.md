# Expression Kernel Tracker

Status: active implementation tracker

This tracker breaks the expression-kernel spec into concrete implementation
work. The source of truth for semantics is
[expression-kernel.md](expression-kernel.md); this file tracks what has landed
in parser, compiler, runtime evaluation, formal modeling, tests, examples, and
agent-facing docs.

The expression kernel covers deterministic logic used by:

- `when ... where <expr>` guards
- `assert <expr>` workflow checks
- fact/effect projection queries
- interpolation paths
- typed record/effect arguments
- static matrix rows
- future action/template parameters

## Status Legend

- [x] Implemented and covered by tests.
- [~] Partially implemented or modeled, with known gaps.
- [ ] Not implemented.

## Development Pipeline Backlog

Every remaining feature must move through the same gates before it is treated
as complete:

1. **Spec**: source syntax, static semantics, runtime semantics, IR/event
   shape, and compatibility rules are explicit enough to implement.
2. **Validation**: Maude model/search, parser/type-checker matrices, golden IR,
   runtime fixtures, or harness failure fixtures define the expected behavior.
3. **Implementation**: parser, lowering, evaluator, runtime/store, CLI, or
   harness code implements the specified behavior.
4. **Tests**: unit, snapshot, fixture, e2e, or generated-model tests cover pass,
   fail, and error paths.

| Feature | Spec | Validation | Implementation | Tests | Owner / Notes |
| --- | --- | --- | --- | --- | --- |
| Full guard/assertion type checking | [x] | [x] | [~] | [x] | Assertions now reuse guard-style expression validation for boolean results, finite-domain checks, unknown dotted roots, simple fact-query guards, function arity/shape checks, and unknown query-field/schema diagnostics. |
| Expression object literals | [x] | [x] | [~] | [x] | Record-field expected-schema object/map literals now work for inline construction and dogfood metadata. Remaining gaps: effect argument fields, multiline nested object bodies, and general expression AST object literals. |
| Duration/time ordering | [x] | [x] | [~] | [x] | Added executable tests documenting current limitations: duration/time ordering is rejected as non-numeric and source cannot seed concrete duration/time values yet. |
| Enum/literal finite-domain typing | [x] | [~] | [~] | [~] | Spec now requires precise finite-domain types, symmetric comparison checks, and contradiction diagnostics. Typed query heads remain partial. |
| Generated per-program Maude checks | [x] | [x] | [~] | [x] | Generated searches now cover symbolic guard true/false/error and assertion pass/fail/error non-mutation while preserving dependency checks. Remaining gap: full expression-semantics lowering rather than finite symbolic outcomes. |
| AgentRef profile/capacity/capability constraints | [x] | [x] | [~] | [x] | Store claimability now filters policy-blocked effects and enforces per-agent capacity when declarations are persisted. Remaining gaps: persist declared agents from real programs, dynamic AgentRef ambiguity checks, and durable blocked-by-capacity status. |
| Tagged terminal-output union branch matching | [x] | [x] | [ ] | [~] | Tracker now has a concrete source/IR/lowering/runtime/test checklist. `after ... completes` still lacks a typed terminal union payload. |
| Branch pattern spans and typed lowering | [~] | [x] | [~] | [x] | Read-only analysis identified exact parser/CLI targets. The branch-binding leak is fixed and tested; branch-level source spans and typed rule-body lowering remain pending. |
| Assertion diagnostics and event surfaces | [x] | [x] | [~] | [x] | Store now records/lists durable diagnostics; kernel trace and CLI trace JSON now support provider diagnostics. Remaining gap: persist assertion/provider diagnostics through store transactions. |
| Provider/harness failure capture | [x] | [x] | [~] | [x] | Kernel trace now includes provider diagnostics before terminal failure/timeout events and CLI trace rendering is tested. Durable store persistence remains partial. |
| Parser-only expression matrix | [x] | [x] | [x] | [x] | Parser-only tests now cover precedence, calls, fact/effect queries, map indexes, arrays, invalid syntax, and optional presence-proof syntax. |
| Golden IR expression fixture | [x] | [x] | [x] | [x] | Added `examples/expression-kernel-dogfood.whip/.ir` covering guards, assertions, projections, maps, arrays, optional presence, and deterministic routing. |
| Static-analysis diagnostic matrix | [x] | [x] | [~] | [x] | Added tests for assertion validation, symmetric finite-domain literal checks, and unknown dotted roots. Remaining categories need broader per-row coverage. |
| Companion-skill dogfood cleanup | [x] | [ ] | [~] | [ ] | Docs now define companion-skill dogfood expectations. Need deterministic routing workflows authored through the companion skill without LLM provider/model classification. |

### Completed Implementation Slices

| Slice | Files | Result |
| --- | --- | --- |
| Parser/type-checker validation | `crates/whippletree-parser/src/lib.rs` | Assertion validation, symmetric finite-domain diagnostics, unknown dotted-root diagnostics, simple fact-query guard typing, and parser-only expression matrix tests landed. |
| Generated Maude validation | `crates/whippletree-cli/src/main.rs` | Guard-gated rule and assertion non-mutation generated searches landed while preserving dependency checks. |
| Store diagnostics | `crates/whippletree-store/src/lib.rs`, `crates/whippletree-store/migrations/0001_runtime_store.sql` | Durable diagnostic record/list APIs, schema columns, idempotency indexes, and legacy upgrade coverage landed. |
| Golden dogfood fixture | `examples/expression-kernel-dogfood.whip`, `examples/expression-kernel-dogfood.ir` | Compiled golden fixture landed for guards, assertions, projections, map indexes, optional presence, arrays, and deterministic routing. |
| Object/map literal construction | `crates/whippletree-parser/src/lib.rs`, `crates/whippletree-cli/src/main.rs`, `examples/expression-kernel-dogfood.*` | Record-field map/object literals now validate and materialize, including dogfood `metadata { phase "kernel" }`. |
| Duration/time validation coverage | `crates/whippletree-cli/tests/control_plane.rs` | Added tests pinning the current unsupported state for duration/time ordering and value seeding. |
| Function/query validation fixture | `examples/invalid/bad-expression-functions.*`, `crates/whippletree-parser/src/lib.rs` | Invalid function/query diagnostics are now enforced and included in invalid fixture discovery. |
| Provider diagnostics trace slice | `crates/whippletree-kernel/src/trace.rs`, `crates/whippletree-kernel/src/lib.rs`, `crates/whippletree-cli/src/main.rs` | Provider diagnostics now appear in kernel trace and CLI trace JSON before terminal events. |
| AgentRef store enforcement | `crates/whippletree-store/src/lib.rs` | Claimability filters profile/capability-blocked effects and start-run enforces per-agent capacity when metadata exists. |
| Tagged terminal checklist | `spec/expression-kernel-tracker.md` | Added implementation checklist for tagged terminal-output union branch matching. |
| Case branch context safety | `crates/whippletree-cli/src/main.rs`, `crates/whippletree-cli/tests/control_plane.rs` | Failed guarded `Some` branches no longer leak payload bindings into later branches or fallbacks. |

## Current Implementation Summary

- [x] Source accepts guarded fact matches such as
  `when LanguageTask as task where task.provider == "codex"`.
- [x] Runtime `whip dev` evaluates simple guard equality/inequality.
- [x] Source accepts top-level `assert` statements.
- [x] Runtime `whip dev` evaluates assertion reports over fact/effect
  projections and exits nonzero on assertion failure.
- [x] Provider-language dogfood asserts provider counts, agent-turn counts, and
  BAML coerce counts in source.
- [~] Compiler validates known field paths in guards.
- [~] Rule bodies support concrete `case expr { Pattern => { ... } }` branches
  for enum/literal values and optional `Some`/`None` branches in the dev
  stepper, including branch-level `where` guards.
- [~] Maude has a finite abstract expression model for guard true/false/error,
  assertion pass/fail/error, optional presence, enum/literal domains, typed
  pattern branches, exhaustive finite-domain misses, and dynamic agent target
  validity.
- [x] Real compiler/runtime guard evaluation now uses a typed expression
  AST.
- [ ] Full guard/assertion type checking is not implemented.

## Feature Matrix

| Feature | Spec | Parser | Type Check | Runtime Eval | Maude | Tests | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Guarded fact match `when X as x where expr` | [x] | [x] | [~] | [x] | [~] | [x] | Guards now use the shared expression parser/evaluator; static typing remains partial. |
| Top-level `assert expr` | [x] | [x] | [~] | [x] | [~] | [x] | Assertions parse through the shared expression parser and run after `dev`; typed assertion scopes remain partial. |
| Fact projection query `Class where expr` | [x] | [x] | [~] | [x] | [ ] | [x] | Parsed as expression query nodes and evaluated in assertions. |
| Effect projection query `effect kind K where expr` | [x] | [x] | [~] | [x] | [ ] | [x] | Parsed as expression query nodes and evaluated in assertions. |
| `count(collection)` | [x] | [x] | [~] | [x] | [ ] | [x] | Implemented over fact/effect projection queries. |
| `exists(collection)` | [x] | [x] | [~] | [x] | [ ] | [x] | Implemented over projection queries. |
| `exists path` presence proof | [x] | [x] | [~] | [x] | [~] | [x] | Runtime checks non-missing/non-null; local parser proof tracking accepts `exists x` before optional field access. |
| `empty(collection)` / `empty(expr)` | [x] | [x] | [~] | [x] | [ ] | [x] | Implemented for projections, arrays, objects, strings, and null. |
| Equality `==` | [x] | [x] | [~] | [x] | [~] | [x] | Runtime supports JSON scalar equality; compiler rejects obvious incompatible scalar comparisons and finite-domain typos. |
| Inequality `!=` | [x] | [x] | [~] | [x] | [~] | [x] | Implemented with same static limitations as equality. |
| Boolean `&&` | [x] | [x] | [~] | [x] | [ ] | [x] | Runtime short-circuits; parser carries presence proofs left-to-right. |
| Boolean `||` | [x] | [x] | [~] | [x] | [ ] | [x] | Runtime short-circuits; presence-proof behavior remains conservative. |
| Boolean `!` | [x] | [x] | [~] | [x] | [ ] | [x] | Implemented in shared expression parser/evaluator. |
| Ordering `< <= > >=` | [x] | [x] | [~] | [x] | [ ] | [x] | Runtime supports numeric ordering; compiler rejects obvious non-numeric ordering. |
| Membership `in` / `not in` | [x] | [x] | [~] | [x] | [~] | [x] | Runtime supports array membership and map key membership; compiler rejects obvious non-array/non-map membership and incompatible item/key types. |
| Parentheses and precedence | [x] | [x] | [~] | [x] | [ ] | [x] | Shared recursive-descent parser handles precedence. |
| Array literals | [x] | [x] | [~] | [x] | [~] | [x] | Implemented for expression evaluation; compiler rejects obvious mixed scalar arrays. |
| Object literals in expected schema contexts | [x] | [~] | [~] | [~] | [ ] | [x] | Record bodies exist; expression-level object literals are not general AST nodes. |
| Map index `path["key"]` | [x] | [x] | [~] | [x] | [ ] | [x] | String-key map indexing works in guards/assertions; Maude coverage is still missing. |
| Enum variant values | [x] | [~] | [~] | [~] | [~] | [~] | Enum schemas exist; expression values are currently strings at runtime. |
| Literal-union values | [x] | [~] | [~] | [~] | [~] | [x] | Literal types exist; guards do not yet verify literal-domain membership. |
| Typed finite-domain pattern branches | [x] | [~] | [~] | [x] | [x] | [x] | Concrete rule-body `case` branches work for enum/literal values in `whip dev`; branch guards use the shared expression evaluator. |
| Exhaustiveness checks for finite patterns | [x] | [~] | [~] | [ ] | [x] | [x] | Parser diagnostics cover enum/literal/optional rule-body cases without fallback; not yet expression-level or source-span precise. |
| Optional Some/None pattern branches | [x] | [~] | [~] | [~] | [x] | [x] | Rule-body `Some name` binds a present runtime value; static presence proof is still local to case validation. |
| Optional presence proofs | [x] | [x] | [~] | [x] | [~] | [x] | Parser rejects local optional field access without `x != null`, `null != x`, `exists x`, or `!(x == null)` proof. |
| Missing vs null distinction | [x] | [x] | [~] | [x] | [~] | [x] | Expression evaluator preserves internal Missing separately from JSON null for guards/assertions/query filters. |
| Type-directed interpolation paths | [x] | [~] | [ ] | [~] | [ ] | [x] | Existing interpolation is path-oriented but not fully expression-kernel typed. |
| Dynamic `AgentRef<...>` | [x] | [~] | [~] | [~] | [~] | [~] | Source/IR support typed agent domains for record values and dynamic `tell`; still needs shared expression evaluator coverage. |
| Deterministic validation capability | [~] | [ ] | [ ] | [ ] | [ ] | [ ] | Still design-level; should handle checks that do not need BAML/model judgment. |

## Implementation Work Queue

### 1. Expression AST And Parser

- [~] Add expression AST nodes for literals, paths, unary ops, binary ops,
  calls, array literals, object literals, fact queries, effect queries, and
  map indexing.
- [x] Replace raw guard strings in the typed IR with parsed expression nodes
  while preserving source spans.
- [x] Replace raw assertion strings in the typed IR with parsed expression nodes
  while preserving source spans.
- [x] Implement precedence and associativity exactly as specified:
  path/indexing, unary/calls, ordering/membership, equality, `&&`, `||`.
- [x] Parse `exists path` separately from `exists(collection)`.
- [x] Parse `not in` as one membership operator, not as `not` plus identifier.
- [ ] Keep formatter output stable and parenthesize when precedence could be
  surprising.
- [~] Add invalid syntax diagnostics for dangling operators, unclosed
  parentheses, malformed queries, and unsupported function names.

### 2. Type Checker

- [~] Compute expression free bindings and verify each binding exists in the
  current rule/assertion scope.
- [~] Resolve every field/index path against class, array, map, optional, ref,
  and projection types.
- [~] Track result type for every expression node.
- [~] Reject non-boolean guard/assertion results.
- [~] Reject unknown fields and map indexes with non-string keys.
- [x] Reject field access through optional values without an accepted presence
  proof.
- [x] Implement presence proof tracking for:
  `x != null`, `null != x`, `exists x`, `!(x == null)`, and left-to-right
  `a && b`.
- [~] Reject incompatible equality comparisons.
- [~] Reject ordering on unsupported types and incompatible numeric/time types.
- [x] Reject string ordering unless a later spec explicitly enables it.
- [~] Reject enum variants outside their enum domain.
- [~] Reject literal values outside literal unions.
- [~] Reject membership against non-array and non-map operands.
- [~] Reject array literals whose elements do not share a valid common type.
- [ ] Reject object literals outside an expected schema context.
- [ ] Reject plain strings where `AgentRef<...>` is required.
- [~] Emit statically unsatisfiable finite-domain guard diagnostics when useful,
  for example `task.provider == "gpt5"` against `"codex" | "claude" | "pi"`.

### 3. Runtime Evaluator

- [x] Replace ad hoc guard and assertion string evaluators with one typed
  expression evaluator.
- [x] Preserve a strict `Missing` result distinct from `Null`.
- [x] Implement short-circuiting `&&` and `||`.
- [x] Implement `!`.
- [x] Implement scalar equality and inequality over typed values.
- [~] Implement ordering over int, float, duration, and time.
- [x] Implement membership for arrays and map keys.
- [x] Implement `exists path`, `exists(collection)`, `empty(...)`, and
  `count(...)`.
- [x] Implement fact and effect projection reads over typed query filters.
- [x] Ensure guard `false` means non-match and guard `Error` means no rule
  commit plus diagnostic.
- [x] Ensure assertion `false` or `Error` cannot mutate facts/effects and
  produces structured failure output.
- [~] Add deterministic JSON output for assertion actual/expected values and
  failure reasons.

### 4. IR, Lowering, And Generated Checks

- [x] Add typed expression IR with source spans and stable snapshot rendering.
- [x] Lower guard expressions into rule readiness predicates.
- [x] Lower assertion expressions into deterministic checkpoint metadata.
- [x] Lower fact/effect projection reads into rule/assertion read metadata.
- [ ] Extend generated per-program Maude checks so a rule can commit only after
  its lowered guard predicate is true.
- [ ] Generate Maude coverage for false/error guard cases and assertion failure
  non-mutation.
- [ ] Ensure generated checks preserve existing effect-graph dependency checks.

### 5. AgentRef And Deterministic Routing

- [x] Specify source syntax for `AgentRef<codex | claude | pi>` or its chosen
  equivalent in [type-system.md](type-system.md) and
  [language.md](language.md).
- [x] Represent `AgentRef` structurally in IR.
- [x] Type-check every possible agent target against declared agents.
- [ ] Type-check target profile, capacity, and capability constraints.
- [x] Reject dynamic `tell` targets that are plain strings.
- [x] Evaluate dynamic target expressions deterministically at rule commit.
- [x] Add provider-language dogfood coverage that routes through typed
  `AgentRef` once available.

### 6. Pattern Matching And Branching

- [x] Specify concrete source syntax for finite-domain branching:
  `case expr { Pattern => { ... } }`.
- [x] Support enum and literal-union branch matching in rule bodies.
- [x] Support optional Some/None branch matching that binds a
  proven-present value in the Some branch.
- [ ] Support tagged terminal-output union branch matching for effect
  completion facts.
- [x] Allow branch guards that reuse the current deterministic guard evaluator.
- [x] Reject unknown enum/literal variants in patterns.
- [x] Emit exhaustiveness diagnostics for finite domains where the branch result
  must be total.
- [~] Preserve source spans for branch alternatives in diagnostics.
- [~] Lower matching pattern branches before effect graph commit in `whip dev`.
- [x] Model finite-domain branch match/non-match in Maude.
- [x] Model exhaustive finite-domain miss diagnostics in Maude.
- [x] Model optional Some/None branch readiness and present binding in Maude.
- [ ] Defer deep object destructuring, array destructuring, user-defined
  extractors, and provider-text pattern matching until a concrete workflow
  requires them.

Tagged terminal-output union branch matching implementation checklist:

- [ ] Source syntax: accept `case <effectBinding>.output { <Tag> <name> [where
  <expr>] => { ... } }` in `after <effectBinding> completes` bodies, using the
  documented terminal tags such as `Completed`, `Failed`, and `Blocked`; keep
  plain status strings and provider transcript text out of the pattern syntax.
- [ ] Source binding rules: require each tagged branch to bind a payload name;
  inside the branch, the payload has only the fields declared for that terminal
  tag, while the outer effect binding remains available for common metadata.
- [ ] IR shape: represent the `after ... completes` binding as a typed
  terminal-output union whose alternatives carry `{ tag, payloadType,
  sourceSpan }`, and represent each branch as `{ tag, binding, guardExpr,
  body, patternSpan }` rather than lowering tags to ad hoc string comparisons.
- [ ] CLI lowering behavior: lower tagged branches before effect graph commit
  in `whip dev`; evaluate the terminal tag match first, then the branch guard
  with the tag-refined payload binding, and commit only the selected branch's
  facts/effects.
- [ ] Parser diagnostics: reject tagged-terminal patterns outside typed
  `after ... completes` scopes, unknown terminal tags, duplicate tags,
  branches without payload bindings, field reads invalid for the refined tag,
  non-boolean branch guards, and non-exhaustive total matches without wildcard
  or default coverage.
- [ ] Runtime branch selection: select exactly one branch for each terminal
  output by tag plus guard; treat non-matching guards as branch misses; surface
  multiple-match, no-match for required total cases, and guard evaluation errors
  as structured diagnostics with the branch span.
- [ ] Validation matrix: add parser/type-checker cases for accepted
  `Completed`/`Failed`/`Blocked` branches, guarded branches, unknown tags,
  duplicate tags, wrong-scope patterns, invalid payload fields, and missing
  payload bindings.
- [ ] IR snapshots: add a golden fixture showing terminal-output union
  alternatives and branch-level source spans for `after ... completes`.
- [ ] Runtime/e2e acceptance: add an e2e workflow where a completed turn records
  an artifact, a failed turn records provider failure, a blocked turn asks for
  human action, and an unmatched or erroring branch produces a deterministic
  diagnostic without committing sibling branch effects.
- [ ] Maude acceptance: extend the existing finite-domain branch model with
  terminal tags so generated/search tests cover tag match, tag miss,
  guard-filtered miss, and exhaustiveness diagnostics.

### 7. Tests And Fixtures

- [ ] Parser tests for every expression form.
- [~] Golden IR snapshots for guards and assertions using every operator class.
- [ ] Static-analysis tests for unknown bindings, unknown fields, optional
  misuse, invalid enums, invalid literal values, bad membership, bad ordering,
  bad array literals, and bad `AgentRef` targets.
- [x] Runtime tests for guard true, false, and error paths.
- [x] Runtime tests for assertion pass, fail, and error paths.
- [~] Parser/type-checker tests for enum, literal, optional, and tagged-union
  pattern branches.
- [x] Exhaustiveness diagnostic tests for finite pattern domains.
- [ ] Golden IR and e2e tests for tagged terminal-output union branches over
  `after ... completes` payloads.
- [x] E2E test showing `&&`, `||`, `!`, ordering, `in`, `exists`, `empty`, and
  `count` in source.
- [x] E2E test showing assertion failures reach JSON output and nonzero exit.
- [x] E2E test showing failed guards do not enqueue effects.
- [x] E2E test showing rule-body `case` branches select literal and optional
  patterns before recording facts.
- [x] Maude tests for guard false/error, optional presence, enum/literal domain
  validity, finite-domain pattern branches, optional Some/None branches,
  dynamic agent target validity, and assertion non-mutation.
- [ ] Companion-skill dogfood test that authors deterministic routing without
  asking an LLM to identify provider/model identity.

### 8. Docs And Agent Guidance

- [ ] Update [language.md](language.md) with concrete expression syntax once the
  AST parser lands.
- [ ] Update [static-analysis.md](static-analysis.md) with the implemented
  finite-domain and presence-proof diagnostics.
- [x] Update [type-system.md](type-system.md) with final `AgentRef` syntax and
  JSON/IR representation.
- [x] Update [companion-skill.md](companion-skill.md) to recommend deterministic
  routing metadata, `AgentRef`, source assertions, and projection checks.
- [ ] Update [e2e.md](e2e.md) with expression-kernel dogfood coverage.
- [ ] Keep this tracker and [implementation-plan.md](implementation-plan.md)
  synchronized as features land.

## Acceptance Gates

- [x] A shared task schema can route work across Codex, Claude, and Pi using
  deterministic source metadata without duplicate provider-specific classes.
- [~] Guards using enum/literal fields are type-checked and finite-domain typos
  are rejected or diagnosed before runtime.
- [~] Boolean, ordering, membership, presence, count, empty, and exists
  expressions work in both guards and assertions where semantically valid.
- [ ] Enum/literal, optional, and tagged-union pattern branches are typed,
  deterministic, and exhaustiveness-checked where the domain is finite.
- [x] Optional field access is rejected unless presence is proven.
- [x] Dynamic agent routing is typed as `AgentRef` or equivalent, and plain
  strings cannot target `tell`.
- [~] Assertion failures are visible in CLI JSON/human output, event or
  diagnostic surfaces, and CI exit status without mutating workflow state.
- [ ] Generated Maude checks include guard-gated rule commits and assertion
  non-mutation cases.
- [x] Companion-skill-authored workflows use deterministic routing and source
  assertions without prompt-level provider/model decisions.
