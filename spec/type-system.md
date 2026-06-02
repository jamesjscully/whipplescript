# Type System

Status: draft

WhippleScript needs a real type system because facts, effect payloads, Loft
contracts, BAML coercions, plugin capabilities, and evidence records all cross
typed boundaries.

WhippleScript is still not a general-purpose data language. Types are schemas for
validation, routing, persistence, and external calls. Supporting a type does not
mean supporting every operation over that type.

## Design Rule

```text
WhippleScript-compatible types are schemas; they do not imply a full data language.
```

For example:

- a `float` can be stored, compared, passed to `coerce`, and returned by an
  effect; v0 does not provide a numeric math library
- a `string[]` can be stored, counted, checked for membership, passed to
  `coerce`, and interpolated; v0 does not provide `map`, `filter`, or `reduce`
- an `image` can be passed to a model/capability as an opaque boundary value;
  WhippleScript cannot inspect pixels or transform the media inline

## Type Universe

Core scalar types:

```text
string
int
float
bool
null
duration
time
```

Schema/container types:

```text
literal
optional<T>
array<T>
map<V>
enum
class
AgentRef<A | B | ...>
```

Opaque multimodal boundary types:

```text
image
audio
pdf
video
```

## Source Syntax

Primitive types:

```whipplescript
string
int
float
bool
null
duration
time
image
audio
pdf
video
```

Optional types:

```whipplescript
string?
WorkReview?
```

Arrays:

```whipplescript
string[]
WorkItem[]
```

Maps:

```whipplescript
map<string>
map<int>
map<WorkReview>
```

WhippleScript maps have string keys in v0. `map<T>` means:

```text
string -> T
```

Enums and classes:

```whipplescript
enum ReviewStatus {
  Accept
  Revise
  Blocked
}

class WorkReview {
  status ReviewStatus
  reason string
  followups string[]
  confidence float
}
```

Literal types use BAML-style literal values where needed:

```whipplescript
class StatusEvent {
  kind "accepted" | "rejected" | "blocked"
  reason string
}
```

Agent references name workflow-declared logical agents and are used for
deterministic routing:

```whipplescript
class LanguageTask {
  provider AgentRef<codex | claude | pi>
}
```

`AgentRef` values are represented as JSON strings at runtime, but statically
they are not plain strings: each listed agent must be declared, record literals
must belong to the allowed domain, and dynamic `tell` targets must have an
`AgentRef` type.

Declared agents carry typed static metadata that is visible to the compiler:

```whipplescript
agent codex {
  profile "code"
  capacity "high"
  capabilities ["edit", "test", "review"]
}
```

The exact declaration syntax may evolve, but the type-system contract is fixed:
each declared agent has a stable name, one profile, one capacity class, and a
finite set of capabilities. `AgentRef<A | B>` is valid only when `A` and `B`
are declared agents. Guards and effect targets may further constrain the domain
by profile, capacity, and required capabilities; every remaining possible
target must satisfy those static constraints before the rule is accepted.

Agent-routing effects can declare their target capability contract directly:

```whipplescript
tell task.provider requires ["repo.write"] as turn """
Update the implementation.
"""
```

For a static target, the named agent must declare every required capability. For
a dynamic `AgentRef`, every agent still possible after type and pattern
refinement must declare every required capability. Runtime policy enforcement
repeats this check against persisted program-version agent metadata and blocks
mismatches as `blocked_by_capability` before provider execution starts.

Union types are limited in v0. The compiler may produce tagged unions for
effect terminal outputs, but user-authored arbitrary unions should wait until
we need them.

## Canonical IR Schema

IR should represent types structurally:

```json
{ "type": "string" }
{ "type": "int" }
{ "type": "float" }
{ "type": "bool" }
{ "type": "null" }
{ "type": "duration" }
{ "type": "time" }
{ "type": "optional", "inner": { "type": "string" } }
{ "type": "array", "items": { "type": "string" } }
{ "type": "map", "values": { "type": "int" } }
{ "type": "ref", "name": "WorkReview" }
{ "type": "literal", "value": "accepted" }
{
  "type": "agent_ref",
  "agents": ["codex", "claude", "pi"],
  "constraints": {
    "profiles": ["code"],
    "capacities": ["medium", "high"],
    "capabilities": ["edit", "test"]
  },
  "provenance": {
    "declared_at": "workflow.whipplescript:3:1",
    "refined_by": ["workflow.whipplescript:18:27"]
  }
}
{ "type": "media", "kind": "image" }
```

Class IR:

```json
{
  "type": "class",
  "name": "WorkReview",
  "fields": [
    { "name": "status", "schema": { "type": "ref", "name": "ReviewStatus" } },
    { "name": "reason", "schema": { "type": "string" } },
    { "name": "followups", "schema": { "type": "array", "items": { "type": "string" } } },
    { "name": "confidence", "schema": { "type": "float" } }
  ]
}
```

Enum IR:

```json
{
  "type": "enum",
  "name": "ReviewStatus",
  "values": ["Accept", "Revise", "Blocked"]
}
```

## JSON Representation

Runtime JSON representation:

```text
string    JSON string
int       JSON number with integer validation
float     JSON number
bool      JSON boolean
null      JSON null
array     JSON array
map       JSON object with string keys
enum      JSON string containing enum variant
AgentRef JSON string containing a declared agent name
class     JSON object with closed fields
literal   exact JSON literal value
optional  missing field or null, depending on containing schema
```

Closed classes reject unknown fields unless a specific adapter contract marks a
payload as open. WhippleScript-authored classes should be closed by default.

## Media Boundary Values

Multimodal values are opaque references:

```text
MediaRef {
  kind: image | audio | pdf | video
  artifact_id?
  uri?
  mime_type?
  content_hash?
  metadata?
}
```

Rules:

- media values are never inline bytes inside facts/effects
- media values may be passed to `coerce` or registered capabilities when schema
  and policy allow it
- WhippleScript may compare media references for identity
- WhippleScript may inspect metadata fields only if the schema exposes them as
  ordinary fields
- WhippleScript cannot transform media inline

## Allowed Operations

Detailed expression semantics live in
[expression-kernel.md](expression-kernel.md). This section names the operation
surface from the type-system perspective.

The v0 expression kernel supports:

```text
literals
field access
optional presence checks
equality and ordering
boolean logic
membership
small object/list construction
string interpolation over paths
enum/literal pattern matching
array count/empty checks
append/remove for small workflow facts, if needed
```

The same expression kernel is used for `when ... where ...` guards and
workflow assertions. Guard and assertion expressions are pure: they can inspect
matched facts and effect projections, but they cannot enqueue effects, call
providers, invoke BAML, mutate facts, read files, perform network access, or
run host-language code.

Operation constraints:

- ordering works on `int`, `float`, `duration`, and `time`
- equality works on scalars, enums, literals, null, and comparable opaque
  identity values
- membership works on arrays and map keys
- interpolation is limited to paths and simple values
- object construction must satisfy a known class/effect/fact schema where one
  is expected
- list items must satisfy the declared item schema where known
- pattern branches are allowed only over finite typed domains, optional
  presence, and tagged terminal-output unions
- pattern branch bindings refine types for that branch only

Not supported in v0:

```text
loops
map/filter/reduce
numeric math library
string parsing toolkit
media manipulation
general user-defined functions
user-defined methods
general object destructuring
array/list destructuring
regex/string pattern matching
user-defined pattern extractors
```

Nontrivial data reasoning belongs in BAML `coerce` functions or registered
capabilities.

## Routing-Friendly Types

Literal unions and enums must be usable in guards because deterministic routing
is a core workflow concern:

```whipplescript
enum Provider {
  Codex
  Claude
  Pi
}

class LanguageTask {
  provider Provider
  language string
  expectedScript string
}

rule route_codex
  when {
    LanguageTask as task where task.provider == Codex
    codex is available
  }
=> {
  tell codex """
  {{ task.language }}
  """
}
```

The compiler must type-check the guard against the matched schema. Comparing an
enum to an unknown variant, a literal union to a value outside the union, or a
field to an incompatible scalar type is a compile-time error.

Agent references are a distinct routing type, not arbitrary strings:

```whipplescript
AgentRef<codex | claude | pi>
```

An `AgentRef` value may be matched, stored in facts, and used by `tell` only
when every possible target is a declared agent and satisfies the rule's static
profile, capacity, and capability constraints. A plain `string` must not be
accepted as a dynamic `tell` target.

Static checking rules:

- each named agent in `AgentRef<...>` must exist in the workflow's declared
  agent table
- the compiler records each declared agent's profile, capacity class, and
  capabilities in the typed environment
- assigning `"codex"` to an `AgentRef<codex | claude>` field is allowed only
  because the literal is in the finite agent domain
- assigning a `string` expression, interpolated string, model output, plugin
  output, or untyped JSON value to an `AgentRef` field is rejected unless it
  passes through an explicit typed boundary that validates it as `AgentRef`
- narrowing an `AgentRef<A | B | C>` with guards or patterns may only remove
  agents from the finite domain; it must not widen to undeclared names
- `tell task.provider` is accepted only when `task.provider` has an
  `AgentRef<...>` type and the current refined domain satisfies the target
  effect's declared constraints
- `tell task.provider requires [...]` is accepted only when every agent in the
  current refined domain declares every listed capability

Dynamic target checking still runs after static checking. When a rule fires,
the runtime validates that the selected JSON string is one of the IR
`agent_ref.agents`, that the selected agent still has the required profile,
capacity, and capabilities, and that the caller is authorized to address or
claim that agent. Runtime failure to authorize or claim a target is a typed
effect failure; it must not silently fall back to another agent or reinterpret
the value as a plain string.

