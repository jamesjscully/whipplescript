# Managed BAML Runtime

Status: target specification, revised after sandbox validation

Armature should manage BAML by default. A user running a `.armature` workflow
should not have to start a sidecar `baml-cli serve` process just because the
workflow contains `coerce`.

`coerce` is part of the Armature language. BAML is the implementation backend
for typed model calls. That makes BAML runtime lifecycle an Armature
responsibility unless the operator explicitly supplies an external BAML service
or broker.

The important product constraint is that coding agents often run inside
sandboxes. Some harness sandboxes deny local TCP listeners and general network
access, including loopback sockets. Therefore the default managed BAML path must
not require Armature to open a local HTTP server inside the agent sandbox.

## Product Target

Default local UX:

```sh
armature run workflow.armature
```

If the workflow executes a real `coerce` call and no fake coerce outputs were
supplied, Armature should:

1. Generate BAML source for reachable `coerce` types/functions.
2. Write the source into the normal workflow build artifact location.
3. Generate or load a small BAML client runner.
4. Call that runner over stdin/stdout JSON, not a local listening socket.
5. Record source hash, runner command, logs, backend mode, health, and failures.
6. Stop the managed process when the owning Armature command exits.

The default backend mode is `generated_stdio`. It may still require outbound
model/provider network authority when the model call happens, but it does not
add a local TCP listen requirement on top of that.

External HTTP UX:

```sh
armature run workflow.armature --baml-url http://127.0.0.1:2024
```

`--baml-url` means "do not manage the local BAML client runner; use this
already-approved endpoint." It is an override for hosted BAML, CI, model
gateways, debugging, or enterprise-managed BAML services. It is not the normal
agent-sandbox path.

Brokered enterprise UX:

```sh
armature run workflow.armature --coerce-backend brokered
```

The exact CLI may change, but the mode means the sandboxed Armature process
records a durable `coerce_requested` item and an out-of-sandbox trusted service
performs the BAML/model call. This keeps provider credentials and network
authority outside the coding agent sandbox.

Fake coerce UX remains deterministic:

```sh
armature run workflow.armature \
  --fake-coerce-output chooseNextStep='{"action":"Done","reason":"complete"}'
```

Fake outputs bypass BAML entirely. They are the default test strategy for
deterministic workflow tests.

Codex OAuth UX:

```sh
armature run workflow.armature --baml-auth codex-oauth
```

This mode is only for generated stdio execution. Armature loads an existing
Codex/ChatGPT OAuth access token from `CODEX_HOME/auth.json`,
`~/.codex/auth.json`, or `ARMATURE_CODEX_OAUTH_ACCESS_TOKEN`, then injects it
into the runner as `ARMATURE_CODEX_OAUTH_ACCESS_TOKEN`. The generated BAML
client uses the `openai-responses` provider with Codex-compatible provider
options:

```baml
api_key env.ARMATURE_CODEX_OAUTH_ACCESS_TOKEN
base_url env.ARMATURE_CODEX_OAUTH_BASE_URL
store false
stream true
instructions "Follow the BAML-generated prompt and return only the requested structured output."
```

The default base URL is `https://chatgpt.com/backend-api/codex`, matching the
Codex ChatGPT-login Responses endpoint. This is explicit credential authority,
not an API-key alias. Enterprise policy must allow it with
`allow_baml_codex_oauth: true`.

Armature should rely on BAML for prompt rendering, provider request
construction, and output parsing. Any Codex-specific local code must be limited
to response compatibility, such as extracting final text from Codex SSE events
when BAML's current `openai-responses` stream parser rejects the endpoint's
final event shape.

## Modes

The effective BAML runtime mode is resolved in this order when a `coerce` call
is actually evaluated:

```text
fake outputs present        -> fake
--baml-url supplied         -> external_http
brokered mode selected      -> brokered
workflow has no coerce      -> none
otherwise                  -> generated_stdio
```

Within `generated_stdio`, `--baml-auth api-key` uses ordinary provider API keys
and `--baml-auth codex-oauth` uses Codex/ChatGPT OAuth credentials.

Armature may resolve this lazily. Ignored events, guard failures, and
transitions that do not reach `coerce` must not require BAML startup or BAML
policy approval just because the workflow file declares a `coerce` function.
The effective mode should be visible in status/debug output when a store exists
or when a coerce call is recorded.

`managed_http` is retained only as an explicit compatibility/debug mode for
`baml-cli serve`. It must not be the default for agent-sandbox execution.

## Generated Stdio Contract

The generated stdio runner is a child process owned by Armature. Its interface
is JSON lines or another framed stdio protocol:

Request:

