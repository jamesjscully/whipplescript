# Agent Harness Layer

Status: draft

The harness layer turns durable `agent.tell` effects into real agent turns.

WhippleScript must not pretend an agent turn exists until this layer can claim an
effect, run a provider, capture evidence, and append a completion event.

## Harness Player

A harness player is the runtime worker that executes queued harness effects.
It is intentionally boring:

```text
poll/subscribe for claimable agent.tell effects
claim one effect under policy
resolve the requested profile to a provider adapter
prepare workspace and context
run one provider turn
capture artifacts and evidence
append completion/failure event
release or renew leases
```

The player is not a workflow engine. It does not choose new work, inspect
Loft readiness, retry policy, or decide when the loop is done. Those decisions
belong to rules and external kernels.

## Responsibilities

The harness owns:

```text
provider adapter selection
profile resolution
sandbox setup
worktree / cwd setup
skill/context assembly
agent process/session launch
turn lifecycle observation
stdout/stderr/provider artifact capture
completion event production
lease recovery and timeout handling
```

The rule language owns:

```text
when to request an agent turn
which logical agent role should receive it
what work/prompt/context should be sent
how completion facts affect policy
```

## Provider Adapter Contract

```text
resolve(profile, agent, input) -> LaunchPlan
run(LaunchPlan) -> ProviderRunResult
collect(ProviderRunResult) -> CompletionEvent + artifacts
```

The common provider result shape is intentionally richer than process
stdout/stderr:

```text
provider
provider_session_id
thread_id
turn_id
status
summary
structured_result_json?
changed_files[]
diff_refs[]
artifact_refs[]
tool_calls[]
approval_events[]
usage_json?
started_at
completed_at
```

Each adapter may obtain those fields differently, but the harness must normalize
them before appending completion events. Missing optional fields should be
explicitly represented as unavailable rather than silently dropped.

Initial provider targets:

```text
codex
claude-code
pi
command fixture
enterprise broker
```

`command fixture` is for deterministic tests only. It is not proof that real
coding agents are wired.

The first implementation exposes this boundary as a kernel `AgentHarness` trait
with a deterministic `MockAgentHarness`. The kernel-owned runner claims the
effect, records injected skill provenance, runs the adapter, stores artifacts
and provider evidence, appends the terminal effect completion, and then emits an
`agent.turn.*` event plus fact. Adapters return data; they do not receive store
handles or mutate kernel state directly.

The real provider adapters are not interchangeable command wrappers. Each one
must map WhippleScript's durable turn contract onto the provider's native session
surface, authentication model, persistence model, and artifact semantics.

### Codex Provider

Codex should integrate through the Codex harness surface, not by scraping a UI.
OpenAI documents the Codex App Server as a bidirectional JSON-RPC style API that
exposes the same Codex harness used by the CLI, IDE extension, web runtime, and
desktop app. The CLI also has scriptable modes such as `codex exec`, but those
are better suited for one-shot automation than long-lived UI-grade session
control.

The Codex adapter therefore needs:

- App Server or Codex SDK client bindings.
- Thread creation/resume/fork semantics.
- Event stream handling for model progress, tool calls, diffs, approvals,
  interruptions, and final turn completion.
- Authentication and model/config discovery through the Codex account/session
  surface.
- Artifact capture for transcript, changed files, diffs, command output,
  approvals, and final summary.

The Codex desktop app runs its own app server for its UI. WhippleScript should not
assume that private desktop app server is externally reachable. For local
automation, WhippleScript should launch or connect to a dedicated Codex App Server
or supported SDK surface under explicit operator configuration.

### Claude Provider

Claude should integrate through the Claude Agent SDK, not a generic command
shim. Anthropic documents the Agent SDK as the programmable surface that exposes
the same tools, agent loop, and context management that power Claude Code in
Python and TypeScript. It also has explicit API-key and cloud-provider
authentication modes; third-party products must not depend on a user's
interactive Claude subscription login unless Anthropic explicitly permits it.

The Claude adapter therefore needs:

- TypeScript or Python Agent SDK host process.
- API-key/provider authentication configuration.
- Tool permission mapping from WhippleScript profiles to Claude SDK allowed
  tools, hooks, and working directories.
- Streaming message handling and final result extraction.
- Artifact capture for transcript, tool calls, edits, command output, and usage.

### Pi Provider

Pi should integrate through the Pi extension system. That is the primary surface
for WhippleScript, not a fallback. Compared with Codex App Server and Claude Agent
SDK, Pi's integration shape is expected to be more extension-native: WhippleScript
will likely provide an extension that receives durable turn requests, submits or
routes them through Pi, observes conversation-thread updates, and reports
structured completion metadata back to the harness.

The Pi adapter therefore needs:

- A Pi extension that can receive WhippleScript turn requests.
- A way to correlate WhippleScript `effect_id` / `run_id` with Pi conversation
  thread ids.
- Thread observation or export plumbing for transcripts and evidence.
- Completion detection and summary extraction from Pi conversations.
- Artifact capture for thread snapshots, extension events, user-visible
  messages, and final outcome metadata.

Pi stores conversation threads, so those thread ids and snapshots should be
first-class evidence. A thread-export bridge is useful for audit and recovery,
but it should complement the extension integration rather than replace it.

