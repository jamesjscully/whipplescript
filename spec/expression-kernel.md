# Expression Kernel

Status: draft

The expression kernel is the small pure language used by guards, assertions,
interpolation, and typed construction. It is deliberately not a general-purpose
programming language. Its job is to let rules select facts, route work, and
check deterministic properties without moving orchestration decisions into
prompts or model outputs.

## Scope

The same expression semantics apply in:

```text
when ... where <expr>
assert <expr>
record field values
effect arguments
string interpolation paths
static matrix rows
static action/template parameters
```

Expressions may inspect matched facts, effect projections, explicit action
parameters, literals, and typed constants. Expressions may not perform I/O,
enqueue effects, call providers, invoke BAML, call plugins, read clocks, use
randomness, mutate facts, or run host-language code.

## Value Domain

The evaluator operates over typed values:

```text
Null
Bool
Int
Float
String
Duration
Time
EnumVariant
Literal
OpaqueId
AgentRef
Array<T>
Map<V>
Object<Class>
Missing
Error
```

`Missing` is an internal evaluation result for absent optional fields or map
keys. It is not a source value and cannot be stored. `Error` is an internal
compile/evaluation failure. A well-typed program should not produce `Error`
during deterministic stepping; if it does, the rule commit fails before any
partial state is written.

Opaque media and external objects are compared only by identity unless their
schemas expose ordinary fields.

## Expression Forms

Source grammar should stay close to this shape:

```text
expr
  = literal
  | enumVariant
  | path
  | "(" expr ")"
  | "!" expr
  | expr "&&" expr
  | expr "||" expr
  | expr "==" expr
  | expr "!=" expr
  | expr "<" expr
  | expr "<=" expr
  | expr ">" expr
  | expr ">=" expr
  | expr "in" expr
  | expr "not" "in" expr
  | "exists" path
  | "exists" "(" collection ")"
  | "empty" "(" expr ")"
  | "empty" "(" collection ")"
  | "count" "(" collection ")"
  | caseExpr
  | objectLiteral
  | arrayLiteral

path
  = binding ("." field | "[" stringLiteral "]")*

collection
  = path
  | factQuery
  | effectQuery

factQuery
  = ClassName ("where" expr)?

effectQuery
  = "effect" ("kind" EffectKind)? ("where" expr)?

caseExpr
  = "case" expr "{" caseBranch+ "}"

caseBranch
  = pattern ("where" expr)? "=>" expr
  | pattern ("where" expr)? "=>" "{" ruleBody "}"
```

The implementation may choose different concrete syntax, but it must preserve
these semantic categories.

Operator precedence, from tightest to loosest:

```text
path access, indexing
! exists empty count
< <= > >= in not in
== !=
&&
||
```

Parentheses are always allowed and should be preferred in diagnostics and
formatter output when precedence could surprise an author.

## Static Typing

Every expression is typed before runtime evaluation. The compiler must reject:

- unknown bindings or fields
- field access through an optional value without a proven presence guard
- comparing incompatible types
- ordering unsupported types
- enum variants outside the enum
- literal values outside a literal union
- membership against non-arrays and non-maps
- patterns that cannot match the scrutinee type
- unknown enum variants or literal values in finite-domain patterns
- non-exhaustive finite-domain patterns where a total expression is required
- object literals that do not satisfy the expected class/effect schema
- arrays whose elements do not share the declared item type
- plain strings used as `AgentRef` or dynamic `tell` targets

No implicit stringification is allowed except inside string interpolation.
Interpolation accepts only paths to simple scalar values, enums, literals,
opaque ids, and agent refs.

## Truth And Errors

Guards and assertions require a boolean result.

```text
true     -> selected / assertion passes
false    -> not selected / assertion fails
Missing  -> false for `exists`, otherwise compile-time rejected or runtime error
Error    -> diagnostic; no rule commit
```

Boolean operators are short-circuiting and deterministic:

