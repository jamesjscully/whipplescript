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

The type checker must compute one result type for every expression node and
must preserve source spans for the node and any inferred finite-domain
constraint. Guards and assertions have no contextual result type other than
`Bool`; any guard/assertion expression whose final type is not `Bool` is a
compile-time error. Assertion fact/effect queries and guard fact bindings use
the same expression checker, so a predicate that is illegal in `when ... where`
is illegal in `assert`, and vice versa.

Expression checking is bidirectional where a context provides an expected type:

```text
record/effect field       -> declared field type
static matrix row field   -> declared row/schema field type
array element             -> target item type, if known
object literal field      -> declared field type
case branch result        -> common expected branch type, if required
guard/assertion root      -> Bool
```

Without an expected type, literals keep their narrowest useful type:

```text
"codex"        -> Literal<"codex"> and String
42             -> Int
42.0           -> Float
null           -> Null, assignable only to optional/null-accepting targets
[a, b]         -> Array<common(a, b)> only if a valid common type exists
{ ... }        -> rejected; object literals require an expected schema
```

Finite domains are first-class static types:

```text
enum Provider { Codex, Claude, Pi }        -> finite domain {Codex, Claude, Pi}
"codex" | "claude" | "pi"                 -> finite literal domain
AgentRef<codex | claude | pi>             -> finite agent domain
Optional<T>                               -> finite presence domain {Some, None}
```

Equality, inequality, membership, and finite-domain patterns must validate both
sides against the complete finite domain when either side has one. A value
outside the domain is a compile-time error even if it appears on the syntactic
side that is not currently implemented as the "field side":

```whippletree
task.provider == "gpt5"     # invalid if provider is "codex" | "claude" | "pi"
"gpt5" == task.provider     # same diagnostic
task.provider != "gpt5"     # same domain error; do not silently accept as true
"gpt5" != task.provider     # same diagnostic
```

When both sides have finite domains and their intersection is empty, the
compiler should emit an unsatisfiable-expression diagnostic. For guards this
means the guarded rule can never match; for assertions this means the assertion
can never pass unless the expression is under a short-circuited branch that the
checker can prove unreachable. The diagnostic should name both domains and the
operator, not only the invalid literal.

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

Type errors are not truthy or falsy. A malformed path, unsupported operator, or
invalid finite-domain value must be rejected before runtime. Runtime `Missing`
can arise only from data that is validly optional or from map indexing; it must
not be used to paper over an expression the compiler could have rejected.

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
a && b, where the left side proves presence for the right side
key in map, proving the exact indexed map entry exists
pattern/case branch that binds a non-null variant
```

Presence proofs are path-specific and flow-sensitive. A proof for `issue.owner`
does not prove `issue.assignee`, and a proof for `issue.assignee` does not
prove `issue.assignee.email` unless the schema says the nested field is
non-optional. The checker may normalize parenthesized expressions but should
not infer arbitrary logical equivalences.

For `&&`, proofs flow left-to-right only:

```whippletree
exists issue.assignee && issue.assignee.name == "Ada"   # accepted
issue.assignee.name == "Ada" && exists issue.assignee   # rejected
```

For `||`, a proof from one branch does not hold in the other branch or after the
operator:

```whippletree
exists issue.assignee || issue.assignee.name == "Ada"   # rejected
```

Negation only creates a presence proof for the explicit accepted form
`!(x == null)`. `!(x != null)` proves absence for exhaustiveness diagnostics but
does not permit field access. `x == null`, `null == x`, and `!exists x` prove
absence, not presence.

An `exists path` expression is well-typed for optional paths and map-index paths
that may produce `Missing`. It returns `Bool` and proves the path present for
later conjuncts in `&&` only when the path itself is syntactically named by the
proof. For optional object paths, "present" means non-missing and non-null. For
map indexes, `exists map["key"]` means the key exists and the value is non-null.
The membership proof `"key" in map` proves only that the map entry exists; it
does not prove the indexed value is non-null, but it is enough to permit
`map["key"] == null`.

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

Ordering across incompatible numeric/time types is rejected. The accepted
ordering pairs are exactly:

```text
Int      with Int
Float    with Float
Int      with Float, using numeric comparison after lossless promotion
Duration with Duration
Time     with Time
```

`Duration` values compare by normalized elapsed length. Source spellings such
as `60s` and `1m` are equal after normalization, and ordering compares the
normalized duration value, not the original unit. Calendar-relative durations
whose length depends on a start time are not valid `Duration` values in the
expression kernel.

`Time` values compare by absolute instant after timezone/offset normalization.
Naive local times without an explicit zone/offset are rejected unless their
schema declares a single canonical timezone before the value reaches the
expression kernel. `Time` never orders against `Duration`; use explicit typed
fields such as `deadline - createdAt` only if a future duration-producing
operator is specified.

## Membership

Membership is deterministic and type-checked:

```whippletree
task.provider in ["codex", "claude", "pi"]
"repo-writer" in agent.profiles
"owner" in issue.labels
"priority" in issue.metadata
```

For arrays, the left-hand type must match the item type. If the array item type
is a finite enum/literal/agent domain, the left-hand value must belong to that
domain before runtime.

For maps, membership checks key existence and the left-hand side must be a
string or string-literal type:

```whippletree
"priority" in issue.metadata
"priority" not in issue.metadata
```

Map membership never inspects values. If the map itself is present and the key
is absent, `key in map` is `false` and `key not in map` is `true`. If the map
path is optional or may be missing, the map path must first be proven present:

```whippletree
exists issue.metadata && "priority" in issue.metadata
```

Map indexing has the same key type rule:

```whippletree
issue.metadata["priority"]
```

An absent key evaluates to internal `Missing`. `exists issue.metadata["priority"]`
is `false` for an absent key and proves that exact indexed path present for the
right side of `&&`. Reading, comparing, ordering, interpolating, or passing an
indexed value that may be `Missing` is rejected unless guarded by such a
presence proof, by a key-membership proof such as `"priority" in issue.metadata`,
or by an expected optional target type that can receive absence. An indexed
value that is present with JSON `null` is not `Missing`; `exists` returns
`false` for both missing and null, while `path == null` distinguishes a present
null only when the key has already been proven present.

Membership does not search object fields or perform substring checks.

## Count And Empty

`count` and `empty` are available for arrays, maps, fact queries, and effect
queries:

```whippletree
count(LanguageE2EResult where provider == "codex") == 2
empty(BlockingIssue where severity == Critical)
```

They do not iterate over arbitrary runtime streams. Fact/effect queries are
projection reads at an assertion/checkpoint boundary.

Function typing is deterministic and fixed:

```text
count(Array<T>)        -> Int
count(Map<V>)          -> Int
count(FactQuery<T>)    -> Int
count(EffectQuery<T>)  -> Int

