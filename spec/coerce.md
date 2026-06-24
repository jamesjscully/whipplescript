# Coerce And Schema Coercion

Status: draft

`coerce` is WhippleScript's typed schema-coercion effect. It turns
unstructured or semi-structured external output into a declared WhippleScript
type, or fails with a typed diagnostic. coerce is one backend/toolchain that can
implement this coercion well; it is not the conceptual owner of `coerce` and it
does not own workflow control flow.

In the rule-machine design, `coerce` is not a synchronous in-transaction call.
It is a durable external effect:

```text
rule requests schema.coerce
provider/backend performs schema coercion
runtime records artifacts
completion event returns typed value
rules consume the completed value
```

Current implementation note: the effect kind and several APIs still use the
legacy name `coerce`. The target conceptual effect kind is `schema.coerce`,
with coerce as a concrete backend/provider under `std.coercion`.

This differs from the earlier statechart design, where `coerce` was described as
a synchronous value effect within one transition. The type and expression
discipline still mostly transfers; the synchronous execution model does not.

See [type-system.md](type-system.md) for the shared boundary type system used by
`coerce` signatures, schema-coercion backend mappings, and runtime validation.

## Source Shape

Possible authoring syntax:

```whipplescript
class WorkClassification {
  status WorkStatus
  reason string
}

coerce classifyWork(summary string) -> WorkClassification {
  prompt """markdown
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

The first rule enqueues a `schema.coerce` effect. A later completion produces a
typed fact that the second rule can match. In the current implementation this is
still emitted as `coerce`.

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

Target lowered effect:

```text
kind: schema.coerce
binding: review
coercion: reviewWork
```

Input:

```text
coercion_name
named_arguments
generated_coercion_artifact_hash
input_schema_hash
output_schema_hash
backend/provider config
idempotency_key
```

Current coerce-backed implementations may use compatibility field names such as
`function_name` and `generated_coerce_source_hash`; those are transport details,
not the conceptual contract.

A `coerce` terminal uses the canonical terminal-output union defined once in
[expression-kernel.md](expression-kernel.md):
`Completed<O> | Failed<E> | TimedOut | Cancelled`. Domain success and failure
payloads are the `O`/`E` type parameters, not new tags.

For `reviewWork(...) -> WorkReview`:

- `O` is `WorkReview` (the declared return type), bound by `succeeds`.
- `E` is the rich `SchemaCoerceFailed` payload below, bound by `fails`. Per
  [admission-and-idempotency.md](admission-and-idempotency.md) and
  [expression-kernel.md](expression-kernel.md), `Failed<E>` carries an optional
  domain payload `E` (defaulting to the core `Failure { reason, ... }`); a
  validation/coercion failure's `E` is the rich payload here.
- `TimedOut` is the canonical timeout tag (terminal status value `timed_out`),
  bound by `times out`. Its payload is the schema-coercion timeout detail below.
- `Cancelled` is the canonical cancellation tag, bound by `cancelled`.

Success output (the `Completed<O>` payload `O`):

```text
DeclaredReturnType
```

Failure payload (the `Failed<E>` payload `E`):

```text
SchemaCoerceFailed {
  coercion_name
  reason
  recoverable
  validation_errors?
  provider_error?
  evidence_refs
}
```

Timeout payload (carried by the canonical `TimedOut` tag):

```text
SchemaCoerceTimedOut {
  coercion_name
  timeout
  recoverable
  evidence_refs
}
```

Typing rules:

```whipplescript
after review succeeds {
  // review : WorkReview                 (Completed<WorkReview>)
}

after review fails as f {
  // f : SchemaCoerceFailed              (the Failed<E> payload)
}

after review times out {
  // review : SchemaCoerceTimedOut       (the TimedOut payload)
}

after review cancelled {
  // canonical Cancelled tag
}

