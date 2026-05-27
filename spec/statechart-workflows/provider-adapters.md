# Provider Adapters

Status: target spec

Provider adapters are the concrete harness implementations that turn a durable
Armature agent invocation into a real process such as Codex, Claude Code, Pi, or
a deterministic local command.

The workflow language must not name provider binaries directly in normal
statechart logic. A workflow declares roles and lifecycle. Harness policy binds
those roles to providers. Provider adapters perform the launch, observation,
artifact capture, and completion reporting.

## Design Boundary

There are three separate concepts:

```text
workflow agent role      semantic role in the statechart
harness profile          authority and intent selected for that role
provider adapter         concrete runner that launches a tool/process
```

Example workflow source:

```armature
agent worker = codingAgent {
  profile "repo-writer"
  maxActive 1
}

state ready {
  on begin as task {
    start worker {
      goal task.goal
      success "Tests pass and the implementation is ready for review."
    }
    goto working
  }
}
```

This does not mean "launch Codex." It means "create a durable invocation for
the `worker` role using the `repo-writer` profile." The harness decides whether
that role/profile is implemented by Codex, Claude Code, Pi, an enterprise
broker, or a local deterministic command.

Provider names are valid in harness policy, not as ordinary workflow intent:

```json
{
  "mode": "custom",
  "defaultProfile": "repo-writer",
  "profiles": {
    "repo-writer": {
      "description": "Use for scoped repository implementation after the task is clear.",
      "provider": "codex",
      "timeoutSeconds": 1800,
      "filesystem": "workspace_write",
      "network": "denied",
      "allowedEnv": ["OPENAI_API_KEY"],
      "allowedTools": ["read", "edit", "test"],
      "enforcement": "native_or_best_effort"
    }
  }
}
```

## Language Interface

### Agent Declarations

The target source syntax should make `codingAgent` read as a declaration kind,
not a function call:

```armature
agent worker = codingAgent {
  profile "repo-writer"
  maxActive 2
}
```

For compatibility during migration, the parser may continue accepting
`codingAgent()` as an alias. Diagnostics and generated examples should prefer
the no-parentheses form.

`codingAgent` means:

- the runtime may persist `start` effects for this agent in the native
  `agent_invocations` ledger
- the harness may claim those invocations
- the agent must eventually produce a typed `finished` event or be recovered by
  timeout/lease logic
- the concrete provider is not determined by the workflow source

It does not mean:

- run a shell command
- run Codex
- run Claude Code
- run Pi
- grant filesystem/network authority by itself

### Profile Selection

Every production workflow agent should name a profile:

```armature
agent researcher = codingAgent {
  profile "research"
  maxActive 2
}

agent implementer = codingAgent {
  profile "repo-writer"
  maxActive 1
}
```

Omitting `profile` is allowed only when harness policy supplies a default.
Validation with `--profile-policy` should warn in local/permissive mode and
error in governed modes when no profile/default can be resolved.

Profile names are semantic. They are intentionally not provider names. A team
may map `repo-writer` to Codex locally, Claude Code in CI, and an enterprise
broker in production.

### Start Payload

`start` payloads are provider-neutral JSON-like records:

```armature
start implementer {
  goal task.goal
  files task.files
  constraints task.constraints
  success "All relevant tests pass and the result is summarized."
}
```

The harness converts this payload into the provider prompt/input. The stable
harness environment includes:

```text
ARMATURE_WORKFLOW_ID
ARMATURE_INVOCATION_ID
ARMATURE_AGENT
ARMATURE_INPUT_JSON
ARMATURE_PROMPT
ARMATURE_RUN_DIR
```

`ARMATURE_PROMPT` is a convenience projection. If the payload contains a
`message` string, it is that string. Otherwise it is the serialized input JSON.
Provider adapters may build richer prompts from `ARMATURE_INPUT_JSON`, but the
workflow language should not embed provider-specific prompt wrappers.

### Completion Event

The default completion convention remains a typed `finished` event:

```armature
event finished {
  id string
  name string
  status string
  summary string
  exitCode int?
}
```