```text
false && x -> false
true  && x -> x
true  || x -> true
false || x -> x
```

Short-circuiting may avoid evaluating a path that would otherwise be invalid,
but v0 should rely on explicit presence proofs rather than clever boolean
reasoning.

## Presence Proofs

Optional field access must be guarded by an obvious proof:

```whippletree
when issue.assignee != null
when issue.assignee.name == "Ada"
```

Accepted proof forms:

```text
x != null
null != x
exists x
!(x == null)
a && b, where either side proves presence for later conjuncts
pattern/case branch that binds a non-null variant
```

Rejected in v0:

```text
equivalent algebraic rewrites
presence hidden behind user-defined functions
presence inferred from string parsing
presence inferred from provider/model text
```

## Pattern Matching And Branching

Pattern matching is a typed finite-domain branching tool, not a general
destructuring language. It exists to keep deterministic routing and variant
handling out of prompts.

The v0 pattern surface should cover:

```text
enum variants
literal-union values
optional Some/None or present/missing branches
tagged terminal-output unions from effect completions
wildcard/default branch, when an explicit catch-all is intended
```

Patterns may have guards, and those guards use the ordinary expression kernel.
Branch guards must not perform I/O or call providers.

The compiler should exhaustiveness-check finite domains when the branch result
must be total:

```whippletree
case review.status {
  Accept => "ready"
  Revise => "needs-work"
  Blocked => "blocked"
}
```

Optional branches may bind a proven-present value:

```whippletree
case issue.assignee {
  Some assignee => assignee.name
  None => "unassigned"
}
```

The initial implemented rule-body form is:

```whippletree
case task.provider {
  "codex" => {
    record RoutedTask {
      provider task.provider
    }
  }
}
```

Exact expression-level syntax may still change, but the semantics must stay
restricted:

- enum/literal patterns can match only variants in the declared domain
- optional `Some` branches prove presence for the bound value
- `None` branches do not permit reading fields through the missing value
- branch selection is deterministic and side-effect-free
- non-matching branches do not commit facts or effects
- exhaustive finite-domain misses produce diagnostics instead of hidden fallthrough

Deferred until a concrete workflow requires them:

```text
deep object destructuring
array/list destructuring
user-defined extractors
regex/string pattern matching
provider transcript pattern matching
```

## Equality

Equality is structural for pure data:

```text
bool, int, float, string
duration, time
enum variants
literal values
null
opaque identity values
agent refs
arrays and objects only when every nested field is equality-comparable
```

`int` and `float` may compare numerically when lossless and well-defined. NaN
and infinities are not valid Whippletree numeric values.

Class/object equality is not a routing primitive unless the class is explicitly
marked comparable or the comparison is over known identity fields. Prefer field
comparisons in guards.

## Ordering

Ordering is defined only for:

```text
int
float
duration
time
string, only if explicitly enabled for lexicographic ordering
```

String ordering is not needed for the provider-routing dogfood path and should
remain disabled unless a concrete workflow requires it. Equality on strings is
allowed.

Ordering across incompatible numeric/time types is rejected.

## Membership

Membership is deterministic and type-checked:

```whippletree
task.provider in ["codex", "claude", "pi"]
"repo-writer" in agent.profiles
"owner" in issue.labels
"priority" in issue.metadata
```

For arrays, the left-hand type must match the item type. For maps, membership
checks key existence and the left-hand side must be a string. Membership does
not search object fields or perform substring checks.

## Count And Empty

`count` and `empty` are available for arrays, maps, fact queries, and effect
queries:

```whippletree
count(LanguageE2EResult where provider == "codex") == 2
empty(BlockingIssue where severity == Critical)
```

They do not iterate over arbitrary runtime streams. Fact/effect queries are
projection reads at an assertion/checkpoint boundary.

## Object And Array Construction

Object literals are allowed only where an expected schema is known:

```whippletree
record LanguageTask {
  provider "codex"
  language "French"
}
```

