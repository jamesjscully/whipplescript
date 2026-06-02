# Providers And Plugins

WhippleScript workflows describe coordination policy. Providers and plugins are
how that policy reaches external agents, tools, and services.

## Start With The Fixture Provider

The fixture provider is deterministic and local. Use it for quick experiments,
tests, and understanding the runtime model:

```sh
whip --store .whipplescript/dev.sqlite \
  dev examples/provider-language-e2e.whip \
  --provider fixture \
  --until idle \
  --json
```

Fixture runs are useful because they exercise the same durable effect lifecycle
without requiring Codex, Claude, Pi, Loft, BAML, or other external credentials.

## Real Providers Are Experimental

Native provider integration is still settling. The public language, CLI,
runtime behavior, and provider/plugin interfaces are not stable yet.

Current provider-related surfaces include:

- `agent.tell` effects for agent turns
- `baml.coerce` effects for typed model decisions
- `human.ask` effects for inbox-backed human review
- `loft.claim` and related Loft compatibility effects
- generic `call plugin.capability` effects

Use [Runtime And Operations Reference](runtime-operations.md) to understand
effect state, provider runs, leases, diagnostics, and evidence.

## How Agents Connect

A workflow declares logical agents:

```whip
agent codex {
  profile "repo-writer"
  capacity 2
  capabilities ["agent.tell"]
  skills ["whipplescript-author"]
}
```

The workflow sends work with `tell`:

```whip
tell codex requires ["agent.tell"] as turn """
Implement this task and summarize the changed files.
"""
```

That creates an `agent.tell` effect. A worker later claims the effect and asks a
provider to execute it. The workflow can then continue from the effect result:

```whip
after turn succeeds as completed => {
  record CompletedTurn {
    summary completed.summary
  }
}
```

This separation is the key integration model:

- source code chooses the logical agent and policy
- the runtime records the durable effect
- a worker executes the effect through a provider
- provider output returns as events, facts, runs, diagnostics, and evidence

## Profiles, Capabilities, And Skills

Profiles describe authority, such as `repo-reader` or `repo-writer`.

Capabilities describe what an agent or provider is allowed to do, such as
`agent.tell` or a plugin capability.

Skills are context bundles attached to agents or turns. They are not imports and
they do not change language semantics.

## Codex, Claude, And Pi

WhippleScript can model Codex-, Claude-, and Pi-style providers as logical
agents today. Fixture-backed examples let you validate the orchestration shape
without real credentials.

Native adapters for these providers are still experimental. Expect setup,
credential references, cancellation behavior, artifacts, and evidence capture to
change while the provider system settles.

Current native surfaces:

| Provider | Native Surface | Identity | Cancellation | Artifact/Evidence Shape |
| --- | --- | --- | --- | --- |
| Codex | app-server JSON-RPC | thread id and turn id | `turn/interrupt` | app-server notifications, tool/approval summaries, diff metadata |
| Claude | Agent SDK sidecar | session id and stream events | SDK cooperative cancellation, live validation still required for strict support | SDK stream messages, hook/tool summaries, usage shape |
| Pi | RPC mode | session id, parent id, event stream | RPC `abort` | RPC events, model/provider metadata, terminal summaries |

Use the native surfaces for provider validation work. Command-backed harnesses
remain compatibility and test surfaces; they are not enough for strict native
provider readiness.

## Credentials And Configuration

Do not put credentials in `.whip` source. Source should name logical agents,
profiles, capabilities, and plugins. Runtime/provider configuration should hold
credential references and execution details.

Provider configs are validated with:

```sh
WHIPPLESCRIPT_NATIVE_PROVIDER_CONFIGS=examples/provider-configs/native/native.example.json \
scripts/check-native-provider-configs.sh
```

Strict validation requires configs for `codex-main`, `claude-main`, and
`pi-main` using native surfaces. The checker records only redacted validation
results. It must not print secret values, auth payloads, raw prompts, or raw
provider responses.

Provider credentials should be represented as references in config, such as an
environment variable name, keychain handle, or external secret id. The runtime
should validate that the reference exists and is allowed for the selected
provider, but source workflows should never embed the value.

Provider-specific setup expectations:

- Codex: install a Codex CLI that supports `codex app-server`; schema pinning is
  checked by `scripts/check-codex-app-server-schema.sh`.
- Claude: local developer validation can reuse the installed Claude CLI login
  reported by `claude auth status`. CI/strict live jobs should stay opt-in; when
  embedded auth is needed, use `ANTHROPIC_API_KEY`, Bedrock environment
  configuration, or Vertex environment configuration.
- Pi: install the Pi CLI with RPC support. `scripts/check-pi-rpc-surface.sh`
  validates offline RPC session/model shape.

