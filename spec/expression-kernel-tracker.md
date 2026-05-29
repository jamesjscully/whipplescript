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
- [ ] Real compiler/runtime guard evaluation does not yet use a typed expression
  AST.
- [ ] Full guard/assertion type checking is not implemented.

## Feature Matrix

| Feature | Spec | Parser | Type Check | Runtime Eval | Maude | Tests | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Guarded fact match `when X as x where expr` | [x] | [x] | [~] | [~] | [~] | [x] | Guards are raw text; only known field paths and simple equality are checked/evaluated. |
| Top-level `assert expr` | [x] | [x] | [ ] | [x] | [~] | [x] | Assertions run after `dev`; not yet typed before runtime. |
| Fact projection query `Class where expr` | [x] | [~] | [ ] | [~] | [ ] | [x] | Currently parsed inside assertion strings, not as a real AST node. |
| Effect projection query `effect kind K where expr` | [x] | [~] | [ ] | [~] | [ ] | [x] | Supports effect kind plus simple `where` equality/inequality over effect fields. |
| `count(collection)` | [x] | [~] | [ ] | [x] | [ ] | [x] | Implemented for fact/effect projection assertions. |
| `exists(collection)` | [x] | [~] | [ ] | [x] | [ ] | [ ] | Implemented for assertion projection counts, but lacks focused tests and guard support. |
| `exists path` presence proof | [x] | [ ] | [ ] | [ ] | [~] | [ ] | Needed for optional field access in guards. |
| `empty(collection)` / `empty(expr)` | [x] | [ ] | [ ] | [ ] | [ ] | [ ] | Not implemented. |
| Equality `==` | [x] | [~] | [ ] | [~] | [~] | [x] | Runtime supports JSON scalar equality; compiler does not type-check comparability. |
| Inequality `!=` | [x] | [~] | [ ] | [~] | [~] | [ ] | Implemented with same limitations as equality; needs explicit tests. |
| Boolean `&&` | [x] | [ ] | [ ] | [ ] | [ ] | [ ] | Must short-circuit and carry presence proofs forward. |
| Boolean `||` | [x] | [ ] | [ ] | [ ] | [ ] | [ ] | Must short-circuit; presence proof behavior should stay conservative. |
| Boolean `!` | [x] | [ ] | [ ] | [ ] | [ ] | [ ] | Needed for `!(x == null)` proof form. |
| Ordering `< <= > >=` | [x] | [ ] | [ ] | [ ] | [ ] | [ ] | Only int, float, duration, time; string ordering remains disabled unless explicitly enabled. |
| Membership `in` / `not in` | [x] | [ ] | [ ] | [ ] | [ ] | [ ] | Arrays require item compatibility; maps require string key lookup. |
| Parentheses and precedence | [x] | [ ] | [ ] | [~] | [ ] | [ ] | Assertion splitter ignores comparisons inside parentheses, but there is no general parser. |
| Array literals | [x] | [ ] | [ ] | [ ] | [ ] | [ ] | Required for membership examples like `provider in ["codex", "claude"]`. |
| Object literals in expected schema contexts | [x] | [~] | [~] | [~] | [ ] | [x] | Record bodies exist; expression-level object literals are not general AST nodes. |
| Map index `path["key"]` | [x] | [ ] | [ ] | [ ] | [ ] | [ ] | Field paths currently support dot access only in guard/assertion runtime. |
| Enum variant values | [x] | [~] | [~] | [~] | [~] | [~] | Enum schemas exist; expression values are currently strings at runtime. |
| Literal-union values | [x] | [~] | [~] | [~] | [~] | [x] | Literal types exist; guards do not yet verify literal-domain membership. |
| Typed finite-domain pattern branches | [x] | [~] | [~] | [~] | [x] | [x] | Concrete rule-body `case` branches work for enum/literal values in `whip dev`; no typed expression AST yet. |
| Exhaustiveness checks for finite patterns | [x] | [ ] | [ ] | [ ] | [x] | [x] | Abstract model covers diagnostic on missing enum/literal branch; compiler support is not implemented. |
| Optional Some/None pattern branches | [x] | [~] | [~] | [~] | [x] | [x] | Rule-body `Some name` binds a present runtime value; static presence proof is still local to case validation. |
| Optional presence proofs | [x] | [ ] | [ ] | [ ] | [~] | [ ] | Must reject unsafe optional field access unless proven present. |
| Missing vs null distinction | [x] | [ ] | [ ] | [~] | [~] | [ ] | Runtime currently collapses missing path lookup to `null` in guards. |
| Type-directed interpolation paths | [x] | [~] | [ ] | [~] | [ ] | [x] | Existing interpolation is path-oriented but not fully expression-kernel typed. |
| Dynamic `AgentRef<...>` | [x] | [ ] | [ ] | [ ] | [~] | [ ] | Needed before plain strings can be rejected as dynamic `tell` targets. |
| Deterministic validation capability | [~] | [ ] | [ ] | [ ] | [ ] | [ ] | Still design-level; should handle checks that do not need BAML/model judgment. |

