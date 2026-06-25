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

- `provider` names the family (`owned`, `codex`, `claude`, `pi`, `fixture`).
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

## Owned harness (`provider owned`)

The `codex`/`claude`/`pi` families **delegate** the whole agent turn to a
provider's own harness; whip captures a redacted summary and cannot truly
enforce what the turn does. The **owned** harness instead runs the tool-use loop
itself: the model requests a tool, *whip* executes it, feeds the result back, and
loops to a single terminal. Because whip is the executor, coordination primitives
become an *enforced* envelope on the turn rather than advisory metadata. See
[DR-0024](https://github.com/jamesjscully/whipplescript/blob/main/spec/decision-records/0024-owned-brokered-agent-harness.md)
for the design.

```whip
agent helper {
  provider owned
  profile "repo-writer"
  capacity 1
}
```

Tool calls inside the turn are recorded as **evidence**, never as
rule-matchable facts; only the single `agent.turn.<status>` terminal becomes a
fact, so `when <agent> completed turn` / `after <turn> succeeds` work exactly as
with the delegating families.

Current scope is **experimental**:

- Tools: `read`, `write`, `edit`, `grep`, `find`, `ls`, executed through the
  `file store` path policy (no absolute/`..` escape), plus `bash` — **default-deny**:
  a command runs only if it matches an allow-list prefix in
  `WHIPPLESCRIPT_HARNESS_BASH_ALLOW` (e.g. `git,cargo,ls`), runs with the
  workspace as cwd, and is killed past a timeout. With no allow-list, every `bash`
  command is refused.
- Tracker tools (`list_todos`/`add_todo`/`update_todo`), offered only when
  `WHIPPLESCRIPT_HARNESS_TRACKER=<queue>` is set: the agent participates in the
  durable work tracker (files/updates items the workflow's rules observe).
  Per the refined I3, these write shared tracker *state*, never rule-matchable
  facts; `add_todo` items are attributed to the agent (`source: "agent"`).
- Workspace: the turn operates under `WHIPPLESCRIPT_HARNESS_WORKSPACE` (default:
  the current directory).
- Model: set `WHIPPLESCRIPT_HARNESS_PROVIDER` (`openai` or `anthropic`) plus
  `WHIPPLESCRIPT_HARNESS_MODEL` to drive the loop with a **live** model
  (credentials reused from the coerce resolver: env var → `whip auth` → Codex
  OAuth; knobs `WHIPPLESCRIPT_HARNESS_BASE_URL` / `_MAX_TOKENS` / `_TIMEOUT_SECS`).
  Unset, a deterministic credential-free **fixture** client drives the loop so
  `dev`/CI need no credentials (`WHIPPLESCRIPT_OWNED_FIXTURE_TOOL=read:<path>`
  makes it exercise one tool call).
- Envelope: a per-turn model-step budget (`WHIPPLESCRIPT_HARNESS_MAX_STEPS`,
  default 16) bounds the loop, and the turn holds a durable workspace lease for
  the duration (a contended workspace blocks, recoverable, rather than racing).
- Context: the model's working context is compacted on long turns (old tool
  results elided to references, the System message + first instruction + recent
  window kept verbatim); this touches only what the model re-reads — the durable
  observation stream is complete and unaffected.
- Still later slice: resume-from-crash. (Full OS-level writable-root confinement
  for `bash` is a refinement over the allow-list.)

```sh
whip --store .whipplescript/owned.sqlite \
  dev examples/owned-harness-demo.whip --provider owned --until idle
```

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

The Codex and Claude adapters are now **optional Cargo features** (`codex`,
`claude`), on by default. The owned harness is the built-in path; a slimmer
binary can drop the delegating adapters:

```sh
# whip without the Codex/Claude delegating adapters
cargo install --path crates/whipplescript-cli --no-default-features
```

A workflow that selects `provider codex`/`claude` against a binary built without
that feature fails the turn with a clear "provider not built into this whip"
message rather than silently falling back.

| Provider | Surface | Identity | Cancellation | Evidence |
| --- | --- | --- | --- | --- |
| Codex | app-server JSON-RPC | thread + turn id | `turn/interrupt` | notifications, tool/approval summaries, diff metadata |
| Claude | Agent SDK sidecar | session id | cooperative SDK cancellation | stream messages, hook/tool summaries, usage |
| Pi | RPC mode | session + parent id | RPC `abort` | RPC events, model metadata, terminal summaries |

After a crash leaves native runs interrupted, `whip recover <instance>`
reconciles them from persisted provider evidence.

The agent-turn model is never hardcoded. For the Codex app-server surface it
resolves provider config `default_model` → `WHIPPLESCRIPT_CODEX_APP_SERVER_MODEL`
→ the `model` in `~/.codex/config.toml`; with none of those set the turn fails
with a clear "no model configured" message rather than guessing a default.

### Provider errors in failure diagnostics

Native evidence is shape-redacted: prompts and model output are recorded as
JSON *shape* only, never values, so a turn's contents never leak into the run
store. Provider **control-plane** errors are the deliberate exception. When a
turn fails for an operational reason — usage-limit exceeded, auth rejected,
model not found — that reason is operational metadata, not model output, so it
crosses the redaction boundary into the failure diagnostic and the effect's
evidence summary. The reason is capped (300 chars) and run through the same
secret redaction as everything else, so an "auth failed" message can name the
cause without echoing a token. Read `whip diagnostics <instance>` /
`whip effects <instance>` on a failed agent turn to see it.

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
