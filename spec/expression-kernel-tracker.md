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
| Full guard/assertion type checking | [x] | [x] | [x] | [x] | Assertions and guards now reuse shared expression validation for free binding/path checks, result typing, boolean result enforcement, incompatible equality, membership, array common-type checks, finite-domain contradictions, function arity/shape checks, and query-field/schema diagnostics. |
| Expression object literals | [x] | [x] | [~] | [x] | Expression AST object literals now parse/evaluate, record-field expected-schema object/map literals support multiline nested bodies, and BAML effect arguments validate against declared parameter types. Anonymous object literals are rejected in untyped guard/assertion contexts. Remaining gaps: payload schemas for effect kinds that still expose no structured source args and broader typed rule-body lowering. |
| Duration/time ordering | [x] | [x] | [x] | [x] | Parser type checking now accepts same-type duration/time ordering, validates ISO-8601 duration and RFC3339 time source literals, accepts fractional seconds and timezone offsets, and CLI evaluator orders parsed duration/time string values. Invalid external duration/time facts now surface deterministic guard/assertion errors where IR field type context is available. Calendar months/years remain unsupported. |
| Enum/literal finite-domain typing | [x] | [x] | [x] | [x] | Parser inference now preserves enum, literal-union, and AgentRef finite domains; guards/assertions/query filters reject out-of-domain literals symmetrically and diagnose useful unsatisfiable finite-domain equality/membership/exclusion cases. Runtime still stores these values as JSON scalars. |
| Generated per-program Maude checks | [x] | [x] | [x] | [x] | Generated searches now lower the currently-supported expression subset into Maude terms for guard true/false/error and assertion non-mutation: scalar equality, boolean `&&`/`||`/`!`, count equality/ordering through 0/1/2/3/many thresholds, `exists(...)`, `empty(...)`, ordering witnesses, membership, exact finite array/object cardinality and membership/index reads, map/object index misses, filtered-query cardinality, and terminal-output tag branches. Effect dependency checks are preserved. Remaining gaps are outside the current supported generator subset: exact runtime map values for non-literal maps, deep path-sensitive query filtering over concrete row sets, and exact counts above the `many` bucket. |
| AgentRef profile/capacity/capability constraints | [x] | [x] | [x] | [x] | Compiled program versions now persist declared agents, including capabilities. Parser checks `tell ... requires [...]` against static and dynamic `AgentRef` domains; store/kernel policy rejects undeclared targets, profile mismatches, capability mismatches, and capacity exhaustion before provider start. |
| Tagged terminal-output union branch matching | [x] | [x] | [x] | [x] | `after ... completes` now exposes a deterministic terminal union in the dev stepper, validates `Completed`/`Failed`/`TimedOut`/`Cancelled` branch tags, snapshots explicit typed IR payload alternatives/branches, and covers completed/failed/timed-out/cancelled BAML routing with branch-local record and effect commits. Runtime branch guard errors are reported without selecting sibling branches. |
| Branch pattern spans and typed lowering | [x] | [x] | [x] | [x] | Ordinary rule-body `case` branches now lower typed IR metadata with scrutinee type, pattern kind, guard expression, branch body hash, and branch pattern span. Parser diagnostics point at branch alternatives for invalid enum/literal/optional patterns, and `whip dev` selects case branches before committing branch-local records/effects, including nested terminal case bodies. Runtime reports guard errors and no-match cases without selecting sibling branches. |
| Assertion diagnostics and event surfaces | [x] | [x] | [x] | [x] | Failed/error assertions now persist deterministic diagnostic rows during `dev` with assertion source spans; CLI JSON/human output, exit status, and `whip diagnostics` list the durable surface after store reopen. |
| Provider/harness failure capture | [x] | [x] | [x] | [x] | Failed/timed-out agent, BAML, and Loft provider runs now persist diagnostic rows linked to terminal event, effect, run, provider evidence, and source span metadata when the effect came from compiled source. Terminal events embed the diagnostic payload so provider diagnostics can be reconstructed from event replay. |
| Parser-only expression matrix | [x] | [x] | [x] | [x] | Parser-only tests now cover precedence, calls, fact/effect queries, map indexes, arrays, invalid syntax, and optional presence-proof syntax. |
| Golden IR expression fixture | [x] | [x] | [x] | [x] | Added the expression-kernel validation fixture covering guards, assertions, projections, maps, arrays, optional presence, and deterministic routing. |
| Static-analysis diagnostic matrix | [x] | [x] | [x] | [x] | Parser tests now cover assertion validation, unknown roots/fields, non-boolean guards/assertions, incompatible equality, membership misuse, bad array literals, finite-domain typos/contradictions, quoted AgentRef misuse, expression function/query errors, bad effect payloads, terminal-output tags, dynamic `AgentRef` capability checks, and duration/time literal errors. |
| Companion-skill validation cleanup | [x] | [x] | [x] | [x] | The companion-skill fixture and CLI e2e coverage now validate typed `AgentRef` routing, tracker-path prompts, source assertions, and no LLM provider/model classification. |