Static matrix rows are object construction with an explicit target class.

Array literals are allowed when the target item type is known or all elements
have an obvious common scalar/literal/enum type. Arrays are immutable values;
`append` and `remove` are source-level update operations over small workflow
facts, not general collection programming.

## String Interpolation

Interpolation is not string programming. It is a formatting operation over
already-typed values:

```whippletree
"""
Implement {{ issue.title }} for {{ issue.owner }}.
"""
```

Allowed interpolation operands:

```text
string
int
float
bool
duration
time
enum
literal
opaque id
AgentRef
```

Interpolating objects, arrays, maps, media values, missing values, or provider
transcripts is rejected unless the target effect/capability schema explicitly
defines a serialization format. Interpolation must not be used as a hidden data
transport between rules; use typed facts/effect arguments for that.

## Small Fact Updates

The kernel may support small deterministic updates over workflow-owned facts:

```whippletree
append item.tags "needs-review"
remove item.blockers blockerId
```

These are not loops or collection transforms. They lower to one typed fact
replacement or append-only update event, with the same schema validation as
`record`. They are valid only for small workflow facts owned by the program, not
for provider-owned projections such as external issue payloads or agent turn
transcripts.

## Agent References

`AgentRef<...>` is the only dynamic agent-target type:

```whippletree
AgentRef<codex | claude | pi>
```

The compiler must prove that every possible value names a declared agent and
that each target satisfies profile/capacity constraints required by the `tell`.
A plain `string` never coerces to `AgentRef`.

## Assertions

Assertions are deterministic checks over projected state:

```whippletree
assert count(LanguageE2EResult where provider == "codex") == 2
assert exists(LanguageE2EResult where language == "Japanese")
assert count(effect kind agent.tell where status == completed) == 6
```

Assertions do not create user facts or effects. The runtime may record assertion
results as diagnostics/evidence for CI and debugging.

## Maude Modeling Target

The first Maude expression model should be abstract and finite. It does not need
to encode strings or JSON. It should model:

```text
typed values: bool, null, enum/literal symbols, scalar comparable placeholders
presence: present/missing
paths: successful lookup, missing optional, invalid path
guards: true/false/error
rule readiness: fact match + guard true
assertions: pass/fail/error
agent refs: declared target vs invalid target
```

Recommended Maude sorts:

```maude
sorts Value Type Expr Path GuardResult AssertionResult AgentTarget AssertionId .
ops vTrue vFalse vNull vMissing vError : -> Value .
ops gTrue gFalse gError : -> GuardResult .
ops aPass aFail aError : -> AssertionResult .
```

Recommended abstract predicates:

```maude
op guard : RuleId FactId GuardResult -> Cfg .
op assertion : AssertionId AssertionResult -> Cfg .
op declaredAgent : AgentTarget -> Cfg .
op tellTarget : EffectId AgentTarget -> Cfg .
op presence : Path Presence -> Cfg .
```

Initial Maude safety searches:

- a rule with a false guard cannot fire
- a rule with an error guard cannot commit facts/effects
- a rule with a true guard can fire when its fact match is present
- an optional field cannot be read after a missing presence proof
- an enum/literal guard cannot be true for a value outside the declared domain
- a dynamic `tell` cannot enqueue an effect for an undeclared agent
- an assertion failure cannot mutate workflow facts/effects
- expression evaluation is deterministic for the same fact projection

The generated per-program Maude path should lower guards to abstract predicates
over matched fact symbols before lowering rule commits. This keeps the existing
effect-graph model intact while adding a readiness gate:

```text
fact(F) + guard(R, F, true)  -> ruleReady(R, F, G)
fact(F) + guard(R, F, false) -> no rewrite
fact(F) + guard(R, F, error) -> diagnostic, no graph
```

Do not model BAML/model semantic truth in Maude. Model only the typed contract:
a `coerce` effect may complete with a value that satisfies its output type, fail,
time out, or be cancelled.
