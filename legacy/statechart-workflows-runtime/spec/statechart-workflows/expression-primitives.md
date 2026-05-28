# Expression Primitives

Status: design proposal

Armature is an orchestration language, not a general-purpose data programming
language. Its expression system exists to route workflow events, update small
durable workflow data, prepare effect payloads, and branch on typed decisions.

The expression system deliberately supports a small set of primitives that cover
ordinary agent-orchestration workflows. Computation that needs iteration,
ranking, summarization, parsing, fuzzy judgment, or domain policy belongs behind
a declared typed function or adapter capability.

## Design Rule

Armature-compatible types are schemas. They do not imply that the workflow
language supports every possible operation over that type.

For example:

- a `float` can be stored, compared, passed to `coerce`, and returned by
  adapters; v1 does not provide a numeric math library
- a `string[]` can be checked for membership, appended to, removed from, and
  counted; v1 does not provide general `map`, `filter`, or `reduce`
- `image`, `audio`, `pdf`, and `video` are reserved opaque boundary values; once
  enabled by schema and policy, workflows may pass them to declared functions or
  adapters, but cannot manipulate their contents inline

If a workflow needs to "find the best next task", it should call:

```armature
let next = coerce chooseNextStep(planText)
```

or:

```armature
let next = plan.nextReadyItem()
```

It should not implement ranking with loops inside the statechart.

## Primitive Set

The v1 expression kernel supports only these categories.

### 1. Literals

Supported literal values:

```text
string
block string
int
float
duration
true
false
nil
list literal
object literal
```

Examples:

```armature
let count = 1
let threshold = 0.75
let retryAfter = 2m
let payload = {
  task next.workItemId
  message next.message
}
let tags = ["worker", "quality"]
```

### 2. Paths And Field Access

Supported path roots:

```text
data
event binding names, such as run
let bindings in the current transition
runtime observations exposed as built-ins
```

Examples:

```armature
run.id
run.exitCode
data.seenRuns
classification.kind
next.message
```

Static validation should reject paths that cannot exist from declared schemas.
When a path crosses an optional value, validation should require an explicit
guard or pattern that proves the value is present before field access.

Implemented optional-presence proof is intentionally conservative. It accepts:

- direct nil comparisons such as `data.user != nil` and `nil != data.user`
- negated nil equality such as `!(data.user == nil)`
- conjunctions where one term proves presence
- disjunctions only when every branch proves the same path is present
- De Morgan-style negated disjunctions such as `!(data.user == nil || blocked)`
- double negation around supported proofs
- `case` arms where the current pattern is non-nil, or where a previous `nil`
  arm makes a later wildcard arm non-nil

It does not attempt full SAT-style boolean reasoning. If the validation rule is
not obvious to a human reader, prefer an explicit `path != nil` guard.

### 3. Equality And Ordering

Supported operators:

```text
==
!=
<
<=
>
>=
```

Rules:

- equality works on scalars, enums, literals, `nil`, and comparable opaque
  identity values
- ordering works on `int`, `float`, `duration`, and `time`
- no implicit string-to-number or int-to-string coercion
- comparing incompatible schemas is a validation error when statically known

Examples:

```armature
guard run.status == "succeeded"
guard run.exitCode != nil
guard elapsedSince(data.lastIdleNudgeAt) >= 2m
```

### 4. Boolean Logic

Supported operators:

```text
!
&&
||
```

Rules:

- operands must be boolean
- guards must evaluate to boolean
- short-circuiting is allowed but expressions must remain effect-free

Example:

```armature
guard activeRuns() == 0 && plan.unfinishedItems() > 0
```

### 5. Membership

Supported operator:

```text
value in list_or_set_or_map_keys
```

Rules:

- list membership requires the value schema to match the list item schema
- map membership checks keys
- `!(value in xs)` is the canonical v1 spelling for negative membership

Example:

```armature
guard !(run.id in data.seenRuns)
```

### 6. Case And Pattern Matching

Supported patterns:

```text
EnumValue
literal
matches "glob-*"
_
```

Rules:

- enum cases should be exhaustive or include `_`
- glob matching is for strings only
- v1 glob semantics are shell-style `*` wildcard matching, not regex
- case arms do not bind new destructured variables in v1

Examples:

```armature
case next.action {
  StartWorker -> { ... }
  Wait -> { ... }
  Done -> { goto done }
}

case run.name {
  matches "worker-*" -> { ... }
  matches "quality-*" -> { ... }
  _ -> { stay }
}
```

### 7. Object And List Construction