Workspace policy is validated before provider launch. Current accepted config
values are `shared`, `read_only`, `per_effect_worktree`, `per_issue_worktree`,
and `remote_sandbox`. Native Codex, Claude, and Pi writer policies currently
allow local writable modes only: `shared`, `per_effect_worktree`, and
`per_issue_worktree`. `read_only` denies write/command capabilities, and
`remote_sandbox` is rejected until remote workspace preparation is implemented.
The runtime store records prepared workspaces with policy, provider, run/effect
links, URI, status, and metadata; `scripts/check-workspace-records.sh` covers
durable record validation and launch-policy denial paths. Real native smoke for
each workspace mode remains a strict-provider validation task.

## Plugin Authoring

Plugins register capabilities, providers, profiles, and bindings. They should
not add hidden control flow. A workflow should still make sequencing explicit
through facts, rules, effects, and `after` branches.

For a user-facing orientation, read [Plugin Authoring](plugin-authoring.md). For
the current manifest design, read the design-facing
[Plugin Author Guide](../spec/plugin-author-guide.md).

## Validation Scripts

Provider smoke tests are opt-in because they may need external tools,
credentials, or local services:

```sh
WHIPPLESCRIPT_E2E_REAL_PROVIDERS=1 \
WHIPPLESCRIPT_REAL_PROVIDERS=loft,baml,codex \
scripts/check-real-providers.sh
```

For the smallest real Codex smoke:

```sh
scripts/check-codex-message.sh
```

For an OpenAI-backed local BAML-compatible coerce bridge:

```sh
scripts/check-openai-coerce.sh
```

Native provider validation:

```sh
whip --json doctor --providers
scripts/check-native-provider-surfaces.sh
scripts/check-native-provider-endpoint-health.sh
scripts/check-native-provider-policy-denials.sh
WHIPPLESCRIPT_CODEX_APP_SERVER_ERROR_LIVE=1 scripts/check-codex-app-server-error-smoke.sh
WHIPPLESCRIPT_CLAUDE_AGENT_SDK_ERROR_LIVE=1 scripts/check-claude-agent-sdk-error-smoke.sh
WHIPPLESCRIPT_PI_RPC_ERROR_LIVE=1 scripts/check-pi-rpc-error-smoke.sh
scripts/check-provider-scheduling-capacity.sh
WHIPPLESCRIPT_E2E_REAL_PROVIDERS=1 \
WHIPPLESCRIPT_REAL_PROVIDER_NATIVE_SURFACE=1 \
WHIPPLESCRIPT_REAL_PROVIDERS=codex,claude,pi \
scripts/check-real-providers-report.sh
```

`whip doctor --providers --json` is a non-live posture check. It reports Codex,
Claude, and Pi CLI availability, credential-reference posture, and which deeper
checks require explicit real-provider validation. It does not start provider
turns or print credential values.

Strict native validation:

```sh
WHIPPLESCRIPT_E2E_REAL_PROVIDERS=1 \
WHIPPLESCRIPT_REAL_PROVIDER_NATIVE_STRICT=1 \
WHIPPLESCRIPT_NATIVE_PROVIDER_CONFIGS=examples/provider-configs/native/native.example.json \
scripts/check-real-providers-report.sh
```

Strict mode rejects command-wrapper-only selections and requires native provider
config validation. The report wrapper writes:

- `target/real-provider-smoke-report.md`
- `target/real-provider-preflight.jsonl`
- `target/real-provider-reports/<provider>.json`

The per-provider JSON reports contain set/unset environment posture, evidence
refs, check counts, and redacted preflight records.

Provider artifact rows are metadata-only. Operators can inspect them with:

```sh
whip --json artifacts <run-id>
```

The command lists artifact ids, kinds, redacted paths/refs, redacted content
hashes, MIME types, and timestamps. It does not read or print raw artifact
content.

Native artifact metadata gates:

```sh
scripts/check-claude-agent-sdk-artifact-smoke.sh
scripts/check-pi-rpc-artifact-smoke.sh
```

These validate provider-shaped artifact metadata refs through native adapter
boundaries. Live provider-generated artifact fixtures still require isolated
provider workspaces/accounts before they can be treated as shipping evidence.

Live artifact fixture mode:

```sh
WHIPPLESCRIPT_CLAUDE_DISPOSABLE_TARGET=claude-artifact-fixture \
WHIPPLESCRIPT_CLAUDE_DISPOSABLE_ACK=I_UNDERSTAND_THIS_PROVIDER_TARGET_IS_DISPOSABLE \
WHIPPLESCRIPT_CLAUDE_AGENT_SDK_ARTIFACT_LIVE=1 \
scripts/check-claude-agent-sdk-artifact-smoke.sh

WHIPPLESCRIPT_PI_DISPOSABLE_TARGET=pi-artifact-fixture \
WHIPPLESCRIPT_PI_DISPOSABLE_ACK=I_UNDERSTAND_THIS_PROVIDER_TARGET_IS_DISPOSABLE \
WHIPPLESCRIPT_PI_RPC_ARTIFACT_LIVE=1 \
scripts/check-pi-rpc-artifact-smoke.sh
```

