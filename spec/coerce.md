# Coerce And BAML

Status: draft

`coerce` is WhippleScript's typed model-decision effect. It uses BAML's strengths:
typed classes/enums, prompt functions, clients, tests, and output validation.

In the rule-machine design, `coerce` is not a synchronous in-transaction call.
It is a durable external effect:

```text
rule requests baml.coerce
harness executes BAML function
runtime records artifacts
completion event returns typed value
rules consume the completed value
```

This differs from the legacy statechart design, where `coerce` was described as
a synchronous value effect within one transition. The old type and expression
discipline still mostly transfers; the synchronous execution model does not.

See [type-system.md](type-system.md) for the shared boundary type system used by
`coerce` signatures, BAML lowering, and runtime validation.

## Source Shape

Possible authoring syntax:

```whipplescript
class WorkClassification {
  status WorkStatus
  reason string
}

coerce classifyWork(summary string) -> WorkClassification {
  prompt """
  Classify this agent result:

  {{ summary }}

  {{ ctx.output_format }}
  """
}

rule classify
  when worker completed work as item
=> {
  coerce classifyWork(item.summary) as classification
}

rule accept
  when classification.status is Accepted for work as item
=> {
  done item -> record AcceptedWork {
    id item.id
    status "accepted"
  }
}
```

The first rule enqueues a `baml.coerce` effect. A later completion produces a
typed fact that the second rule can match.

If another effect must use the coerce output immediately, use the execution
contract's dependency syntax:

```whipplescript
coerce classifyWork(item.summary) as classification

after classification succeeds {
  tell reviewer "{{ classification.status }}"
}
```

This lowers to durable effect dependency edges, not an inline model call.

## Coerce Effect Contract

Source syntax:

```whipplescript
coerce reviewWork(turn.issue.title, turn.summary, turn.changedFiles) as review
```

Lowered effect:

```text
kind: baml.coerce
binding: review
function: reviewWork
```

Input:

```text
function_name
named_arguments
generated_baml_source_hash
input_schema_hash
output_schema_hash
client/model config
idempotency_key
```

Success output:

```text
DeclaredReturnType
```

If `reviewWork(...) -> WorkReview`, then inside:

```whipplescript
after review succeeds {
  // review : WorkReview
}
```

Failure output:

```text
BamlCoerceFailed {
  function_name
  reason
  recoverable
  validation_errors?
  provider_error?
  evidence_refs
}
```

Timeout output:

```text
BamlCoerceTimedOut {
  function_name
  timeout
  recoverable
  evidence_refs
}
```

Typing rules:

```whipplescript
after review succeeds {
  // review : WorkReview
}

after review fails {
  // review : BamlCoerceFailed
}

after review completes {
  // review : WorkReview | BamlCoerceFailed | BamlCoerceTimedOut
}
```

## Data Operations

WhippleScript should keep the legacy expression rule, now formalized in
[type-system.md](type-system.md):

```text
WhippleScript-compatible types are schemas; they do not imply a full data language.
```

The language may support BAML-compatible boundary types:

```text
string
int
float
bool
null
array/list
map/object
literal
enum
class
image
audio
pdf
video
```

But v0 should only provide small, pure operations needed for orchestration:

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

It should not provide:

```text
loops
map/filter/reduce
floating point math library
string parsing library
media manipulation
ranking/search algorithms
general user-defined functions
```

If a workflow needs nontrivial reasoning over data, it should call a typed
`coerce` function or registered capability. That keeps WhippleScript an
orchestration language rather than slowly growing into a brittle half-language.

Multimodal values are opaque boundary values. A workflow may pass an `image`,
`audio`, `pdf`, or `video` to a declared `coerce` function or capability when
schema and policy allow it, but it cannot inspect or transform the media inline.

## Generated BAML

WhippleScript source may define:

- classes
- enums
- coerce functions
- prompt templates
- model/client shorthand

The compiler lowers reachable declarations into generated BAML source. Generated
BAML is an artifact, not the main authoring surface.

Generated artifacts should record:

```text
baml_source_hash
baml_cli_or_sdk_version
function_name
input_schema_hash
output_schema_hash
client/model config
```

## Execution Modes

The preferred execution mode is still open pending implementation research.
Allowed targets:

1. Generated BAML runner process using stdio.
2. Managed BAML server if local listener authority is available.
3. External BAML endpoint in hosted environments.

All modes must expose the same durable `baml.coerce` effect contract.

Current implementation:

- `ManagedBamlService` starts and owns a local BAML-compatible process, exposing
  its configured endpoint until the service guard is dropped.
- `HttpBamlClient` posts the durable coerce contract to `http://.../coerce` and
  decodes JSON status, value, error, transcript, and usage fields.
- `scripts/openai-coerce-server.mjs` is a local BAML-compatible `/coerce`
  bridge backed by the OpenAI Responses API. It loads `OPENAI_API_KEY` from
  `.env` or the environment, uses Structured Outputs, and defaults to
  `gpt-5.4-mini` unless `WHIPPLESCRIPT_OPENAI_MODEL` is set.
- `scripts/check-openai-coerce.sh` starts that bridge and runs the no-mock
  Coerce smoke test through the same `HttpBamlClient` path used for external
  BAML-compatible endpoints.
- `FakeBamlClient` provides deterministic CI coverage for success and failure
  branches without credentials.
- A no-mock smoke test runs when `WHIPPLESCRIPT_BAML_TEST_ENDPOINT`,
  `WHIPPLESCRIPT_BAML_TEST_FUNCTION`, `WHIPPLESCRIPT_BAML_TEST_ARGUMENTS_JSON`, and
  `WHIPPLESCRIPT_BAML_TEST_OUTPUT_TYPE` are set. `scripts/check-real-providers.sh`
  requires those variables before claiming real-provider readiness.

The built-in HTTP transport intentionally covers plain `http://` endpoints only.
TLS and BAML SDK-specific transport are tracked separately.

## Completion Fact

A successful coerce effect produces a typed completion fact:

```text
coerced(function, correlation, output)
```

The source language should provide friendlier pattern syntax so users do not
write raw correlation plumbing in common cases.

Failed coerce effects produce visible facts/events:

```text
coerceFailed(function, correlation, error)
```

Rules may retry, ask a human, or block based on those facts.

## Idempotency And Replay

A `coerce` idempotency key includes:

```text
instance_id
program_version
function_name
normalized_named_args_hash
trigger_event_or_fact_correlation
```

If a successful output already exists for the key, the runtime reuses it and
does not call the model again.

## Policy

BAML execution requires policy approval for:

- model/client provider
- credentials source
- network posture
- prompt artifact retention
- input/output logging posture

Enterprise environments may forbid raw prompt logging while still retaining
schema hashes, timing, status, and redacted summaries.