exists(path)           -> Bool, and may create a presence proof
exists(Array<T>)       -> Bool, true when non-empty
exists(Map<V>)         -> Bool, true when non-empty
exists(FactQuery<T>)   -> Bool, true when at least one projection matches
exists(EffectQuery<T>) -> Bool, true when at least one projection matches

empty(Array<T>)        -> Bool
empty(Map<V>)          -> Bool
empty(String)          -> Bool
empty(FactQuery<T>)    -> Bool
empty(EffectQuery<T>)  -> Bool
empty(Optional<T>)     -> Bool only when empty(T) is defined
empty(Null)            -> Bool, true
```

`count` is never defined for strings, objects, null, optional values, or scalar
values. `exists(path)` is a presence test; `exists(collection)` is a non-empty
test. The parser/type checker must distinguish these forms instead of
overloading by runtime value shape. Unsupported calls are compile-time errors,
including unknown function names and arity mismatches.

`empty(expr)` is a structural emptiness test only for the listed types. It must
not coerce numbers, booleans, enum variants, literals, object values, agent
refs, opaque ids, or provider transcripts to collections. For optional values,
`empty(x)` is true for missing/null and otherwise delegates to the present
inner value only if that inner type itself supports `empty`; `empty(optional
scalar)` is rejected. Use `exists x` or `x == null` for optional scalar
presence/null checks.

## Object And Array Construction

Object literals are allowed only where an expected schema is known:

```whippletree
record LanguageTask {
  provider "codex"
  language "French"
}
```

Static matrix rows are object construction with an explicit target class.

Expression-level object literals are valid only with an expected object schema
from a record/effect argument, static row, action/template parameter, or another
typed object field. The checker must validate them exactly like `record`
construction:

```whippletree
tell codex WritePatch {
  repo "whippletree"
  task {
    title issue.title
    metadata {
      priority "high"
    }
  }
}
```

Required fields must be present, unknown fields are rejected, field expressions
are checked against their declared field types, and optional fields may be
omitted. A field value of `null` is accepted only for optional/null-accepting
fields. Defaults, if the schema has them, are applied after type checking and
before runtime evaluation records the constructed value.

Object literals outside an expected schema context are rejected, including
comparisons against anonymous objects and untyped array elements:

```whippletree
{ provider "codex" } == task          # rejected
[{ provider "codex" }]                # rejected unless Array<SomeSchema> expected
```

Object construction is deterministic and pure. It may contain expressions,
array literals, and nested object literals, but those nested object literals
must also have an expected schema from their field type. It may not contain
fact/effect projection queries except where the target field type explicitly
accepts the query result type.

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