### Completed Implementation Slices

| Slice | Files | Result |
| --- | --- | --- |
| Parser/type-checker validation | `crates/whipplescript-parser/src/lib.rs` | Assertion validation, symmetric finite-domain diagnostics, unknown dotted-root diagnostics, simple fact-query guard typing, and parser-only expression matrix tests landed. |
| Full guard/assertion type-checking closure | `crates/whipplescript-parser/src/lib.rs`, checked example fixtures, `spec/expression-kernel-tracker.md` | Expression inference now preserves finite domains and AgentRefs, rejects non-boolean/incompatible/membership/array errors in guards and assertions, rejects quoted AgentRef record values, and diagnoses unsatisfiable finite-domain relations. |
| Generated Maude validation | `crates/whipplescript-cli/src/main.rs`, `models/maude/kernel.maude` | Guard-gated rule and assertion non-mutation searches now use generated expression terms for equality, boolean connectives, and query count/exists/empty while preserving dependency checks. |
| Broadened generated Maude expressions | `crates/whipplescript-cli/src/main.rs`, `models/maude/kernel.maude`, `models/maude/tests/expression-kernel.maude` | Generated model searches now include abstract Maude terms for ordering, membership, array/object/map collection witnesses, index misses, and query filters, with hand-written Maude tests for true/false/error behavior. |
| Store diagnostics | `crates/whipplescript-store/src/lib.rs`, `crates/whipplescript-store/migrations/0001_runtime_store.sql` | Durable diagnostic record/list APIs, schema columns, idempotency indexes, and existing-store upgrade coverage landed. |
| Golden validation fixture | expression-kernel example fixture | Compiled golden fixture landed for guards, assertions, projections, map indexes, optional presence, arrays, and deterministic routing. |
| Object/map literal construction | `crates/whipplescript-parser/src/lib.rs`, `crates/whipplescript-cli/src/main.rs`, `crates/whipplescript-cli/tests/control_plane.rs`, expression-kernel example fixture | Record-field map/object literals now validate and materialize, including multiline nested bodies and BAML object/map effect arguments. |
| Effect payload validation | `crates/whipplescript-parser/src/lib.rs`, `crates/whipplescript-cli/tests/control_plane.rs`, `examples/invalid/bad-effect-payload.*` | `coerce` arguments are checked against declared parameter types, multiline object/map arguments are validated deterministically, `claim ... with loft` requires a `LoftIssue`, and anonymous object literals are rejected when no expected type exists. |
| Duration/time ordering | `crates/whipplescript-parser/src/lib.rs`, `crates/whipplescript-cli/src/main.rs`, `crates/whipplescript-cli/tests/control_plane.rs` | Duration/time fields are no longer treated as strings for ordering validation; source literals are validated with fractional timestamp support; runtime ordering parses typed duration/time strings and reports typed errors for invalid external values. CLI tests cover check/dev/invalid-literal/external-invalid behavior. |
| Function/query validation fixture | `examples/invalid/bad-expression-functions.*`, `crates/whipplescript-parser/src/lib.rs` | Invalid function/query diagnostics are now enforced and included in invalid fixture discovery. |
| Provider diagnostics trace slice | `crates/whipplescript-kernel/src/trace.rs`, `crates/whipplescript-kernel/src/lib.rs`, `crates/whipplescript-cli/src/main.rs` | Provider diagnostics now appear in kernel trace and CLI trace JSON before terminal events. |
| Durable runtime diagnostics | `crates/whipplescript-store/src/lib.rs`, `crates/whipplescript-kernel/src/lib.rs`, `crates/whipplescript-cli/src/main.rs`, `crates/whipplescript-cli/tests/control_plane.rs` | Provider/harness failures and assertion failures are recorded through the store diagnostics API and listed through `whip diagnostics` after reopening the SQLite store. Terminal provider diagnostics are also embedded in `effect.terminal` events and covered by replay-derived store tests. |
| AgentRef store enforcement | `crates/whipplescript-store/src/lib.rs`, `crates/whipplescript-kernel/src/lib.rs`, `crates/whipplescript-cli/src/main.rs` | Real compiled `agent` declarations are persisted to program versions; claimability/start-run enforce declared target/profile/capability/capacity metadata; capacity blocks now emit `effect.blocked` and set `blocked_by_capacity`. |
| Tagged terminal checklist | `spec/expression-kernel-tracker.md` | Added implementation checklist for tagged terminal-output union branch matching. |
| Case branch context safety | `crates/whipplescript-cli/src/main.rs`, `crates/whipplescript-cli/tests/control_plane.rs` | Failed guarded `Some` branches no longer leak payload bindings into later branches or fallbacks. |
| Tagged terminal branch runtime | `crates/whipplescript-parser/src/lib.rs`, `crates/whipplescript-cli/src/main.rs`, `crates/whipplescript-cli/tests/control_plane.rs` | Parser validates terminal-output tags inside `after ... completes`; runtime binds completed/failed terminal payload variants for deterministic case routing. |
| Companion skill validation | `skills/whipplescript-author/SKILL.md`, `spec/companion-skill.md`, companion-skill example fixture, `crates/whipplescript-cli/tests/control_plane.rs` | Companion-skill workflow authored with one shared task schema, typed `AgentRef` reviewers, source assertions, and fixture-provider e2e coverage. |
| Assertion JSON detail surface | `crates/whipplescript-cli/src/main.rs`, `crates/whipplescript-cli/tests/control_plane.rs` | `whip dev --json` assertion reports now include deterministic expected predicate metadata, evaluated actual operand values, and failure reasons for failed/error assertions. |

