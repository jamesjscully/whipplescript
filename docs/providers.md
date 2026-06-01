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

## Credentials And Configuration

Do not put credentials in `.whip` source. Source should name logical agents,
profiles, capabilities, and plugins. Runtime/provider configuration should hold
credential references and execution details.

For now, real-provider smoke scripts document their own required environment
variables. Treat those scripts as validation tools, not a stable deployment
interface.

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

## Practical Advice

- Start with fixture provider runs.
- Add assertions to prove the workflow reached the expected state.
- Inspect `effects`, `runs`, `diagnostics`, and `evidence` before debugging
  provider code.
- Keep provider identity in source metadata, not in model-generated text.
- Treat real providers as experimental until your target adapter has its own
  smoke test and recovery story.