after review completes {
  // review : Completed<WorkReview> | Failed<SchemaCoerceFailed>
  //          | TimedOut | Cancelled    (full union, for case)
}
```

## Data Operations

WhippleScript should keep the boundary-type expression rule, now formalized in
[type-system.md](type-system.md):

```text
WhippleScript-compatible types are schemas; they do not imply a full data language.
```

The language may support schema-coercion boundary types:

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

## Schema Coercion Artifacts And coerce

WhippleScript source may define:

- classes
- enums
- coerce functions
- prompt templates
- backend/client shorthand

Default mode: WhippleScript declarations are the source of truth, and the
configured schema-coercion toolchain emits locked backend artifacts. For the
coerce backend, that means generated coerce source. Generated coerce is an artifact,
not the main authoring surface.

Interop mode: existing backend artifacts, such as `.coerce` files, can be included
or bound explicitly. The checker must cross-validate the backend functions and
types against WhippleScript declarations and record compatibility hashes.

Generated artifacts should record:

```text
coercion_artifact_hash
backend_toolchain_version
coercion_name
input_schema_hash
output_schema_hash
backend/provider config
```

coerce-specific evidence may additionally record `coerce_source_hash`,
`coerce_cli_or_sdk_version`, and coerce function/type names.

## Execution Modes

All execution modes must expose the same durable schema-coercion contract,
exposed today through the `coerce` effect kind.

**Removed bridge-server design (2026-06-20).** An earlier draft proposed a local
coerce-compatible HTTP service that the runtime would POST a `/coerce` body to
(`ManagedCoerceService`, `HttpCoerceClient`, `scripts/openai-coerce-server.mjs`,
`scripts/check-openai-coerce.sh`). No real provider implements such a `/coerce`
endpoint — it was a fictional bridge — so that design and its placeholders have
been removed. The real integration is **provider-native structured outputs**:
OpenAI via the Responses endpoint with a JSON-schema constraint, Anthropic via
the Messages API with a single tool whose `input_schema` is the output type.

Current implementation:

- `FakeCoerceClient` (`whipplescript_kernel::coerce`) provides deterministic CI
  coverage for success, failure, and timeout branches without credentials; the
  fixture provider drives coerce through it. It is the default path.
- `NativeCoerceClient` (`whipplescript_kernel::coerce_native`) is the real
  provider-native client. Request construction and response parsing are pure,
  unit/mock-tested functions; the network call lives behind a `CoerceTransport`
  (the CLI supplies a synchronous `ureq` transport, so the kernel stays
  network-free). For OpenAI it POSTs `/v1/responses` with a
  `text.format.json_schema` constraint (`strict` when the schema has no
  schema-valued `additionalProperties`); for Anthropic it POSTs `/v1/messages`
  with one forced tool whose `input_schema` is the output schema. The output
  JSON Schema is synthesized from the declared `coerce` output type, and the
  prompt's `{{ ctx.output_format }}` token embeds that schema so endpoints
  without native structured output can still return schema-shaped JSON.

The native path is **opt-in and credential-gated** via environment variables, so
the fixture path remains the default for `dev`/`worker`/CI:

| Variable | Meaning |
| --- | --- |
| `WHIPPLESCRIPT_COERCE_PROVIDER` | `openai` or `anthropic`; unset → fixture path |
| `WHIPPLESCRIPT_COERCE_MODEL` | the model id (codex path falls back to `~/.codex/config.toml`; required otherwise) |
| `WHIPPLESCRIPT_COERCE_BASE_URL` | override the API base URL (e.g. a mock or the Codex backend) |
| `WHIPPLESCRIPT_COERCE_MAX_TOKENS` | Anthropic output-token bound (default 4096) |
| `WHIPPLESCRIPT_COERCE_TIMEOUT_SECS` | per-request timeout (default 120) |

Credentials resolve as:

- **OpenAI**: `OPENAI_API_KEY` (standard `api.openai.com` Responses, non-streaming
  JSON), else the Codex OAuth token in `~/.codex/auth.json`, which routes to the
  ChatGPT-plan codex backend `chatgpt.com/backend-api/codex/responses`. The codex
  path adds the codex headers (`chatgpt-account-id`, `openai-beta:
  responses=experimental`, `originator`, `session_id`), message-shaped input, and
  `stream: true`; the transport assembles the SSE `response.output_text` deltas
  (SSE is keyed off the request's `accept`, since the server content-type is
  unreliable). The model is not hard-coded: `WHIPPLESCRIPT_COERCE_MODEL` wins,
  else the codex path reads `model` from `~/.codex/config.toml`. **Validated 2026-06-23**: the codex
  endpoint honors `text.format` json_schema structured outputs (live probe + a
  full `whip dev` coerce run returning conforming JSON).
- **Anthropic**: a console API key only (`ANTHROPIC_API_KEY` or a `whip
  auth`-stored `sk-ant-api*` key, sent as `x-api-key`). A Claude Code OAuth token
  (`sk-ant-oat*`) is rejected at resolution — reusing it for the API is a terms
  gray area.

When the provider is set but no credential resolves, the coerce effect fails with
a clear message rather than silently falling back to a fixture.

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

`coerce` follows the admission, idempotency, and replay contract in
[admission-and-idempotency.md](admission-and-idempotency.md): the terminal
outcome is recorded once with its evidence, and replay reads the recorded fact
without re-invoking the model.

Because `coerce` is model-backed, its idempotency key (or its companion
execution fingerprint) must commit to every input that changes the result, so a
changed model, prompt, or schema is a different key and a stale outcome is never
reused:

```text
instance_id
program_version
function_name
normalized_named_args_hash
trigger_event_or_fact_correlation
provider_or_model_id
prompt_or_coercion_artifact_hash
output_schema_hash
```

If a successful output already exists for the key, the runtime reuses it and
does not call the model again. A change to the provider/model id, the
prompt/coercion-artifact hash, or the output-schema hash yields a different key,
so a recorded outcome from a different contract is never reused.

## Policy

Schema coercion execution requires policy approval for:

- backend and model/client provider
- credentials source
- network posture
- prompt artifact retention
- input/output logging posture

Enterprise environments may forbid raw prompt logging while still retaining
schema hashes, timing, status, and redacted summaries.
