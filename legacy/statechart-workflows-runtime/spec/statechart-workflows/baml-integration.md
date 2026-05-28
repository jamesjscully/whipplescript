# BAML Integration

Status: design proposal

Whippletree uses BAML's type and prompt model without making users author a
separate embedded BAML island by default.

`.whip` source owns:

- `enum` declarations
- `class` declarations
- `coerce` declarations
- model/client shorthand used by `coerce`

The compiler lowers those declarations into generated BAML source files and
validates them with the BAML toolchain. The generated BAML source is a build
artifact, not the primary authoring surface.

## Decision

Use this source model:

```whippletree
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

client<llm> WhippletreeDefaultLLM {
  provider "openai"
  options {
    model "gpt-4o-mini"
    api_key env.OPENAI_API_KEY
  }
}

function selectNextAction(planText: string) -> NextAction {
  client WhippletreeDefaultLLM
  prompt #"
    Choose the next safe orchestration action.

    {{ planText }}

    {{ ctx.output_format }}
  "#
}
```

Do not require this as the normal user-authored shape:

```whippletree
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

Whippletree declarations are canonical. Generated BAML artifacts are derived:

```text
.whippletree/build/workflows/<machine>/baml_src/*.baml
```

The generated BAML source includes only declarations reachable from `coerce`
function inputs and outputs. Workflow-only `class` or `enum` declarations that
are used only for events, data, or adapter schemas remain in WorkflowIR but are
not emitted into `workflow.baml`.

The v1 runtime uses BAML HTTP as the selected execution path. Whippletree either
connects to an existing BAML server or, in a later managed mode, starts:

```sh
baml-cli serve --from <build-dir>/baml_src
```

The Rust runtime then calls:

```text
POST /call/<function_name>
```

with named JSON arguments. Generated TypeScript clients and the BAML Rust SDK
are not part of the v1 `coerce` execution plan. They may be revisited later if
the HTTP path proves insufficient.

## Coerce Execution Contract

`coerce` is a synchronous value effect. It may produce a typed value used by the
same transition. It may not directly start agents, enqueue events, edit files,
or mutate workflow data.

Runtime sequence:

```text
evaluate coerce argument expressions
validate each argument against the declared parameter schema
build named JSON argument object
check durable coerce call log for an existing successful idempotency key
if present, reuse the recorded parsed output
otherwise call BAML HTTP
validate returned JSON against the declared output schema
record input, backend, raw metadata, parsed output, validation result, and error
continue transition with the parsed value
```

Request shape:

```json
{
  "planText": "W1 ready",
  "recentRuns": []
}
```

The source syntax can look positional:

```whippletree
let next = coerce chooseNextStep(planText, recentRuns)
```

but lowering preserves parameter names from the `coerce` declaration so the
HTTP request is named:

```json
{
  "planText": "...",
  "recentRuns": []
}
```

The runtime must reject too many, too few, or schema-invalid arguments before
calling BAML.

## Backend Modes

### External BAML Server

The first implementation should support an external server:

```sh
whip run workflow.whip --baml-url http://127.0.0.1:2024
```

The user or hosting environment is responsible for starting `baml-cli serve`
against the generated `baml_src` directory.

Advantages:

- smallest Whippletree implementation slice
- clearer enterprise deployment boundary
- no process supervision required for the first real `coerce` path
- simple integration testing when `baml-cli` and provider credentials exist

### Managed BAML Server

Managed mode is a later convenience:

```sh
whip run workflow.whip --manage-baml
```

Whippletree starts `baml-cli serve --from <build-dir>/baml_src`, captures logs, and
stops the server when the workflow host stops. This mode may reuse process and
log-capture lessons from the legacy runtime, but the legacy runtime should not
own the `coerce` semantics.

Managed mode must record:

- server command and version
- generated `baml_src` hash
- selected port or socket
- stdout/stderr paths or captured log records
- startup and health-check failures

## Idempotency And Replay

Every `coerce` call has a stable idempotency key:

```text
workflow_id/workflow_version/event_id/step_path/function_name/args_json
```

If the runtime finds a successful record for that key, it reuses the parsed
output. It must not call BAML again for a committed transition and silently
produce a different decision.

`transition_attempt` is intentionally not part of the v0 runtime key. Retrying a
failed call appends a new failed or successful attempt; once a successful record
exists for the same event, step, function, and arguments, replay reuses it.

If a previous failed record exists for the same key, retry behavior is governed
by workflow failure policy and circuit breakers. Retries append new attempts;
they do not overwrite the previous record.

## Durable Coerce Call Record

The storage layer should persist a `coerce_calls` record with:

```json
{
  "coerce_call_id": "coerce_01H...",
  "workflow_id": "implementationLoop",
  "workflow_version": "statechart-workflow-ir/v0",
  "transition_id": "tr_01H...",
  "event_id": "evt_01H...",
  "step_path": "state:choosing/entry/0",
  "function_name": "chooseNextStep",
  "idempotency_key": "implementationLoop/statechart-workflow-ir/v0/evt_01H/state:choosing/entry/0/chooseNextStep/{\"planText\":\"W1 ready\"}",
  "backend": {
    "kind": "baml_http",
    "url": "http://127.0.0.1:2024",
    "baml_src_hash": "sha256:..."
  },
  "args": {
    "planText": "W1 ready"
  },
  "status": "succeeded",
  "http_status": 200,
  "raw_response": {
    "redacted": true
  },
  "parsed_output": {
    "action": "StartWorker",
    "workItemId": "W1",
    "reason": "ready",
    "message": "Implement W1"
  },
  "error": null,
  "duration_ms": 1350,
  "created_at": "2026-05-23T10:00:00Z"
}
```

Raw response storage is controlled by policy and may be replaced with a
redaction marker before persistence. Parsed output and error category must
remain visible enough for replay and status.

## Failure Behavior

Failure categories:

```text
baml_server_unavailable
baml_http_error
baml_timeout
baml_parse_failure
baml_schema_validation_failure
baml_policy_denied
internal_error
```

When `coerce` fails:

- no later steps in the transition run
- tentative state changes from the failed transition are discarded
- the queued event is marked failed or routed through a declared failure path
  when supported
- a durable coerce call record is written
- status and overview show a current coerce failure while the failed event is
  unresolved, and keep latest coerce failures as historical diagnostics

Default behavior is visible blocked/failure. Silent fallback values are not
allowed.

## Supported Declaration Surface

Whippletree-authored BAML-shaped declarations:

```text
enum
class
coerce
prompt blocks
model shorthand
```

The first implementation should not expose BAML tests, tools, generators, or
advanced client routing in `.whip` source. Those can be introduced later if
the product needs them.

## Type Mapping

BAML-compatible declarations map into Whippletree schema types:

```text
BAML string              -> Schema::String
BAML int                 -> Schema::Int
BAML float               -> Schema::Float
BAML bool                -> Schema::Boolean
BAML null                -> Schema::Null
BAML image/audio/pdf/video -> reserved opaque media schemas, once enabled
BAML string/int/bool literal -> Schema::Literal
BAML Type?               -> Schema::Optional
BAML Type[]              -> Schema::List
BAML map<Key, Value>     -> Schema::Map
BAML A | B               -> Schema::Union
BAML enum                -> Schema::Enum
BAML class               -> Schema::Record or Schema::Ref
```

Generated Whippletree records/classes are closed for validation. Values may omit
optional fields but may not include undeclared fields. Use `map<string, T>` or a
native `json` field for deliberately open object-shaped data.

Whippletree stores and validates map values as JSON objects. The compiler therefore
accepts only string-compatible map keys in v0: `string`, enums, string literals,
and unions/refs composed from those. This keeps generated BAML map boundaries
aligned with the runtime representation instead of accepting schemas that cannot
round-trip through JSON object keys.

Enum values must start with an uppercase ASCII letter, matching BAML's enum
rules. The workflow validator rejects lowercase enum values before build so
generated `workflow.baml` does not fail later in the BAML toolchain.

Whippletree workflow-native types such as `time`, `duration`, and `agent` may
exist in `data`, event payloads, and adapter schemas, but they are not valid
BAML boundary types unless an adapter or compiler rule maps them explicitly to a
BAML-compatible representation.

Multimodal BAML types such as `image`, `audio`, `pdf`, and `video` are reserved
for future opaque media schema support. The current runtime must reject them
unless the schema layer, policy layer, and BAML HTTP executor all explicitly
support the media representation being passed.

BAML does not support `set` or `tuple`. If Whippletree supports set-like durable
data internally, it must not emit that type as a BAML input/output schema.

BAML does not support generic `any/json` as a preferred structured boundary. If
a workflow needs arbitrary JSON at a model boundary, it should pass a string or
use a more specific class/map/union schema.

BAML-compatible schemas do not imply full inline operations in the Whippletree
expression language. Whippletree can store, compare where meaningful, route, and
pass values through typed boundaries. The supported operation set is defined in
[expression-primitives.md](expression-primitives.md).

## Coerce Calls

Both call forms are accepted:

```whippletree
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

Whippletree should support these as `Schema::Literal` and `Schema::Union`.

For workflow control branches, examples should prefer enums:

```whippletree
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

```whippletree
coerce choose(planText string) -> NextStep {
  model "gpt-4o-mini"
  prompt """..."""
}
```

The compiler expands this to generated BAML source according to workspace
defaults and policy. Policy controls:

```text
allowed providers
allowed model names
allowed environment variables
whether network/model calls are allowed
whether BAML tools are allowed
allowed BAML HTTP URLs
whether Whippletree may manage a local baml-cli serve process
```

Advanced explicit client declarations may be added later, but they should stay
compatible with BAML rather than inventing a parallel client language.

## Template Strings

Prompt blocks use BAML/Jinja template semantics inside generated BAML.

Whippletree expression interpolation is not active inside prompt blocks. Prompt
variables are the coerce function parameters and BAML-provided context such as
`ctx.output_format`.

Outside prompt blocks, Whippletree strings use Whippletree interpolation rules:

```whippletree
send director """
Worker failed: {{ classification.reason }}
"""
```

## Multimodal Types

BAML supports multimodal types such as `image`, `audio`, `pdf`, and `video`.

Whippletree should not support multimodal coerce inputs in the first
implementation unless adapter policy explicitly enables them. URL-based media
can create SSRF and egress risks, so multimodal values need explicit policy,
allowlists, and audit records.

When enabled, multimodal values are opaque in Whippletree expressions. Workflows may
store them, pass them to BAML, pass them to declared adapters, or test for
presence. They may not inspect, transform, fetch, transcode, or parse media
inline.

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