## Current Implementation Summary

- [x] Source accepts guarded fact matches such as
  `when LanguageTask as task where task.provider == "codex"`.
- [x] Runtime `whip dev` evaluates simple guard equality/inequality.
- [x] Source accepts top-level `assert` statements.
- [x] Runtime `whip dev` evaluates assertion reports over fact/effect
  projections and exits nonzero on assertion failure.
- [x] Provider-language validation asserts provider counts, agent-turn counts, and
  BAML coerce counts in source.
- [x] Companion-skill validation asserts deterministic phase-review dispatch counts
  in source without asking providers or BAML to identify route/provider/model
  identity.
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
- [~] Full guard/assertion type checking is implemented for the current shared
  expression subset, with remaining gaps tracked in the type-checker queue.

## Feature Matrix

| Feature | Spec | Parser | Type Check | Runtime Eval | Maude | Tests | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Guarded fact match `when X as x where expr` | [x] | [x] | [~] | [x] | [~] | [x] | Guards now use the shared expression parser/evaluator; static typing remains partial. |
| Top-level `assert expr` | [x] | [x] | [~] | [x] | [~] | [x] | Assertions parse through the shared expression parser and run after `dev`; typed assertion scopes remain partial. |
| Fact projection query `Class where expr` | [x] | [x] | [~] | [x] | [~] | [x] | Parsed as expression query nodes and evaluated in assertions; generated Maude preserves filter true/false/error and 0/1/2/3/many cardinality buckets. Full concrete row-set filtering remains outside the current generated model. |
| Effect projection query `effect kind K where expr` | [x] | [x] | [~] | [x] | [~] | [x] | Parsed as expression query nodes and evaluated in assertions; generated Maude preserves filter true/false/error and 0/1/2/3/many cardinality buckets. Full concrete row-set filtering remains outside the current generated model. |
| `count(collection)` | [x] | [x] | [~] | [x] | [~] | [x] | Implemented over fact/effect projection queries; generated Maude now covers exact finite array/object literal cardinality plus query count thresholds through 0/1/2/3/many. |
| `exists(collection)` | [x] | [x] | [~] | [x] | [~] | [x] | Implemented over projection queries; generated Maude covers query/array/map/object presence witnesses. |
| `exists path` presence proof | [x] | [x] | [~] | [x] | [~] | [x] | Runtime checks non-missing/non-null; local parser proof tracking accepts `exists x` before optional field access. |
| `empty(collection)` / `empty(expr)` | [x] | [x] | [~] | [x] | [~] | [x] | Implemented for projections, arrays, objects, strings, and null; generated Maude covers query/array/map/object emptiness witnesses. |
| Equality `==` | [x] | [x] | [~] | [x] | [~] | [x] | Runtime supports JSON scalar equality; compiler rejects obvious incompatible scalar comparisons and finite-domain typos. |
| Inequality `!=` | [x] | [x] | [~] | [x] | [~] | [x] | Implemented with same static limitations as equality. |
| Boolean `&&` | [x] | [x] | [~] | [x] | [~] | [x] | Runtime short-circuits; parser carries presence proofs left-to-right; generated Maude models abstract conjunction truth/error paths. |
| Boolean `||` | [x] | [x] | [~] | [x] | [~] | [x] | Runtime short-circuits; presence-proof behavior remains conservative; generated Maude models abstract disjunction truth/error paths. |
| Boolean `!` | [x] | [x] | [~] | [x] | [~] | [x] | Implemented in shared expression parser/evaluator and generated Maude expression terms. |
| Ordering `< <= > >=` | [x] | [x] | [~] | [x] | [~] | [x] | Runtime supports numeric/duration/time ordering; generated Maude has abstract order and count-order witnesses. |
| Membership `in` / `not in` | [x] | [x] | [~] | [x] | [~] | [x] | Runtime supports array membership and map key membership; compiler rejects obvious non-array/non-map membership and incompatible item/key types. |
| Parentheses and precedence | [x] | [x] | [~] | [x] | [ ] | [x] | Shared recursive-descent parser handles precedence. |
| Array literals | [x] | [x] | [~] | [x] | [x] | [x] | Implemented for expression evaluation; generated Maude now represents finite literal arrays with exact cardinality and membership. |
| Object literals in expected schema contexts | [x] | [~] | [~] | [~] | [x] | [x] | General expression AST object literals parse/evaluate; generated Maude now represents finite literal objects with exact key cardinality, membership, and index reads. |
| Map index `path["key"]` | [x] | [x] | [~] | [x] | [~] | [x] | String-key map indexing works in guards/assertions; generated Maude models literal object index reads exactly and keeps abstract witnesses for runtime map paths. |
| Enum variant values | [x] | [~] | [~] | [~] | [~] | [~] | Enum schemas exist; expression values are currently strings at runtime. |
| Literal-union values | [x] | [x] | [~] | [x] | [~] | [x] | Literal-union guards, assertions, query filters, membership arrays, and record values verify domain membership before runtime. |
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

