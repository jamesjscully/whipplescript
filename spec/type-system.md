# Type System

Status: draft

Armature needs a real type system because facts, effect payloads, Docket
contracts, BAML coercions, plugin capabilities, and evidence records all cross
typed boundaries.

Armature is still not a general-purpose data language. Types are schemas for
validation, routing, persistence, and external calls. Supporting a type does not
mean supporting every operation over that type.

## Design Rule

```text
Armature-compatible types are schemas; they do not imply a full data language.
```

For example:

- a `float` can be stored, compared, passed to `coerce`, and returned by an
  effect; v0 does not provide a numeric math library
- a `string[]` can be stored, counted, checked for membership, passed to
  `coerce`, and interpolated; v0 does not provide `map`, `filter`, or `reduce`
- an `image` can be passed to a model/capability as an opaque boundary value;
  Armature cannot inspect pixels or transform the media inline

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

```armature
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

```armature
string?
WorkReview?
```

Arrays:

```armature
string[]
WorkItem[]
```

Maps:

```armature
map<string>
map<int>
map<WorkReview>
```

Armature maps have string keys in v0. `map<T>` means:

```text
string -> T
```

Enums and classes:

```armature
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

```armature
class StatusEvent {
  kind "accepted" | "rejected" | "blocked"
  reason string
}
```

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
class     JSON object with closed fields
literal   exact JSON literal value
optional  missing field or null, depending on containing schema
```

Closed classes reject unknown fields unless a specific adapter contract marks a
payload as open. Armature-authored classes should be closed by default.

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
- Armature may compare media references for identity
- Armature may inspect metadata fields only if the schema exposes them as
  ordinary fields
- Armature cannot transform media inline

## Allowed Operations

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

Operation constraints:

- ordering works on `int`, `float`, `duration`, and `time`
- equality works on scalars, enums, literals, null, and comparable opaque
  identity values
- membership works on arrays and map keys
- interpolation is limited to paths and simple values
- object construction must satisfy a known class/effect/fact schema where one
  is expected
- list items must satisfy the declared item schema where known

Not supported in v0:

```text
loops
map/filter/reduce
numeric math library
string parsing toolkit
media manipulation
general user-defined functions
user-defined methods
```

Nontrivial data reasoning belongs in BAML `coerce` functions or registered
capabilities.

## Optional And Null Rules

Optional fields must be proven present before field access:

```armature
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

## BAML Lowering

Reachable Armature declarations used by `coerce` lower to generated BAML:

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

The compiler must reject any Armature type used in a `coerce` signature that
cannot be lowered to the selected BAML version.

Generated BAML is a build artifact. Armature declarations remain the source of
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

Armature-authored durable facts use class schemas:

```armature
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

Built-in and integration facts, such as agent turn completions and Docket ready
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
`review.followups.map(...)` is not supported in Armature.
Armature is an orchestration language, not a data language.
Move this transformation into a `coerce` function or a registered capability.
```