The Claude live fixture accepts either local `claude auth status` login or
embedded auth through API key, Bedrock, or Vertex. The disposable target value is
not used as a path; it is a marker proving the operator meant to run a provider
that may create files in a temporary test workspace. The live fixture uses the
Agent SDK's `bypassPermissions` mode with `allowDangerouslySkipPermissions`
inside that temporary disposable workspace and asks for the exact disposable
artifact path. Weaker edit-acceptance mode, and relative-path prompts under the
local SDK, can complete without creating the requested file in the verified
workspace.

The optional GitHub Actions workflow `Native Provider Validation` runs Codex,
Claude, and Pi as separate matrix jobs in native-surface mode and uploads the
same report directory. Dispatching it with `strict=true` runs the all-provider
strict gate.

## Cancellation, Artifacts, And Recovery

Provider cancellation is capability-declared. The runtime must not ask a
provider for a deeper cancellation mode than the binding supports. Today:

- Codex and Pi have validated native stop paths.
- Claude has deterministic fake cancellation coverage and live local-auth Agent
  SDK interruption coverage. The observed SDK terminal after interrupt is
  `error_during_execution`; the sidecar normalizes it to
  `claude.turn.cancelled` when a cancel request is in flight.

Artifacts and evidence are bounded records, not raw transcript dumps. Provider
runs should capture refs for transcripts, stdout/stderr compatibility logs,
diff/changed-file metadata, stream/tool events, and artifact capture failures.
Required artifact capture failures force the terminal outcome to failed rather
than completed.

For Codex app-server request callbacks, WhippleScript records approval-capable
server requests as redacted `agent.turn.tool_requested` evidence and answers
conservatively by default: command/file approvals are declined, user-input and
tool-call callbacks receive empty or failed responses, and unknown callbacks get
a JSON-RPC error response. On Codex CLI `0.128.0`, disposable live file-change
probes emitted `item/fileChange/outputDelta`, `turn/diff/updated`, and
`fileChange` item notifications under `workspace-write`/`on-request`; no
server-side approval request was observed for that file fixture.
`scripts/check-codex-app-server-error-smoke.sh` validates the corresponding
error path. In live mode it sends an invalid `turn/start` to the installed
Codex app-server and records only the JSON-RPC error code and message shape.
`scripts/check-claude-agent-sdk-error-smoke.sh` validates the Claude error path;
in live mode it uses an intentionally invalid Claude executable path and records
only the sidecar error code plus message shape. `scripts/check-pi-rpc-error-smoke.sh`
validates the Pi RPC error path; in live mode it sends an intentionally invalid
RPC command and records only the failed response/error payload shape.

Native boundary policy denial is validated before provider launch. A source
workflow that requires `repo.write` while the current native bridge requests a
read-only workspace fails durably with `workspace_denied` for Codex, Claude, and
Pi. Current live provider-emitted denial probes are not stable shipping gates:
Claude unavailable-write prompts can terminate as `error_max_turns`, and Pi
no-tools write prompts can complete as ordinary assistant messages.

Recovery is event-log driven. If provider evidence is recorded but terminal
append fails, recovery reconciles the running run from provider evidence and
appends exactly one terminal outcome. Terminal completion keys include provider
correlation and a terminal payload hash so retries cannot create contradictory
terminal states. Operators can run `whip recover <instance>` to invoke this
reconciliation path after restarting a worker or reopening a store; completed
native runs should report `recovered_count: 0` and must not append duplicate
terminal lifecycle events.

## Destructive Provider Tests

Provider-destructive tests are refused unless the target is explicitly marked
disposable. Set a destructive flag, such as
`WHIPPLESCRIPT_REAL_PROVIDER_DESTRUCTIVE_TESTS=1` or
`WHIPPLESCRIPT_CODEX_DESTRUCTIVE_TESTS=1`, then set a disposable target marker
and acknowledgement:

```sh
WHIPPLESCRIPT_CODEX_DISPOSABLE_TARGET=codex-disposable-smoke
WHIPPLESCRIPT_CODEX_DISPOSABLE_ACK=I_UNDERSTAND_THIS_PROVIDER_TARGET_IS_DISPOSABLE
```

The report records only whether those markers are set. It does not print target
values.

Local release checks exercise this gate without contacting providers by running
`scripts/check-real-provider-destructive-gate.sh`. That fixture covers the
non-destructive skip path, the missing disposable marker failure path, and the
acknowledged disposable marker pass path.

## Practical Advice

- Start with fixture provider runs.
- Add assertions to prove the workflow reached the expected state.
- Inspect `effects`, `runs`, `diagnostics`, and `evidence` before debugging
  provider code.
- Keep provider identity in source metadata, not in model-generated text.
- Treat real providers as experimental until your target adapter has native
  validation, cancellation, artifact, and recovery coverage.