Object literals are used to build typed function inputs and effect payloads.
List literals are used for small literal collections and initial data.

Rules:

- object fields use Armature field syntax: `field expr`
- records are closed when validated against a `class`/record schema
- optional fields may be omitted or set to `nil`
- list item values must satisfy the declared item schema where known

Example:

```armature
let summary = {
  id run.id
  name run.name
  status run.status
  stdoutTail run.stdoutTail
  stderrTail run.stderrTail
  exitCode run.exitCode
}
```

### 8. String Interpolation

String interpolation is supported in Armature strings outside `prompt` blocks.

Rules:

- interpolation expressions are restricted to paths in v1
- interpolation is for messages and small text payloads
- prompt blocks use BAML/Jinja template semantics instead

Example:

```armature
send director """
Worker failed: {{ classification.reason }}
"""
```

### 9. List Helpers

Supported helpers:

```text
list.length(xs) -> int
list.isEmpty(xs) -> bool
list.contains(xs, value) -> bool
list.append(xs, value) -> list<T>
list.remove(xs, value) -> list<T>
list.first(xs) -> T?
xs.append(value) -> list<T>
xs.remove(value) -> list<T>
```

Rules:

- helpers are pure and effect-free
- `append` requires `value` to match the list item schema
- `remove` removes values by equality
- `first` returns `nil` for an empty list
- v1 does not include `map`, `filter`, `reduce`, sorting, slicing, or lambdas

Examples:

```armature
assign data.seenRuns = data.seenRuns.append(run.id)
guard list.length(data.activeItems) < 4
```

### 10. Map Helpers

Supported helpers:

```text
map.get(m, key) -> T?
map.set(m, key, value) -> map<K, T>
map.remove(m, key) -> map<K, T>
map.containsKey(m, key) -> bool
```

Rules:

- v1 map keys must be string-compatible because runtime values are JSON objects
- `set` requires `value` to match the map value schema
- `get` returns `nil` if absent
- no map iteration is supported in v1

### 11. Text Helpers

Supported helpers:

```text
text.trim(s) -> string
text.contains(s, needle) -> bool
text.startsWith(s, prefix) -> bool
text.endsWith(s, suffix) -> bool
text.matchesGlob(s, pattern) -> bool
```

Rules:

- no regex in v1
- no casing, splitting, replacing, or parsing library in v1
- text helpers are for routing and diagnostics, not document processing

### 12. Time And Duration Helpers

Supported helpers:

```text
now() -> time
elapsedSince(t) -> duration
time.elapsedSince(t) -> duration
```

Rules:

- duration literals support `ms`, `s`, `m`, `h`, and `d`
- durations and times can be compared with ordering operators
- no calendar arithmetic in v1

Example:

```armature
guard elapsedSince(data.lastIdleNudgeAt) >= 2m
assign data.lastIdleNudgeAt = now()
```

### 13. Typed Function Calls

Supported typed function calls:

```text
coerce functionName(args...)
functionName(args...) when functionName resolves to a coerce declaration
capability.operation(args...)
approved built-ins listed in this document
```

Rules:

- no user-defined arbitrary functions in v1
- direct calls to undeclared names are validation errors
- coerce calls are synchronous value effects backed by BAML HTTP
- capability value calls are synchronous value effects backed by adapters
- statement-style capability calls are adapter effects

Example:

```armature
let planText = plan.snapshot()
let next = coerce chooseNextStep(planText)
```

## Explicit Non-Goals

The v1 expression kernel does not support:

```text
loops
recursion
user-defined functions
lambdas
map/filter/reduce
sorting
arbitrary arithmetic libraries
regex
general string processing
multimodal manipulation
arbitrary JSON mutation
implicit type coercions
host-language imports
shell commands
TypeScript/JavaScript bodies
```

These exclusions are product decisions, not missing conveniences. They keep
workflow files legible to non-specialists and preserve a tractable verification
surface.

## Multimodal Values

BAML supports multimodal values such as `image`, `audio`, `pdf`, and `video`.
Armature reserves those names for opaque boundary values, but they are enabled
only when the schema layer and policy layer explicitly support them.

Allowed operations:

- store in declared data when policy allows
- pass to `coerce`
- pass to adapter operations
- compare to `nil`
- inspect declared metadata fields if the value is represented by an Armature
  `class`

Disallowed operations:

- append media bytes
- crop, transcode, extract text, or inspect contents inline
- fetch URL media without explicit adapter policy
- infer filesystem or network authority from the type

Any media fetch, decoding, OCR, transcription, or transformation must be an
adapter operation or BAML provider operation with explicit policy and audit
records.