Rules:

- `id` is the Armature invocation id.
- `name` is the declared Armature agent name, not necessarily a provider run
  name.
- `status` is one of `succeeded`, `failed`, `timed_out`, or `cancelled`.
- `summary` is short operator-facing text.
- stdout/stderr and provider metadata are artifacts referenced from harness
  status, not stuffed into the workflow event.
- completion payloads must validate before being enqueued.

The active-invocation projection retires work only after a processed
`finished` event. A workflow that wants to start the next `maxActive 1` run
immediately after a completion should use a separate internal event or a runtime
semantic that explicitly retires before the next start. This is a known UX
pressure from dogfooding and should be revisited.

## Adapter Trait

The internal provider adapter interface should be provider-neutral:

```rust
trait ProviderAdapter {
    fn name(&self) -> &'static str;
    fn capabilities(&self) -> ProviderCapabilities;
    fn build_launch(&self, invocation: &AgentInvocation, profile: &ResolvedProfile)
        -> Result<LaunchPlan, AdapterError>;
    fn run(&self, launch: LaunchPlan) -> Result<ProviderRunResult, AdapterError>;
}
```

The MVP may execute synchronously in `run`; the contract should not assume that
forever. A later long-running adapter can split `run` into `start`, `observe`,
`cancel`, and `collect` while preserving the same durable ledger.

Conceptual interface:

```text
resolve(profile, agent, input) -> LaunchPlan
start/run(LaunchPlan) -> ProviderRunResult
collect(result) -> CompletionPayload + artifacts
```

### LaunchPlan

```json
{
  "provider": "codex",
  "program": "codex",
  "args": ["--sandbox", "workspace-write", "exec", "{{prompt}}"],
  "cwd": ".",
  "timeoutSeconds": 1800,
  "envAllowlist": ["OPENAI_API_KEY"],
  "requestedAuthority": {
    "filesystem": "workspace_write",
    "network": "denied",
    "tools": ["read", "edit", "test"]
  },
  "enforcedAuthority": {
    "filesystem": "workspace_write",
    "network": "web_search_not_enabled",
    "providerCliFlagsApplied": true
  },
  "warnings": []
}
```

### ProviderRunResult

```json
{
  "status": "succeeded",
  "exitCode": 0,
  "stdoutPath": ".armature/runs/inv_123/stdout.log",
  "stderrPath": ".armature/runs/inv_123/stderr.log",
  "summary": "Implemented the requested change and tests passed.",
  "providerRunId": "inv_123",
  "metadata": {
    "provider": "codex",
    "command": ["codex", "--sandbox", "workspace-write", "exec", "..."]
  }
}
```

## Provider Capabilities

Each adapter must report what it can enforce. Armature must not claim stronger
authority than the adapter can actually supply.

```json
{
  "provider": "codex",
  "supports": {
    "filesystem": ["read_only", "workspace_write"],
    "network": ["allowed", "denied"],
    "envAllowlist": true,
    "toolAllowlist": "none",
    "cwd": true,
    "timeout": "harness"
  }
}
```

Authority fields:

```text
filesystem   provider_default | none | read_only | workspace_write
network      provider_default | denied | allowed
allowedEnv   process environment allowlist
allowedTools semantic provider-specific tool allowlist
timeout      enforced by Armature harness unless provider has stronger support
```

Enforcement evidence values:

```text
native                  provider flag or harness mechanism enforces it
best_effort             Armature can only avoid granting obvious authority
external                an external wrapper/sandbox is responsible
unsupported             requested but not enforced
provider_default        intentionally delegated to provider defaults
```

## Concrete MVP Adapters

### Command Adapter

Purpose:

- deterministic local tests
- dogfood fixtures
- externally sandboxed enterprise wrappers

Policy:

- allowed by default only in permissive mode
- denied in separated/custom mode unless an explicit profile uses provider
  `command` and `allowCommandProvider` is true
- always records that filesystem/network authority is external-process unless
  wrapped by a declared external enforcement backend

