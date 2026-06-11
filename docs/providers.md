# Providers & plugins

Workflows describe coordination policy; providers and plugins are how that
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
WHIPPLESCRIPT_REAL_PROVIDERS=loft,baml,codex \
scripts/check-real-providers.sh
```

Reports land in `target/real-provider-smoke-report.md` and
`target/real-provider-reports/<provider>.json`. Strict native validation and
destructive provider tests have additional gates; see
[troubleshooting](troubleshooting.md#native-provider-strict-mode-fails).

## Effect kinds

| Effect | Created by | Executed as |
| --- | --- | --- |
| `agent.tell` | `tell` | an agent turn |
| `baml.coerce` | `coerce` / `decide` | a typed model decision |
| `human.ask` | `askHuman` | an inbox item awaiting a human answer |
| `queue.*` | `file` / `claim` / `release` / `finish` | work-queue operations |
| `timer.wait` | `timer` | a delay that fires when due |
| `exec.command` | `exec` | dev raw command or hosted SHA-256-pinned script capability |
| `workflow.invoke` | `invoke` | a child workflow instance |
| namespaced capabilities | `call plugin.capability` | a plugin handler |

## Plugins

A plugin registers capabilities, providers, profiles, schemas, resources, and
optional skills. The contract is simple: plugins expose explicit effects;
they never add hidden control flow or new grammar.

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

The `call` creates a durable effect like any other; a provider bound by the
plugin handles it. Plugin packaging is still experimental — the manifest
shape and policy details live in the design-facing
[plugin author guide](../spec/plugin-author-guide.md).

## Practical advice

- Validate orchestration with the fixture provider before touching real
  providers; assertions prove the workflow reaches the intended state.
- Keep provider identity in source metadata (`AgentRef<...>`, agent
  declarations) — never let a model's text output choose the route.
- When a real provider misbehaves, read `effects`, `runs`, `diagnostics`,
  and `evidence` before reading adapter code.
