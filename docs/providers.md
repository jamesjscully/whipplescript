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

- `provider` names the family (`owned`, `codex`, `claude`, `fixture`).
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

The `codex`/`claude` families **delegate** the whole agent turn to a
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
  file tools are offered only when the turn carries matching file-store grants
  (`edit` needs both read and write). When an IFC governance envelope is active,
  those granted file stores must also be governed by the envelope before the
  turn is admitted. A `tell ... requires [...]` list further narrows the owned
  tool surface for known harness capabilities (`repo.read`, `repo.write`,
  `command.run`, `tracker.*`, and `workflow.invoke`); the scheduler already
  requires those values to be declared by the target agent. `bash` is offered
  only with `with access to command { run }`, and runs only when the
  profile/required-capability set permits `command.run` AND the turn carries
  that grant. It then executes in the **in-isolate Bashkit virtual shell**
  (DR-0039) over the governed workspace file surface — **not** a real OS shell:
  no `fork`/`exec`, no ambient filesystem, no ambient network. Ordinary shell
  features (pipes, substitution, redirection to workspace files, a fixed set of
  builtins) work, but they cannot reach outside the workspace. Every file the
  command reads, writes, or deletes crosses the **same labeled-store policy
  boundary as the file tools** (read/write globs of the granted stores), and a
  path outside the workspace simply does not exist to it. When an IFC governance
  envelope is active, the `command` resource must also be governed. Because the
  interpreter has no OS reach, there is no command allow-list to configure — the
  sandbox plus the workspace policy IS the enforcement boundary.