Default command:

None. `command` always requires an explicit command array.

### Codex Adapter

Provider: `codex`

Default launch:

```text
codex <authority flags> exec {{prompt}}
```

Target flag mapping:

```text
filesystem read_only       -> --sandbox read-only
filesystem none            -> --sandbox read-only plus warning until a stronger no-fs mode exists
filesystem workspace_write -> --sandbox workspace-write
network allowed            -> --search
network denied             -> omit --search and record web_search_not_enabled
```

Notes:

- Omitting `--search` is not the same as proving all network access is denied.
  Record this as limited or best-effort unless Codex provides a documented
  network-deny flag for the execution mode being used.
- If a profile supplies a custom Codex command, Armature must record
  `providerCliFlagsApplied: false` unless it can inspect and prove the flags.

### Claude Code Adapter

Provider: `claude-code`

Accept `claude` as a temporary alias in config, but generated examples and
docs should say `claude-code`.

Default launch:

```text
claude <authority flags> -p {{prompt}}
```

Target flag mapping:

```text
filesystem read_only       -> --permission-mode plan
filesystem none            -> --permission-mode plan
filesystem workspace_write -> --permission-mode acceptEdits
allowedTools               -> --allowedTools <comma-list>
network denied             -> best-effort warning until a stable native flag is validated
```

Notes:

- `plan` mode is a permission posture, not a complete filesystem sandbox.
- Tool allowlists are provider-specific and should be treated as native only
  for documented Claude Code tools.

### Pi Adapter

Provider: `pi`

Default launch:

```text
pi run {{prompt}}
```

Current enforcement:

- best-effort only until stable Pi sandbox flags are documented and tested
- profile restrictions must be recorded as requested but not strongly enforced

Pi should remain available for dogfood only when the operator intentionally
selects it. It should not be the default enterprise provider until enforcement
support is validated.

## Harness Policy

The product path should be one harness policy document that can fully bind
profiles to providers. `--config` may remain as a deterministic/local override,
but it should not be required when `--profile-policy` contains enough provider
information.

Target CLI:

```sh
armature harness run workflow.armature \
  --store workflow.sqlite \
  --profile-policy .armature/harness-policy.json \
  --drive-workflow
```

Compatibility/local test CLI:

```sh
armature harness run workflow.armature \
  --store workflow.sqlite \
  --config dogfood/ralph-wiggum/harness.json \
  --drive-workflow
```

Resolution order:

```text
1. workflow agent profile
2. profile-policy defaultProfile, if agent omitted profile
3. profile-policy profile definition
4. optional local config override for deterministic tests
5. provider adapter default launch template
```

Mismatch rules:

- if config and profile policy both name a provider, they must match
- custom command overrides for Codex/Claude/Pi are allowed only when policy
  permits best-effort/custom command evidence
- command provider requires explicit policy authority outside permissive mode

## Observability

Every provider launch must append a harness event with:

```text
agent
requested profile
resolved profile
provider
command after template expansion, with secrets redacted
requested authority
enforced authority
warnings
stdout/stderr artifact paths
exit status or timeout
completion event id, if enqueued
```

`harness status --json` should expose these records without requiring users to
inspect SQLite manually.

## Security Notes

- The workflow language is the capability gate for orchestration logic, not for
  arbitrary provider behavior after launch.
- Provider adapters must use process env allowlists when profile policy
  supplies `allowedEnv`.
- Command provider is powerful and should be treated as an escape hatch.
- Real provider adapters are still only as strong as the provider CLI's own
  sandbox semantics.
- Armature should prefer clear warnings over false claims of enforcement.

## Dogfood Requirements

Maintain two kinds of dogfood fixtures:

1. Deterministic command-provider fixtures for CI and language ergonomics.
2. Real provider fixtures for Codex, Claude Code, and Pi that are opt-in and
   record provider evidence.

The Ralph Wiggum fixture belongs to the first category. It should be labeled as
a deterministic command-provider fixture, not as evidence that real coding
agents are wired correctly.