Runtime authorization and claimability are separate from type membership:

- authorization answers "may this workflow/rule/session address this declared
  agent?"
- claimability answers "may this rule instance reserve work for this agent now,
  given current capacity, leases, and availability?"
- both checks must run before enqueueing `tell`, assignment, or other agent
  work-claiming effects
- failed checks preserve the original target value and diagnostic metadata in
  evidence records

AgentRef IR must carry enough metadata for audit and replay. Besides the finite
agent domain and constraint set, IR should preserve provenance for declared
agent definitions, source spans for refinements, and whether the value came from
a source literal, fact field, validated boundary input, or previous runtime
selection. Provenance is not used to decide type equality, but it is required
for diagnostics, evidence records, and runtime authorization logs.

Provider/model identity should normally be deterministic metadata. BAML output
types should contain provider fields only when the model is reviewing observed
provider evidence, not as a way to select or confirm orchestration routes.

## Optional And Null Rules

Optional fields must be proven present before field access:

```whipplescript
when issue.assignee != null
when issue.assignee.name == "Ada"
```

The compiler should accept only obvious presence proofs in v0:

```text
x != null
null != x
!(x == null)
conjunction containing a presence proof
case branch matching non-null variant
```

It should reject clever boolean reasoning and suggest an explicit presence
guard.

## Pattern Matching Types

Pattern matching is type refinement over a finite or presence-aware domain. It
does not introduce a general destructuring language.

Allowed v0 scrutinee types:

```text
enum
literal union
optional<T>
tagged terminal-output union
```

Enum patterns must name declared variants:

```whipplescript
enum ReviewStatus {
  Accept
  Revise
  Blocked
}

case review.status {
  Accept => ...
  Revise => ...
  Blocked => ...
}
```

Literal-union patterns must use one of the exact literal values in the union:

```whipplescript
class LanguageTask {
  provider "codex" | "claude" | "pi"
}

case task.provider {
  "codex" => ...
  "claude" => ...
  "pi" => ...
}
```

Optional patterns refine presence:

```whipplescript
case issue.assignee {
  Some assignee => assignee.name
  None => "unassigned"
}
```

Inside the `Some` branch, `assignee` has the non-optional inner type. Inside the
`None` branch, the missing value cannot be dereferenced.

Tagged terminal-output unions are matched by terminal status/variant. This is
the typed form of branching over effect completions; providers do not decide
which branch should run through free-form text.

Compiler obligations:

- reject patterns that cannot match the scrutinee type
- reject unknown enum variants and literal values outside the declared domain
- reject duplicate branches unless the duplicate is unreachable by construction
- require an explicit wildcard/default branch when a total expression is needed
  and the finite domain is not fully covered
- preserve branch-specific type refinements in later field access checks
- keep branch guards type-checked with the normal expression kernel

## BAML Lowering

Reachable WhippleScript declarations used by `coerce` lower to generated BAML:

```text
enum       -> BAML enum
class      -> BAML class
string     -> string
int        -> int
float      -> float
bool       -> bool
array<T>   -> T[]
map<T>     -> map<string, T>, or BAML-compatible equivalent
literal    -> literal
image      -> image
audio      -> audio
pdf        -> pdf
video      -> video
```

The compiler must reject any WhippleScript type used in a `coerce` signature that
cannot be lowered to the selected BAML version.

Generated BAML is a build artifact. WhippleScript declarations remain the source of
truth.

## Boundary Validation

Typed validation happens at every boundary:

```text
source declarations
fact production
effect input construction
effect success/failure/timeout outputs
BAML request arguments
BAML parsed responses
plugin capability inputs/outputs
human answer payloads
evidence metadata, where typed
```

Validation failures should produce diagnostics with source spans when possible.
Provider output validation failures become typed effect failures, not partial
successes.

## Fact Types

WhippleScript-authored durable facts use class schemas:

```whipplescript
class ReviewedWork {
  turn AgentTurn
  review WorkReview
}

record ReviewedWork {
  turn turn
  review review
}
```

The `record` body is checked against the class:

- every required field must be present
- unknown fields are rejected
- field expressions must satisfy declared schemas
- optional fields may be omitted or set to `null`
- classes are closed by default

Built-in and integration facts, such as agent turn completions and Loft ready
issues, also expose class-like schemas even when their source syntax is
conversational.

## Diagnostics And Desire Paths

The compiler should be generous with harmless aliases:

```text
array<T> and T[]
null and nil, if we choose to keep nil as an alias
map<T> and object<T>, if desired
```

But strict where semantics matter:

- no implicit string/int/float coercion
- no field access through optional values without a presence proof
- no unknown enum variants
- no unknown class fields
- no inline media manipulation
- no unsupported data operations disguised as method calls

Good diagnostic shape:

```text
`review.followups.map(...)` is not supported in WhippleScript.
WhippleScript is an orchestration language, not a data language.
Move this transformation into a `coerce` function or a registered capability.
```