```json
{
  "id": "coerce_01H...",
  "function": "chooseNextStep",
  "arg_order": ["planText"],
  "args": {
    "planText": "W1 ready"
  },
  "baml_src_dir": ".armature/build/workflows/ImplementationLoop/baml_src",
  "baml_src_hash": "sha256:..."
}
```

`args` are named because Armature validates workflow calls by parameter name.
`arg_order` is also required because BAML's generated TypeScript client exposes
functions as positional methods such as `b.ChooseNextStep(planText)`. A runner
must call generated clients using `arg_order`, not object key iteration.

Response:

```json
{
  "id": "coerce_01H...",
  "ok": true,
  "value": {
    "action": "StartWorker",
    "workItemId": "W1",
    "reason": "ready",
    "message": "Implement W1"
  },
  "raw": {
    "redacted": true
  }
}
```

The default local runner artifact is a Node ESM entrypoint at:

```text
.armature/build/workflows/<workflow>/baml_runner/armature-baml-runner.mjs
```

Armature invokes it as `node <runner> --baml-src <baml_src>`. The runner expects
Armature to write `generators.baml`, run
`baml-cli generate --from <baml_src> --no-version-check --no-tests`, and create
a generated TypeScript `baml_client` beside the runner. The runner imports
`{ b }`, resolves the requested function by name, and writes exactly one JSON
response to stdout. Custom runners configured with
`ARMATURE_BAML_GENERATED_STDIO_RUNNER` are executed directly and skip generation
so tests and future enterprise brokers can replace the local Node runner without
changing the workflow language.

The runner may be generated TypeScript, generated Rust, or another BAML
toolchain-supported client. The first implementation should choose the smallest
path that:

- uses BAML-generated client code rather than reimplementing BAML calls
- supports one request per process or a persistent process
- communicates over stdin/stdout, not a listening socket
- preserves named JSON arguments and typed JSON outputs
- exits with machine-readable errors
- writes stdout/stderr or protocol logs to durable run artifacts

One-request-per-process is acceptable for the first slice if it is simpler and
reliable. A persistent runner can be added after the protocol and audit shape
are stable.

## External HTTP Contract

External mode uses a BAML-compatible HTTP endpoint:

```text
POST /call/<function_name>
```

with named JSON arguments. `baml-cli serve` is one way to provide that endpoint.
Armature does not manage the process in this mode. The operator is responsible
for source approval, service lifecycle, credentials, and network placement.

This mode is appropriate when the user or enterprise already runs a model
gateway or wants a shared BAML service outside the agent sandbox.

## Brokered Contract

Brokered mode is the enterprise-friendly split authority path:

1. The sandboxed workflow runtime validates `coerce` arguments.
2. It records a durable request with workflow id, event id, step path, function,
   args, schema hash, BAML source hash, and policy context.
3. A trusted broker claims the request outside the agent sandbox.
4. The broker performs the BAML/model call with approved credentials/network.
5. The broker writes a durable success or failure result.
6. Armature resumes the event or a retry command consumes the completed result.

The broker may internally use generated stdio, BAML HTTP, hosted BAML, or
provider-specific execution. That choice is outside the workflow language
boundary.

Brokered mode avoids granting model credentials and broad network access to
coding agents while preserving typed `coerce` semantics.

## Artifacts And Audit

Managed BAML runtime records should include:

```json
{
  "mode": "generated_stdio",
  "command": ["node", ".armature/build/workflows/MyWorkflow/baml_runner.js"],
  "baml_src_dir": ".armature/build/workflows/MyWorkflow/baml_src",
  "baml_src_hash": "sha256:...",
  "runner_hash": "sha256:...",
  "stdout_path": ".armature/runs/baml/.../stdout.log",
  "stderr_path": ".armature/runs/baml/.../stderr.log",
  "started_at": "2026-05-26T20:00:00Z",
  "ready_at": "2026-05-26T20:00:01Z",
  "exit_code": null,
  "error": null
}
```

The existing `coerce_calls.backend_json` should continue to record the backend
used for each call. Example backend variants:

```json
{ "kind": "baml_generated_stdio", "baml_src_hash": "sha256:...", "runner_hash": "sha256:..." }
{ "kind": "baml_http", "url": "https://baml.internal", "baml_src_hash": "sha256:..." }
{ "kind": "baml_brokered", "request_id": "coerce_req_01H...", "baml_src_hash": "sha256:..." }
```

Runtime-level BAML records may initially be logged as durable operational
records. If process reuse, broker queues, or pooled services become important,
use dedicated tables.

## Policy

Real BAML requires model/network authority somewhere because it can cause model
calls through BAML clients. The key question is where that authority lives:

- `generated_stdio`: in the Armature command's process sandbox
- `external_http`: in the external service endpoint
- `brokered`: in a trusted out-of-sandbox broker

Policy fields:

```json
{
  "allowed_capabilities": ["baml.coerce"],
  "allow_baml_network": true,
  "allow_baml_stdio_runner": true,
  "allow_baml_http": false,
  "allow_baml_broker": true,
  "allowed_baml_urls": ["https://baml.internal"],
  "allowed_models": ["gpt-4o-mini"],
  "allowed_env_vars": ["OPENAI_API_KEY"],
  "store_baml_raw_responses": false
}
```

Rules:

- `baml.coerce` authorizes structured model output as a workflow capability.
- `allow_baml_network` controls whether the selected local execution path may
  contact model providers or model gateways directly.
- `allow_baml_stdio_runner` controls whether Armature may generate and run a
  local BAML client process over stdin/stdout.
- `allow_baml_http` controls whether Armature may call a BAML HTTP endpoint.
- `allow_baml_broker` controls whether Armature may enqueue brokered coerce
  requests.
- `allowed_baml_urls` applies to `external_http`.
- `allowed_models` applies once Armature owns or can inspect model/client
  selection in generated BAML source.
- `allowed_env_vars` limits environment variables inherited by generated local
  runners.
- `store_baml_raw_responses` controls raw response persistence.

Local/default mode may allow `generated_stdio` with warnings. Enterprise mode
should require explicit `baml.coerce` plus one approved execution path:

- direct local model calls: `allow_baml_stdio_runner: true`,
  `allow_baml_network: true`, and explicit allowed env/model policy
- external HTTP: `allow_baml_http: true` and exact `allowed_baml_urls`
- brokered: `allow_baml_broker: true`

If policy denies all real BAML paths and no fake output is supplied, the
diagnostic should say exactly how to proceed:

```text
workflow uses coerce, but no approved real BAML backend is available.
Fix: allow generated stdio execution, pass --baml-url with an approved endpoint,
select brokered coerce, or supply --fake-coerce-output for deterministic tests.
```

## Environment

Generated local runners must not inherit the entire Armature environment in
governed modes. They should receive:

- minimal runtime variables needed to execute the process, such as `PATH`,
  `HOME`, and `TMPDIR`
- variables listed in `allowed_env_vars`
- no workflow secrets unless explicitly allowed

Local mode may be more permissive, but the target implementation should use the
same allowlist machinery as harness provider profiles where practical.

## Failure Behavior

BAML failures are workflow execution failures, not hidden retries.

Initial categories:

```text
baml_cli_not_found
baml_generation_failed
baml_runner_start_failed
baml_runner_protocol_error
baml_broker_unavailable
baml_http_unavailable
baml_http_error
baml_timeout
baml_policy_denied
baml_parse_failure
baml_schema_validation_failure
internal_error
```

Before a local runner is ready, failures should fail the command without
mutating workflow state for the triggering event. During a coerce call,
existing durable coerce failure semantics apply: tentative state is discarded,
the event is marked failed, and status/overview show the current coerce failure
while unresolved.

## Status

`status` and `overview` should expose enough information to debug BAML without
opening logs first:

```json
{
  "baml_runtime": {
    "mode": "generated_stdio",
    "baml_src_hash": "sha256:...",
    "runner_hash": "sha256:...",
    "status": "ready",
    "stdout_path": "...",
    "stderr_path": "...",
    "last_error": null
  }
}
```

If no runtime record exists yet, status can derive only from coerce call
history. The first implementation may show BAML runtime evidence through latest
coerce call backend metadata and recent failure records, then add a dedicated
projection once runtime records exist.

## Implementation Plan

Suggested slices:

1. Replace the default managed HTTP path with a generated stdio backend.
2. Add policy validation for `generated_stdio`, `external_http`, and `brokered`
   modes.
3. Add a `GeneratedBamlRunner` helper:
   - generate/write BAML source
   - run `baml-cli generate`
   - generate a tiny runner entrypoint if the BAML toolchain does not provide
     one directly
   - pass one named JSON request over stdin/stdout
   - capture stdout/stderr artifacts
   - validate protocol responses
4. Keep `--baml-url` as explicit `external_http`.
5. Add brokered storage/protocol only after stdio is working.
6. Record backend metadata in coerce call records and status/logs.
7. Add tests:
   - unit tests for mode resolution and policy diagnostics
   - deterministic generated-runner tests with a tiny fake generated runner
   - protocol error tests that require no TCP listener
   - real BAML e2e remains opt-in for actual BAML toolchain plus model
     credentials

## Non-Goals

- Replacing BAML's compiler or runtime.
- Embedding arbitrary BAML source as the primary authoring surface.
- Making `baml-cli serve` the default agent-sandbox execution path.
- Proving LLM behavior in formal models.
- Long-lived shared BAML daemon pooling in the first implementation.