- [x] Add expression AST nodes for literals, paths, unary ops, binary ops,
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

- [x] Compute expression free bindings and verify each binding exists in the
  current rule/assertion scope.
- [~] Resolve every field/index path against class, array, map, optional, ref,
  and projection types.
- [x] Track result type for every expression node in the shared guard/assertion
  subset.
- [x] Reject non-boolean guard/assertion results.
- [x] Reject unknown fields and map indexes with non-string keys.
- [x] Reject field access through optional values without an accepted presence
  proof.
- [x] Implement presence proof tracking for:
  `x != null`, `null != x`, `exists x`, `!(x == null)`, and left-to-right
  `a && b`.
- [x] Reject incompatible equality comparisons.
- [x] Reject ordering on unsupported types and incompatible numeric/time types.
- [x] Reject string ordering unless a later spec explicitly enables it.
- [x] Reject enum variants outside their enum domain.
- [x] Reject literal values outside literal unions.
- [x] Reject membership against non-array and non-map operands.
- [x] Reject array literals whose elements do not share a valid common type.
- [x] Reject object literals outside an expected schema context.
- [x] Reject plain strings where `AgentRef<...>` is required.
- [x] Emit statically unsatisfiable finite-domain guard diagnostics when useful,
  for example `task.provider == "gpt5"` against `"codex" | "claude" | "pi"`.