- Tracker tools (`list_todos`/`add_todo`/`update_todo`), offered only when
  `WHIPPLESCRIPT_HARNESS_TRACKER=<tracker>` is set: the agent participates in the
  durable work tracker (files/updates items the workflow's rules observe).
  `list_todos` remains read-only and ungated; mutating tools are surfaced and
  executed only when the turn delegates tracker authority with
  `with access to tracker { file }` for `add_todo`, or `claim`/`finish`/`release`
  (or `update`) for the corresponding `update_todo` status transition. Registered
  profiles may further narrow this with `tracker.file`, `tracker.claim`,
  `tracker.finish`, `tracker.release`, `tracker.update`, or `tracker.write`.
  When an IFC governance envelope is active, mutating tracker authority also
  requires the envelope to govern the `tracker` resource.
  Per the refined I3, these write shared tracker *state*, never rule-matchable
  facts; `add_todo` items are attributed to the agent (`source: "agent"`).
- Sub-workflow tools ([DR-0025](https://github.com/jamesjscully/whipplescript/blob/main/spec/decision-records/0025-workflows-as-agent-tools.md)):
  a `@tool` workflow becomes a typed agent tool (its `input` contract is the
  tool's JSON schema) that the model may **invoke synchronously** mid-turn. The
  call blocks the turn until the sub-workflow reaches its terminal, then returns
  its `output` payload (a non-`completed` terminal surfaces as a tool error). A
  `@tool` workflow is held to a **convergence check** (it must terminate — no
  `@service`, no external-signal/`@external`/inbound-message readiness, no
  `human.ask`, and for v1 no nested `invoke`), so the synchronous block is bounded:
  it provably will not block forever. This is brokering, not shelling out — the
  sub-workflow runs through the runtime with first-class parent↔child lineage,
  durable events, and crash recovery, not as an opaque subprocess. Curation is
  two-sided: a workflow opts in with `@tool`; an agent is **granted** specific
  ones with a `tools [WordCount, OpenPr]` field — the in-program curation surface,
  checked at `whip check` (a granted non-`@tool` is a compile error). Granted
  names resolve against the same program bundle or a `use`d package; the operator
  override `WHIPPLESCRIPT_HARNESS_TOOLS=<path>[,<path>…]` still lists out-of-tree
  `@tool` sources for a turn (merged with the grant, which wins on a name clash).
  Registered profiles and `tell ... requires [...]` may further narrow the
  model-facing workflow-tool surface with `workflow.invoke`; direct calls are
  refused at dispatch if that capability is not present in the resolved
  profile/required-capability policy. Cross-package `@tool` imports also require
  the active IFC envelope to govern the package invoke door
  `invoke:<package>/<tool>` before the tool is offered.
  A package exports a `@tool` workflow by shipping its source and listing it in
  the manifest (`"workflow_tools": [{ "name": …, "source": … }]`); the package
  contract then carries a convergence-eligibility **attestation** (the tool's
  derived input/output schema) and an information-flow surface that includes the
  package invoke membrane door `invoke:<package_id>/<tool>`. Under a governed
  envelope, that invoke door must be governed before the imported tool can be
  checked or offered to the model. A consumer's grant is checked against the
  contract and the tool is driven from the package's shipped source. See
  `examples/subworkflow-tool-consumer.whip` (grants the `toolkit` package's
  `EchoText`).
- Workspace: the turn operates under `WHIPPLESCRIPT_HARNESS_WORKSPACE` (default:
  the current directory).
- Model: set `WHIPPLESCRIPT_HARNESS_PROVIDER` (`openai` or `anthropic`) plus
  `WHIPPLESCRIPT_HARNESS_MODEL` to drive the loop with a **live** model
  (credentials reused from the coerce resolver: env var → `whip auth` → Codex
  OAuth; knobs `WHIPPLESCRIPT_HARNESS_BASE_URL` / `_MAX_TOKENS` / `_TIMEOUT_SECS`).
  Unset, a deterministic credential-free **fixture** client drives the loop so
  `dev`/CI need no credentials (`WHIPPLESCRIPT_OWNED_FIXTURE_TOOL=read:<path>`
  makes it exercise one tool call).
- Provider configs may list `profile_ids`; a non-empty list is enforced as an
  endpoint allow-list before provider launch. A mismatched agent profile leaves
  the effect recoverably `blocked` with category `provider_config`.
- Envelope: a per-turn model-step budget (`WHIPPLESCRIPT_HARNESS_MAX_STEPS`,
  default 16) bounds the loop, and the turn holds a durable workspace lease for
  the duration (a contended workspace blocks, recoverable, rather than racing).
  The lease is keyed on the **unit of work** (the top-level invocation), not the
  individual turn, so a turn that synchronously invokes a sub-workflow tool shares
  its own root's lease re-entrantly rather than self-deadlocking on it; only a
  *different* unit of work contends ([DR-0025](https://github.com/jamesjscully/whipplescript/blob/main/spec/decision-records/0025-workflows-as-agent-tools.md)).
- Context: the model's working context is compacted on long turns (old tool
  results elided to references, the System message + first instruction + recent
  window kept verbatim); this touches only what the model re-reads — the durable
  observation stream is complete and unaffected.
- Crash recovery: the turn transcript is persisted after each step; if a turn is
  interrupted, a later worker pass resumes it from that projection (a dangling
  final tool-call is dropped so the model re-decides) rather than re-running from
  scratch.
- Refinement still open: full OS-level writable-root confinement for `bash` (the
  allow-list is the current boundary).

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

### Spend prices (`prices` block)

A provider-config file may carry a top-level `prices` array — the spend
price table the improve subsystem consumes (`--spend-cap`, `std.spend`):

```json
{
  "providers": [...],
  "prices": [
    {"provider": "anthropic", "model": "claude-sonnet-5",
     "input_per_mtok_usd": 3.0, "output_per_mtok_usd": 15.0}
  ]
}
```

Rates are USD per million tokens, per (provider, model), input and output
separately. Pricing is **config-only**: whip ships no built-in rates (a
stale built-in would misprice spend silently). Usage with no matching
entry records honestly as `unpriced` with cost 0 — visible in the spend
events, and unable to bind a spend cap. Pricing happens at record time;
history is never repriced. The maintained example lives at
`examples/provider-configs/native/native.example.json` — verify its rates
against your provider's current price sheet before relying on the cap.

> **Unpriced models under a `--spend-cap`.** Because the cap binds only
> *priced* cost, a paid model with no `prices` entry would let the cap
> silently never bind. A campaign that spends under a cap while any usage is
> unpriced now ends with a **warning** (and a `campaign.spend_cap_unpriced`
> record event) naming the gap. This matters most for arbitrary
> `openai-generic` endpoints: add a `prices` entry for the model to make the
> cap enforceable — or, for a genuinely-free local model (Ollama, local
> vLLM), add an entry with `input_per_mtok_usd: 0` / `output_per_mtok_usd: 0`
> to declare it free and silence the warning.

## Native providers

Native adapters for Codex and Claude are experimental: setup,
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
WHIPPLESCRIPT_REAL_PROVIDERS=coerce,codex \
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

## OpenAI-compatible & local models (Ollama, vLLM, OpenRouter, Groq, …)

Anything that speaks the OpenAI Chat Completions wire format
(`/v1/chat/completions`) works through the **`openai-generic`** provider — local
runtimes (Ollama, LM Studio, vLLM, llama.cpp) and hosted gateways (OpenRouter,
Together, Groq, Fireworks, DeepSeek, Azure OpenAI). One provider, selected by
`base_url`.

> **`base_url` must include the API version segment** (usually `/v1`). Unlike the
> `openai`/`anthropic` providers — whose base is the bare host and whip appends
> `/v1/...` — `openai-generic` appends only `/chat/completions`, following the
> OpenAI-SDK convention that the version lives in `base_url`. So use
> `http://localhost:11434/v1`, **not** `http://localhost:11434`.

### Coerce / decide (structured output)

```sh
# A local Ollama model making a typed `coerce` decision:
WHIPPLESCRIPT_COERCE_PROVIDER=openai-generic \
WHIPPLESCRIPT_COERCE_BASE_URL=http://localhost:11434/v1 \
WHIPPLESCRIPT_COERCE_MODEL=llama3.1:8b \
OPENAI_API_KEY=ollama \
  whip dev workflow.whip --provider fixture --until idle
```

`OPENAI_API_KEY` carries the bearer token; endpoints that don't check it (Ollama)
accept any non-empty value. Confirm the resolved config with
`whip --json coercion status`. The declared `coerce` output type is sent as a
`response_format: json_schema` constraint; endpoints that support it (Ollama,
vLLM, OpenAI) return schema-conforming JSON parsed straight into the typed value.

### Agent turns (owned harness)

Point the [owned harness](#owned-harness-provider-owned) at the same endpoint
through a **provider-profiles** file (`WHIPPLESCRIPT_PROVIDER_PROFILES`), keyed by
the agent's declared `profile` (with `default` as the catch-all):

```json
{
  "default": {
    "provider": "openai-generic",
    "model": "llama3.1:8b",
    "base_url": "http://localhost:11434/v1",
    "api_key_env": "OPENAI_API_KEY",
    "max_tokens": 1024
  }
}
```

```sh
WHIPPLESCRIPT_PROVIDER_PROFILES=./profiles.json OPENAI_API_KEY=ollama \
  whip dev workflow.whip --provider owned --until idle
```

whip drives the tool-use loop itself (tools are sent as
`{type:"function", ...}`, results fed back as `role:"tool"`), so tool quality
tracks the model — small local models will tool-call poorly, but the wire path is
identical to any hosted OpenAI-compatible model.

> **On the Durable Object host**, outbound model calls must be HTTPS except for
> loopback. A hosted compatible endpoint (OpenRouter/Together/…) works as-is; a
> non-loopback plain-HTTP local endpoint is refused.

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
| `schema.coerce` | `coerce` / `decide` | a typed model decision |
| `human.ask` | `askHuman` | an inbox item awaiting a human answer |
| `tracker.file` / `tracker.claim` / `tracker.release` / `tracker.finish` | `file` / `claim` / `release` / `finish` | durable work-tracker operations |
| `timer.wait` | `timer` | a delay that fires when due |
| `exec.command` | `exec` | dev raw command or hosted SHA-256-pinned script capability |
| `event.emit` | `emit <event> { ... }` | a typed event injected into this instance's fact stream |
| `signal.emit` | `emit signal` | typed signal injection into another instance |
| `lease.acquire` / `lease.release` | `acquire` / `release` | workspace-scoped coordination lock/semaphore operations |
| `ledger.append` | `append ... to <ledger>` | durable partitioned append-log write |
| `counter.consume` | `consume ... amount ...` | bounded budget consumption |
| `file.read` / `file.write` | `read text from <store> at <path>` / `write text to <store> at <path>` | durable read/write through a `file store` path policy |
| `file.import` / `file.export` | `import <fmt> <Schema> from <store> ...` / `export <fmt> <Schema> to <store> ...` | structured records read from / written to a file-store path |
| `workflow.invoke` | `invoke` | a child workflow instance |
| namespaced capabilities | `call package.capability` | a package capability provider |

## Packages

A package registers capabilities, providers, profiles, schemas, resources, and
optional skills. First-class package manifests separate libraries,
capabilities, providers, profiles, and bindings. The contract is
package/library/provider: packages expose explicit effects; they never add
hidden control flow or new grammar.

```whip
use std.memory

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
Standard-library packages such as `std.memory` ship embedded in the platform:
the `use` import is all a workflow needs. Third-party packages must be
validated and pinned before use (a lock may never claim a `std.*` name — the
embedded manifest always wins):

```sh
whip package check examples/packages/notes.json
whip package lock --output whip.lock examples/packages/notes.json
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
