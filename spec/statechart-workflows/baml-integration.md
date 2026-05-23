# BAML Integration

Status: design proposal

Armature uses BAML's type and prompt model without making users author a
separate embedded BAML island by default.

`.armature` source owns:

- `enum` declarations
- `class` declarations
- `coerce` declarations
- model/client shorthand used by `coerce`

The compiler lowers those declarations into generated BAML source files and
validates them with the BAML toolchain. The generated BAML source is a build
artifact, not the primary authoring surface.

## Decision

Use this source model:

```armature
enum ActionKind {
  StartWorker
  Wait
  Done
}

class NextAction {
  kind ActionKind
  reason string
}

coerce selectNextAction(planText string) -> NextAction {
  model "gpt-4o-mini"

  prompt """
  Choose the next safe orchestration action.

  {{ planText }}

  {{ ctx.output_format }}
  """
}
```

The compiler may generate BAML like:

```baml
enum ActionKind {
  StartWorker
  Wait
  Done
}

class NextAction {
  kind ActionKind
  reason string
}

client<llm> ArmatureDefaultLLM {
  provider "openai"
  options {
    model "gpt-4o-mini"
    api_key env.OPENAI_API_KEY
  }
}

function selectNextAction(planText: string) -> NextAction {
  client ArmatureDefaultLLM
  prompt #"
    Choose the next safe orchestration action.

    {{ planText }}

    {{ ctx.output_format }}
  "#
}
```

Do not require this as the normal user-authored shape:

```armature
baml {
  '''
  enum ActionKind { ... }
  function selectNextAction(...) -> ... { ... }
  '''
}
```

Raw BAML imports may be added later as an escape hatch, but they are not the v1
product surface.

## Source Of Truth

Armature declarations are canonical. Generated BAML artifacts are derived:

```text
.armature/build/workflows/<machine>/baml_src/*.baml
.armature/build/workflows/<machine>/baml_client/*, if generated clients are used
```

The generated BAML source includes only declarations reachable from `coerce`
function inputs and outputs. Workflow-only `class` or `enum` declarations that
are used only for events, data, or adapter schemas remain in WorkflowIR but are
not emitted into `workflow.baml`.

The runtime may use either:

- the BAML Rust SDK with in-memory generated source, or
- generated BAML client artifacts,

as an implementation detail.

The spec should prefer the Rust SDK path if it proves stable because it avoids
checked-in generated clients. Local probing showed the current Rust crate may
require `protoc` at build time, so implementation plans must include that tool
in development and CI images if the SDK is adopted directly.

## Supported Declaration Surface

Armature-authored BAML-shaped declarations:

```text
enum
class
coerce
prompt blocks
model shorthand
```

The first implementation should not expose BAML tests, tools, generators, or
advanced client routing in `.armature` source. Those can be introduced later if
the product needs them.

## Type Mapping

BAML-compatible declarations map into Armature schema types:

```text
BAML string              -> Schema::String
BAML int                 -> Schema::Int
BAML float               -> Schema::Float
BAML bool                -> Schema::Boolean
BAML null                -> Schema::Null
BAML string/int/bool literal -> Schema::Literal
BAML Type?               -> Schema::Optional
BAML Type[]              -> Schema::List
BAML map<Key, Value>     -> Schema::Map
BAML A | B               -> Schema::Union
BAML enum                -> Schema::Enum
BAML class               -> Schema::Record or Schema::Ref
```

Generated Armature records/classes are closed for validation. Values may omit
optional fields but may not include undeclared fields. Use `map<string, T>` or a
native `json` field for deliberately open object-shaped data.

Armature stores and validates map values as JSON objects. The compiler therefore
accepts only string-compatible map keys in v0: `string`, enums, string literals,
and unions/refs composed from those. This keeps generated BAML map boundaries
aligned with the runtime representation instead of accepting schemas that cannot
round-trip through JSON object keys.

Enum values must start with an uppercase ASCII letter, matching BAML's enum
rules. The workflow validator rejects lowercase enum values before build so
generated `workflow.baml` does not fail later in the BAML toolchain.

Armature workflow-native types such as `time`, `duration`, and `agent` may
exist in `data`, event payloads, and adapter schemas, but they are not valid
BAML boundary types unless an adapter or compiler rule maps them explicitly to a
BAML-compatible representation.

BAML does not support `set` or `tuple`. If Armature supports set-like durable
data internally, it must not emit that type as a BAML input/output schema.

BAML does not support generic `any/json` as a preferred structured boundary. If
a workflow needs arbitrary JSON at a model boundary, it should pass a string or
use a more specific class/map/union schema.

## Coerce Calls

Both call forms are accepted:

```armature
let next = coerce selectNextAction(planText)
let next = selectNextAction(planText)
```

The explicit `coerce` call form is recommended when model-dependent control
flow should be easy to spot during review. Direct calls are valid when the
callee resolves to a `coerce` declaration.

The compiler must reject direct calls to undeclared names. There are no
user-defined arbitrary functions in the workflow language.

## Literal Types Versus Enums

BAML supports literal return types:

```baml
function classify(input: string) -> "bug" | "feature" {
  client WorkflowLLM
  prompt #"{{ ctx.output_format }}"#
}
```

Armature should support these as `Schema::Literal` and `Schema::Union`.

For workflow control branches, examples should prefer enums:

```armature
enum ActionKind {
  StartWorker
  Wait
  Done
}
```

Enums produce clearer diagnostics, generated client types, and finite
model-generation domains.

## Clients And Policy

The first authoring surface uses simple model shorthand:

```armature
coerce choose(planText string) -> NextStep {
  model "gpt-4o-mini"
  prompt """..."""
}
```

The compiler expands this to a BAML client declaration according to workspace
defaults and policy. Policy controls:

```text
allowed providers
allowed model names
allowed environment variables
whether network/model calls are allowed
whether BAML tools are allowed
```

Advanced explicit client declarations may be added later, but they should stay
compatible with BAML rather than inventing a parallel client language.

## Template Strings

Prompt blocks use BAML/Jinja template semantics inside generated BAML.

Armature expression interpolation is not active inside prompt blocks. Prompt
variables are the coerce function parameters and BAML-provided context such as
`ctx.output_format`.

Outside prompt blocks, Armature strings use Armature interpolation rules:

```armature
send director """
Worker failed: {{ classification.reason }}
"""
```

## Multimodal Types

BAML supports multimodal types such as `image`, `audio`, `pdf`, and `video`.

Armature should not support multimodal coerce inputs in the first
implementation unless adapter policy explicitly enables them. URL-based media
can create SSRF and egress risks, so multimodal values need explicit policy,
allowlists, and audit records.

## Model Generation

For model generation, coerce functions lower to nondeterministic outputs
constrained by their output schema.

Prompt text, model identity, BAML tests, and client/provider behavior are not
modeled. The model sees only:

```text
coerce function name
input schema
output schema
possible output domain, bounded where needed
```

If a coerce output schema cannot be finitely abstracted for a selected backend,
model generation must fail with an explicit diagnostic or require a user-supplied
abstraction.