### 3. Runtime Evaluator

- [x] Replace ad hoc guard and assertion string evaluators with one typed
  expression evaluator.
- [x] Preserve a strict `Missing` result distinct from `Null`.
- [x] Implement short-circuiting `&&` and `||`.
- [x] Implement `!`.
- [x] Implement scalar equality and inequality over typed values.
- [x] Implement ordering over int, float, duration, and time.
- [x] Implement membership for arrays and map keys.
- [x] Implement `exists path`, `exists(collection)`, `empty(...)`, and
  `count(...)`.
- [x] Implement fact and effect projection reads over typed query filters.
- [x] Ensure guard `false` means non-match and guard `Error` means no rule
  commit plus diagnostic.
- [x] Ensure assertion `false` or `Error` cannot mutate facts/effects and
  produces structured failure output.
- [x] Add deterministic JSON output for assertion actual/expected values and
  failure reasons.

### 4. IR, Lowering, And Generated Checks

- [x] Add typed expression IR with source spans and stable snapshot rendering.
- [x] Lower guard expressions into rule readiness predicates.
- [x] Lower assertion expressions into deterministic checkpoint metadata.
- [x] Lower fact/effect projection reads into rule/assertion read metadata.
- [x] Extend generated per-program Maude checks so a rule can commit only after
  its lowered guard predicate is true.
- [x] Generate Maude coverage for false/error guard cases and assertion failure
  non-mutation.
- [x] Ensure generated checks preserve existing effect-graph dependency checks.

### 5. AgentRef And Deterministic Routing

- [x] Specify source syntax for `AgentRef<codex | claude | pi>` or its chosen
  equivalent in [type-system.md](type-system.md) and
  [language.md](language.md).
- [x] Represent `AgentRef` structurally in IR.
- [x] Type-check every possible agent target against declared agents.
- [x] Type-check target profile, capacity, and capability constraints.
- [x] Reject dynamic `tell` targets that are plain strings.
- [x] Evaluate dynamic target expressions deterministically at rule commit.
- [x] Add provider-language validation coverage that routes through typed
  `AgentRef` once available.

### 6. Pattern Matching And Branching

- [x] Specify concrete source syntax for finite-domain branching:
  `case expr { Pattern => { ... } }`.
- [x] Support enum and literal-union branch matching in rule bodies.
- [x] Support optional Some/None branch matching that binds a
  proven-present value in the Some branch.
- [~] Support tagged terminal-output union branch matching for effect
  completion facts.
- [x] Allow branch guards that reuse the current deterministic guard evaluator.
- [x] Reject unknown enum/literal variants in patterns.
- [x] Emit exhaustiveness diagnostics for finite domains where the branch result
  must be total.
- [x] Preserve source spans for branch alternatives in diagnostics.
- [x] Lower matching pattern branches before effect graph commit in `whip dev`.
- [x] Model finite-domain branch match/non-match in Maude.
- [x] Model exhaustive finite-domain miss diagnostics in Maude.
- [x] Model optional Some/None branch readiness and present binding in Maude.
- [ ] Defer deep object destructuring, array destructuring, user-defined
  extractors, and provider-text pattern matching until a concrete workflow
  requires them.

Tagged terminal-output union branch matching implementation checklist:

- [x] Source syntax: accept `case <effectBinding> { <Tag> <name> [where
  <expr>] => { ... } }` in `after <effectBinding> completes` bodies, using the
  documented terminal tags `Completed`, `Failed`, `TimedOut`, and `Cancelled`; keep
  plain status strings and provider transcript text out of the pattern syntax.
