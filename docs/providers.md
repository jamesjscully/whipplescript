# Providers & packages

Workflows describe coordination policy; providers and packages are how that
policy reaches real agents, tools, and services. Source code names logical
agents and capabilities. The runtime records durable effects. Workers execute
those effects through providers. Provider output returns as events, facts,
runs, and evidence.

## The fixture provider

The fixture provider completes effects deterministically and locally. Use it
for development, tutorials, tests, and workflow design — it exercises the
full durable effect lifecycle with no credentials:

```sh
whip --store .whipplescript/dev.sqlite \
  dev examples/provider-language-e2e.whip \
  --provider fixture --until idle --json
```

Fixture outcome flags (`--fail`, `--timeout`, `--cancel` on `dev` and
`worker`) force terminal branches so you can test failure handling.

## Binding agents to providers

A workflow binds each logical agent to a provider family in source:

```whip
agent implementer {
  provider codex
  profile "repo-writer"
  capacity 2
  capabilities ["agent.tell"]
  skills ["whipplescript-author"]
}
```

- `provider` names the family (`codex`, `claude`, `pi`, `fixture`).
- `profile` describes authority, such as `repo-reader` or `repo-writer`.
- `capacity` bounds concurrent turns.
- `capabilities` lists what the agent may be asked to do.
- `skills` attaches context bundles to the agent's turns.

`tell implementer ...` creates an `agent.tell` effect; a worker executes it
through whatever the provider configuration binds `codex` to. Swapping the
fixture provider for a real one changes configuration, not rules.

For the rare case where one provider family needs several configured
endpoints, declare named endpoints with `harness` and bind agents with
`agent ... using harness`; harness configs bind by harness name.

## Credentials and configuration

Source never holds credentials. Provider configuration binds a source-level
provider id to a concrete surface and a credential *reference*:

```json
{
  "provider_id": "codex",
  "provider_kind": "codex",
  "surface": "codex_app_server",
  "credentials_ref": "env:OPENAI_API_KEY"
}
```

`credentials_ref` is an environment variable name, keychain handle, or secret
id — never a value. Validate configs with:

```sh
WHIPPLESCRIPT_PROVIDER_CONFIGS=examples/provider-configs/native/native.example.json \
scripts/check-native-provider-configs.sh
```

The checker records redacted results only; it never prints secret values,
prompts, or raw provider responses.

## Native providers

Native adapters for Codex, Claude, and Pi are experimental: setup,
cancellation behavior, artifact capture, and evidence shape may change.

| Provider | Surface | Identity | Cancellation | Evidence |
| --- | --- | --- | --- | --- |
| Codex | app-server JSON-RPC | thread + turn id | `turn/interrupt` | notifications, tool/approval summaries, diff metadata |
| Claude | Agent SDK sidecar | session id | cooperative SDK cancellation | stream messages, hook/tool summaries, usage |
| Pi | RPC mode | session + parent id | RPC `abort` | RPC events, model metadata, terminal summaries |

After a crash leaves native runs interrupted, `whip recover <instance>`
reconciles them from persisted provider evidence.

Real-provider smoke tests are opt-in:

```sh
WHIPPLESCRIPT_E2E_REAL_PROVIDERS=1 \
WHIPPLESCRIPT_REAL_PROVIDERS=loft,coerce,codex \
scripts/check-real-providers.sh
```