## Implementation Work Queue

### 1. Expression AST And Parser

- [ ] Add expression AST nodes for literals, paths, unary ops, binary ops,
  calls, array literals, object literals, fact queries, effect queries, and
  map indexing.
- [ ] Replace raw guard strings in the typed IR with parsed expression nodes
  while preserving source spans.
- [ ] Replace raw assertion strings in the typed IR with parsed expression nodes
  while preserving source spans.
- [ ] Implement precedence and associativity exactly as specified:
  path/indexing, unary/calls, ordering/membership, equality, `&&`, `||`.
- [ ] Parse `exists path` separately from `exists(collection)`.
- [ ] Parse `not in` as one membership operator, not as `not` plus identifier.
- [ ] Keep formatter output stable and parenthesize when precedence could be
  surprising.
- [ ] Add invalid syntax diagnostics for dangling operators, unclosed
  parentheses, malformed queries, and unsupported function names.

### 2. Type Checker

- [ ] Compute expression free bindings and verify each binding exists in the
  current rule/assertion scope.
- [ ] Resolve every field/index path against class, array, map, optional, ref,
  and projection types.
- [ ] Track result type for every expression node.
- [ ] Reject non-boolean guard/assertion results.
- [ ] Reject unknown fields and map indexes with non-string keys.
- [ ] Reject field access through optional values without an accepted presence
  proof.
- [ ] Implement presence proof tracking for:
  `x != null`, `null != x`, `exists x`, `!(x == null)`, and left-to-right
  `a && b`.
- [ ] Reject incompatible equality comparisons.
- [ ] Reject ordering on unsupported types and incompatible numeric/time types.
- [ ] Reject string ordering unless a later spec explicitly enables it.
- [ ] Reject enum variants outside their enum domain.
- [ ] Reject literal values outside literal unions.
- [ ] Reject membership against non-array and non-map operands.
- [ ] Reject array literals whose elements do not share a valid common type.
- [ ] Reject object literals outside an expected schema context.
- [ ] Reject plain strings where `AgentRef<...>` is required.
- [ ] Emit statically unsatisfiable finite-domain guard diagnostics when useful,
  for example `task.provider == "gpt5"` against `"codex" | "claude" | "pi"`.

### 3. Runtime Evaluator

- [ ] Replace ad hoc guard and assertion string evaluators with one typed
  expression evaluator.
- [ ] Preserve a strict `Missing` result distinct from `Null`.
- [ ] Implement short-circuiting `&&` and `||`.
- [ ] Implement `!`.
- [ ] Implement scalar equality and inequality over typed values.
- [ ] Implement ordering over int, float, duration, and time.
- [ ] Implement membership for arrays and map keys.
- [ ] Implement `exists path`, `exists(collection)`, `empty(...)`, and
  `count(...)`.
- [ ] Implement fact and effect projection reads over typed query filters.
- [ ] Ensure guard `false` means non-match and guard `Error` means no rule
  commit plus diagnostic.
- [ ] Ensure assertion `false` or `Error` cannot mutate facts/effects and
  produces structured failure output.
- [ ] Add deterministic JSON output for assertion actual/expected values and
  failure reasons.

### 4. IR, Lowering, And Generated Checks