Provider adapters are replaceable. The rule language addresses logical agents
and profiles; the registry chooses whether that means Codex, Claude Code, Pi,
an enterprise broker, or a test fixture.

## Control Plane Bridge

The harness cannot run until the control plane has materialized ready rule
commits into durable outbox effects. A complete local validation loop needs both:

```text
source + instance event -> ready rule evaluation -> rule commit -> effect outbox
effect outbox -> harness claim -> provider run -> completion event
completion event -> derived facts -> next ready rules
```

`whip run` may remain a "start an instance" command, but local validation needs an
explicit driver such as `whip step`, `whip worker`, or `whip dev` that advances
ready rules and runs configured local providers until idle, stopped, or blocked.

## Agent Turn Lifecycle

```text
effect queued
effect claimed
provider session prepared
turn started
turn streaming/running
turn completed | failed | timed_out | cancelled
completion event appended
facts derived by rules
```

Completion payload must include:

```text
effect_id
run_id
agent
provider
status
summary
artifact_refs
structured_result?
```

Completion must also derive standard facts that later rules can match:

```text
agent.turn.completed
agent.turn.failed
agent.turn.timed_out
agent.turn.cancelled
```

Harness failures are part of the event stream. A worker must not lose failures
by returning only process stderr or CLI diagnostics.

Failure events must cover at least these phases:

```text
provider.config.missing
provider.auth.failed
workspace.prepare.failed
adapter.resolve.failed
provider.launch.failed
provider.stdin.failed
provider.stream.failed
provider.timeout
provider.cancelled
provider.result.invalid
artifact.capture.failed
```

If a durable effect has been claimed, any adapter, launch, stream, provider, or
artifact-capture failure must append a terminal effect event whenever the store
is reachable:

```text
effect.failed
effect.timed_out
effect.cancelled
```

The terminal event payload must include:

```text
phase
provider
adapter
error_kind
message
recoverable
retry_after?
workspace_id?
provider_session_id?
provider_thread_id?
artifact_refs
stderr_ref?
transcript_ref?
missing_config_keys[]
```

Secrets must never be written to failure payloads. Missing credentials should be
reported by credential reference or key name, not value.

The command-backed harness now implements the deterministic boundary taxonomy
for the test/compatibility surface: missing required environment keys,
unresolvable adapter commands, missing configured workspaces, launch/stdin/wait
failures, provider timeout, nonzero exit, and invalid structured stdout are
classified before they become `ProviderRunResult` values. This does not make
Codex, Claude, or Pi real adapters complete; it gives those adapters a stable
failure vocabulary to target.

If the failure happens before a provider run can be started, the effect should
be marked blocked rather than silently skipped. Missing provider config,
credentials, native enforcement, or capacity are `blocked_by_policy` or
`effect.blocked` states with a structured reason such as
`provider_config_missing` until corrected. Workspace and adapter failures after
claim are provider runtime failures and should produce `agent.turn.failed` with
evidence.

For convenience patterns, the runtime may also derive profile-specific aliases
such as `worker completed turn` or relationship facts such as `worker completed
turn for loft issue` when the originating effect carries enough correlation
metadata. These aliases must be deterministic projections over recorded effect
input, run output, and related facts; they must not depend on prompt text.

## Provider Configuration

Provider bindings are runtime configuration, not workflow source. A binding
should specify at least:

```text
provider_id
provider_kind
profile_ids[]
credentials_ref
workspace_policy
default_model_or_runtime
timeout
max_parallel_runs
artifact_policy
retention_policy
approval_policy
native_enforcement_level
```

Provider-specific examples:

```text
codex:
  app_server_command | app_server_url | sdk_host
  auth/account profile
  model/reasoning defaults
  approval/sandbox policy

claude:
  sdk language/host
  ANTHROPIC_API_KEY or cloud-provider credential ref
  allowed_tools mapped from WhippleScript profile
  hooks and cwd policy

pi:
  extension id/path
  extension host configuration
  thread store/export path
  completion observation policy
```

The status view for a blocked effect must distinguish missing provider config,
missing credentials, insufficient native enforcement, capacity exhaustion, and
provider runtime failure.

## Profiles

Profiles are semantic authority bundles:

```text
repo-reader
repo-writer
internet-research
review-only
```

Provider names are not profile names. The capability registry binds profiles to
providers and enforcement options.

## Skills And Context

Before a turn starts, the harness assembles:

```text
base prompt
rule-provided message
attached skills
plugin-provided context bundles
Loft/Thoth/memory artifacts, if requested
capability instructions
```

Every injected context bundle must record provenance in the evidence store.

## Workspaces

Coding agents often need isolated workspaces. The first harness may run in the
current repository, but the target design should support:

```text
shared cwd
per-effect worktree
per-issue worktree
remote sandbox
container/vm
```

Workspace creation is a capability of the harness/policy layer, not a rule
language primitive.

Workspace records should include:

```text
workspace_id
policy
path_or_remote_ref
base_revision
dirty_state_before
dirty_state_after
files_changed[]
cleanup_policy
```

For repo-writing providers, the harness must record enough workspace metadata to
let an operator answer whether a turn wrote to the shared checkout, an isolated
worktree, or a remote sandbox.