Reports land in `target/real-provider-smoke-report.md` and
`target/real-provider-reports/<provider>.json`. Strict native validation and
destructive provider tests have additional gates; see
[troubleshooting](troubleshooting.md#native-provider-strict-mode-fails).

## Native coerce (real model decisions)

By default `coerce` / `decide` run against the deterministic fixture, so
`dev`, `worker`, and CI need no credentials. To run a real model decision,
opt in with environment variables:

```sh
# OpenAI (Responses API, JSON-schema structured output)
WHIPPLESCRIPT_COERCE_PROVIDER=openai OPENAI_API_KEY=sk-... \
  whip dev workflow.whip --provider fixture

# Anthropic (Messages API, single forced tool)
WHIPPLESCRIPT_COERCE_PROVIDER=anthropic ANTHROPIC_API_KEY=sk-ant-api... \
  whip dev workflow.whip --provider fixture
```

The output JSON Schema is built from the declared `coerce` output type and sent
as the provider's native structured-output constraint, so the result is parsed
straight into the typed value. Useful knobs: `WHIPPLESCRIPT_COERCE_MODEL`,
`WHIPPLESCRIPT_COERCE_BASE_URL`, `WHIPPLESCRIPT_COERCE_MAX_TOKENS`,
`WHIPPLESCRIPT_COERCE_TIMEOUT_SECS`.

Credentials:

- **OpenAI** uses `OPENAI_API_KEY` against `api.openai.com`, or — with no key —
  the Codex OAuth token already in `~/.codex/auth.json`, which routes to the
  ChatGPT-plan codex backend (`chatgpt.com/backend-api/codex/responses`, SSE; the
  model comes from `WHIPPLESCRIPT_COERCE_MODEL` or `~/.codex/config.toml`). This
  path is validated to honor structured outputs; it bills your ChatGPT plan.
  OpenAI publicly permits codex-endpoint use.
- **Anthropic** requires a console API key (`sk-ant-api*`, `ANTHROPIC_API_KEY` or
  `whip auth set anthropic`). A Claude Code OAuth token (`sk-ant-oat*`) is
  rejected with a clear message — reusing it for the API is a terms gray area.

If the provider is set but no credential resolves, the coerce effect fails with
a clear message instead of silently using a fixture.

## `whip auth`

whip does not run its own login — your environment is already authenticated
(`codex login`, the Claude CLI). coerce reads those existing credentials; use
`whip auth` to inspect what resolves or to store an explicit API key:

```sh
whip auth status                           # show what resolves (redacted) + source
whip auth set anthropic sk-ant-api03-...   # store an explicit coerce credential
whip auth set openai     sk-proj-...
```

`whip auth set` writes an owner-only (`0600`) config file at
`$WHIPPLESCRIPT_CONFIG_DIR/auth.json` (else `$XDG_CONFIG_HOME/whipplescript/` or
`~/.config/whipplescript/`). Coerce credential precedence is **environment
variable → stored config → Codex OAuth token** (OpenAI only), so an env var
always overrides a stored key.

There are two distinct credential needs, but only one is whip's job. **coerce /
decide** use the credential resolved above. **Harnesses** (Codex/Claude agent
turns) authenticate through their own provider CLI (`codex login`; the Claude
CLI's `/login`) — whip does not re-run those flows; it reuses whatever the
environment already has.

## Effect kinds

| Effect | Created by | Executed as |
| --- | --- | --- |
| `agent.tell` | `tell` | an agent turn |
| `coerce` | `coerce` / `decide` | a typed model decision |
| `human.ask` | `askHuman` | an inbox item awaiting a human answer |
| `queue.*` | `file` / `claim` / `release` / `finish` | work-queue operations |
| `timer.wait` | `timer` | a delay that fires when due |
| `exec.command` | `exec` | dev raw command or hosted SHA-256-pinned script capability |
| `signal.emit` | `emit signal` | typed signal injection into another instance |
| `lease.acquire` / `lease.release` | `acquire` / `release` | workspace-scoped coordination lock/semaphore operations |
| `ledger.append` | `append ... to <ledger>` | durable partitioned append-log write |
| `counter.consume` | `consume ... amount ...` | bounded budget consumption |
| `workflow.invoke` | `invoke` | a child workflow instance |
| namespaced capabilities | `call package.capability` | a package capability provider |

## Packages

A package registers capabilities, providers, profiles, schemas, resources, and
optional skills. First-class package manifests separate libraries,
capabilities, providers, profiles, and bindings. The contract is
package/library/provider: packages expose explicit effects; they never add
hidden control flow or new grammar.

```whip
use memory

rule fetch_context
  when WorkItem as item where item.status == "queued"
=> {
  call memory.query for item as context

  after context succeeds as found {
    tell worker as turn "Use this context: {{ found.summary }}"
  }
}
```

The `call` creates a durable `capability.call` effect like any other and
requires the package capability (`memory.query` in the example). When the
locked package contract declares `validation: runtime_boundary`, provider
output is checked against `output_schema` before `capability.call.succeeded` is
derived; mismatches fail the effect instead of becoming workflow facts.
Validate and pin package manifests before use:

```sh
whip package check examples/packages/memory.json
whip package lock --output whip.lock examples/packages/memory.json
whip dev workflow.whip --package-lock whip.lock
```

Package packaging is still experimental. Treat package manifests as part of the
checked source contract: pin them with `whip package lock`, commit the lockfile
with the workflow, and update both together.

## Practical advice

- Validate orchestration with the fixture provider before touching real
  providers; assertions prove the workflow reaches the intended state.
- Keep provider identity in source metadata (`AgentRef<...>`, agent
  declarations) — never let a model's text output choose the route.
- When a real provider misbehaves, read `effects`, `runs`, `diagnostics`,
  and `evidence` before reading adapter code.