- [ ] Add typed expression IR with source spans and stable snapshot rendering.
- [ ] Lower guard expressions into rule readiness predicates.
- [ ] Lower assertion expressions into deterministic checkpoint metadata.
- [ ] Lower fact/effect projection reads into rule/assertion read metadata.
- [ ] Extend generated per-program Maude checks so a rule can commit only after
  its lowered guard predicate is true.
- [ ] Generate Maude coverage for false/error guard cases and assertion failure
  non-mutation.
- [ ] Ensure generated checks preserve existing effect-graph dependency checks.

### 5. AgentRef And Deterministic Routing

- [ ] Specify source syntax for `AgentRef<codex | claude | pi>` or its chosen
  equivalent in [type-system.md](type-system.md) and
  [language.md](language.md).
- [ ] Represent `AgentRef` structurally in IR.
- [ ] Type-check every possible agent target against declared agents.
- [ ] Type-check target profile, capacity, and capability constraints.
- [ ] Reject dynamic `tell` targets that are plain strings.
- [ ] Evaluate dynamic target expressions deterministically at rule commit.
- [ ] Add provider-language dogfood coverage that routes through typed
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
- [ ] Emit exhaustiveness diagnostics for finite domains where the branch result
  must be total.
- [~] Preserve source spans for branch alternatives in diagnostics.
- [~] Lower matching pattern branches before effect graph commit in `whip dev`.
- [x] Model finite-domain branch match/non-match in Maude.
- [x] Model exhaustive finite-domain miss diagnostics in Maude.
- [x] Model optional Some/None branch readiness and present binding in Maude.
- [ ] Defer deep object destructuring, array destructuring, user-defined
  extractors, and provider-text pattern matching until a concrete workflow
  requires them.

### 7. Tests And Fixtures

- [ ] Parser tests for every expression form.
- [ ] Golden IR snapshots for guards and assertions using every operator class.
- [ ] Static-analysis tests for unknown bindings, unknown fields, optional
  misuse, invalid enums, invalid literal values, bad membership, bad ordering,
  bad array literals, and bad `AgentRef` targets.
- [ ] Runtime tests for guard true, false, and error paths.
- [ ] Runtime tests for assertion pass, fail, and error paths.
- [~] Parser/type-checker tests for enum, literal, optional, and tagged-union
  pattern branches.
- [ ] Exhaustiveness diagnostic tests for finite pattern domains.
- [ ] E2E test showing `&&`, `||`, `!`, ordering, `in`, `exists`, `empty`, and
  `count` in source.
- [ ] E2E test showing assertion failures reach JSON output and nonzero exit.
- [ ] E2E test showing failed guards do not enqueue effects.
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
- [ ] Update [type-system.md](type-system.md) with final `AgentRef` syntax and
  JSON/IR representation.
- [ ] Update [companion-skill.md](companion-skill.md) to recommend deterministic
  routing metadata, `AgentRef`, source assertions, and projection checks.
- [ ] Update [e2e.md](e2e.md) with expression-kernel dogfood coverage.
- [ ] Keep this tracker and [implementation-plan.md](implementation-plan.md)
  synchronized as features land.

## Acceptance Gates

- [ ] A shared task schema can route work across Codex, Claude, and Pi using
  deterministic source metadata without duplicate provider-specific classes.
- [ ] Guards using enum/literal fields are type-checked and finite-domain typos
  are rejected or diagnosed before runtime.
- [ ] Boolean, ordering, membership, presence, count, empty, and exists
  expressions work in both guards and assertions where semantically valid.
- [ ] Enum/literal, optional, and tagged-union pattern branches are typed,
  deterministic, and exhaustiveness-checked where the domain is finite.
- [ ] Optional field access is rejected unless presence is proven.
- [ ] Dynamic agent routing is typed as `AgentRef` or equivalent, and plain
  strings cannot target `tell`.
- [ ] Assertion failures are visible in CLI JSON/human output, event or
  diagnostic surfaces, and CI exit status without mutating workflow state.
- [ ] Generated Maude checks include guard-gated rule commits and assertion
  non-mutation cases.
- [ ] Companion-skill-authored workflows use deterministic routing and source
  assertions without prompt-level provider/model decisions.