- [x] Source binding rules: require each tagged branch to bind a payload name;
  inside the branch, the payload has only the fields declared for that terminal
  tag, while the outer effect binding remains available for common metadata.
- [x] IR shape: represent the `after ... completes` binding as a typed
  terminal-output union whose alternatives carry `{ tag, payloadType,
  sourceSpan }`, and represent each branch as `{ tag, binding, guardExpr,
  body, patternSpan }` rather than lowering tags to ad hoc string comparisons.
- [x] CLI lowering behavior: lower tagged branches before effect graph commit
  in `whip dev`; evaluate the terminal tag match first, then the branch guard
  with the tag-refined payload binding, and commit only the selected branch's
  facts/effects.
- [~] Parser diagnostics: reject tagged-terminal patterns outside typed
  `after ... completes` scopes, unknown terminal tags, duplicate tags,
  branches without payload bindings, field reads invalid for the refined tag,
  non-boolean branch guards, and non-exhaustive total matches without wildcard
  or default coverage. Typed payload field reads now reject mismatches such as
  reading failure-only fields from a `Completed` payload.
- [x] Runtime branch selection: select exactly one branch for each terminal
  output by tag plus guard; treat non-matching guards as branch misses; surface
  multiple-match/no-match cases through static coverage checks, and guard
  evaluation errors as structured dev-step diagnostics.
- [~] Validation matrix: add parser/type-checker cases for accepted
  `Completed`/`Failed`/`TimedOut`/`Cancelled` branches, guarded branches, unknown tags,
  duplicate tags, wrong-scope patterns, invalid payload fields, and missing
  payload bindings.
- [x] IR snapshots: add a golden fixture showing terminal-output union
  alternatives and branch-level source spans for `after ... completes`.
- [x] Runtime/e2e acceptance: add an e2e workflow where a completed turn records
  an artifact, a failed turn records provider failure, a blocked turn asks for
  human action, and an unmatched or erroring branch produces a deterministic
  diagnostic without committing sibling branch effects.
- [x] Maude acceptance: extend the existing finite-domain branch model with
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
- [~] Golden IR and e2e tests for tagged terminal-output union branches over
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
- [x] Companion-skill validation test that authors deterministic routing without
  asking an LLM to identify provider/model identity.

### 8. Docs And Agent Guidance

- [x] Update [language.md](language.md) with concrete expression syntax once the
  AST parser lands.
- [x] Update [static-analysis.md](static-analysis.md) with the implemented
  finite-domain and presence-proof diagnostics.
- [x] Update [type-system.md](type-system.md) with final `AgentRef` syntax and
  JSON/IR representation.
- [x] Update [companion-skill.md](companion-skill.md) to recommend deterministic
  routing metadata, `AgentRef`, source assertions, and projection checks.
- [x] Update [e2e.md](e2e.md) with expression-kernel validation coverage.
- [x] Keep this tracker and [implementation-plan.md](implementation-plan.md)
  synchronized as features land.

## Acceptance Gates

- [x] A shared task schema can route work across Codex, Claude, and Pi using
  deterministic source metadata without duplicate provider-specific classes.
- [x] Guards using enum/literal fields are type-checked and finite-domain typos
  are rejected or diagnosed before runtime.
- [x] Boolean, ordering, membership, presence, count, empty, and exists
  expressions work in both guards and assertions where semantically valid.
- [~] Enum/literal, optional, and tagged-union pattern branches are typed,
  deterministic, and exhaustiveness-checked where the domain is finite.
- [x] Optional field access is rejected unless presence is proven.
- [x] Dynamic agent routing is typed as `AgentRef` or equivalent, and plain
  strings cannot target `tell`.
- [x] Assertion failures are visible in CLI JSON/human output, event or
  diagnostic surfaces, and CI exit status without mutating workflow state.
- [x] Generated Maude checks include guard-gated rule commits and assertion
  non-mutation cases.
- [x] Companion-skill-authored workflows use deterministic routing and source
  assertions without prompt-level provider/model decisions.
