# Agent Harness Layer

Status: draft

The harness layer turns durable `agent.tell` effects into real agent turns.

WhippleScript must not pretend an agent turn exists until this layer can start a
provider run, capture evidence, and append a completion event.

## Terminology: Three Senses Of "harness"

The word "harness" is overloaded. This spec uses exactly one meaning and names
the others explicitly:

```text
harness layer        THIS document — the runtime execution layer that runs
                     agent.tell effects as provider turns. The bare word
                     "harness" in this file always means this layer.

harness keyword      a soft-deprecated source keyword for advanced named
                     endpoints. Authoring should route agents through the
                     ordinary `provider` clause; see 0009. Always written as
                     "the `harness` keyword" when referenced here.

native harness       the provider's own agent harness (Codex's harness, Claude
                     Code, Pi). Always written as "native harness" / "provider
                     harness" when referenced here.
```

## Harness Player

A harness player is the runtime worker that executes queued harness effects.
It is intentionally boring:

```text
poll/subscribe for claimable agent.tell effects
start one effect run under policy
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
run(LaunchPlan, AbortSignal) -> ProviderRunResult
collect(ProviderRunResult) -> CompletionEvent + artifacts
```

The `LaunchPlan` carries the **turn-access grant** as authority-narrowing
metadata, not as a separate effect or lowering class. When the source `tell`
appears under `with access to <resource> { … }`, that grant is metadata on the
`agent.tell` effect (Proposal A in
[`admission-and-idempotency.md`](admission-and-idempotency.md)). The harness
resolves the grant into the LaunchPlan as the **effective intersection** of
store policy, the source-declared grant, the provider's reported capabilities,
and the agent's profile. The adapter then runs the turn under that narrowed
authority. In-turn tool invocations made under the grant are recorded as
evidence (see below); they never become rule-matchable child facts.

The normalized provider result is a **terminal summary plus evidence/artifact
references**, not a typed transcript of provider-specific stream concepts:

```text
provider
provider_session_id
thread_id
turn_id
status                 # terminal: completed | failed | timed_out | cancelled
summary
structured_result_json?
changed_files[]
diff_refs[]
artifact_refs[]
evidence_refs[]        # streamed progress, tool calls, approvals: EVIDENCE only
usage_json?
started_at
completed_at
```

Provider tool calls and approval events are **provider-incompatible stream
concepts** (Codex approvals, Claude tool/hook events, Pi tool events do not
share a shape), so they are not typed result fields. They live in evidence,
reachable through `evidence_refs`/`artifact_refs`. Each adapter may obtain the
summary fields differently, but the harness normalizes them before appending
completion events. Missing optional fields should be explicitly represented as
unavailable rather than silently dropped.

### Cancellation

`run` takes an abort/cancel signal because the providers expose materially
different cancellation surfaces: Codex `turn/interrupt`, Claude request-only
cancel (semantics still unvalidated), and Pi RPC `abort` whose acknowledgement
can arrive out of order with the terminal event. The adapter contract therefore
requires a cancel entry point and a rule for how it resolves:

```text
cancel acknowledged + terminal observed  -> status = cancelled
cancel requested, terminal arrives first  -> use the observed terminal
                                             (Pi: terminal may precede the
                                             abort ack; tolerate reordering)
cancel requested, no terminal observable, provider supports idempotent re-query
                                          -> re-query; admit discovered terminal
cancel requested, no terminal, no idempotent re-query
                                          -> resolve to `uncertain` (a Failed
                                             subkind) per the exactly-once rules
                                             in admission-and-idempotency.md
```

This follows the exactly-once / `uncertain` terminal rules in the keystone: a
worker never silently re-executes an external side effect when the terminal is
unknown.

### Replay

An agent turn is **record-once** (Determinism And Replay in
[`admission-and-idempotency.md`](admission-and-idempotency.md)): the turn records
its terminal outcome plus evidence durably, and replay reads the recorded
terminal and evidence and **never re-invokes the provider**. The turn's
idempotency key (or companion execution fingerprint) commits to the
provider/model id, prompt/coercion-artifact hash, and output-schema hash, so a
changed contract is a different key and a stale recorded outcome is never
reused.

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
with a deterministic `MockAgentHarness`. The kernel-owned runner starts an
effect run, records injected skill provenance, runs the adapter, stores
artifacts and provider evidence (including in-turn stream/tool/approval
evidence), appends the terminal effect completion, and then emits the terminal
`agent.turn.*` lifecycle event plus its rule-matchable fact. Adapters return
data; they do not receive store handles or mutate kernel state directly.

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

The first Claude adapter boundary is a TypeScript sidecar around
`@anthropic-ai/claude-agent-sdk`; see `spec/claude-agent-sdk-strategy.md`.
Python remains a fallback/probe surface because it has a useful stateful
`ClaudeSDKClient`, but it adds a second runtime packaging path.

The Claude adapter therefore needs:

- TypeScript Agent SDK host process with a small JSONL protocol.
- API-key/provider authentication configuration.
- Tool permission mapping from WhippleScript profiles to Claude SDK allowed
  tools, hooks, and working directories.
- Streaming message handling and final result extraction.
- Artifact capture for transcript, tool calls, edits, command output, and usage.

### Pi Provider

Pi should integrate first through `pi --mode rpc`. The extension and SDK surfaces
remain important for custom resources and tools, but the validated native
adapter boundary is the RPC subprocess protocol, not ordinary print mode or a
WhippleScript-specific extension.

The Pi adapter therefore needs:

- A Pi RPC subprocess client that sends prompt, state, and abort commands.
- A way to correlate WhippleScript `effect_id` / `run_id` with Pi session ids.
- Event observation for `agent_start`, `turn_start`, message events, `turn_end`,
  and `agent_end`.
- Completion and cancellation detection from terminal event metadata, including
  assistant `stopReason: "aborted"` for RPC aborts.
- Artifact capture for thread snapshots, extension events, user-visible
  messages, and final outcome metadata.

Pi stores conversation sessions, so those session ids and snapshots should be
first-class evidence. A thread/session export bridge is useful for audit and
recovery, but it should complement the RPC adapter rather than replace it.

Provider adapters are replaceable. The rule language addresses logical agents
and profiles; the registry chooses whether that means Codex, Claude Code, Pi,
an enterprise broker, or a test fixture.

## Control Plane Bridge

The harness cannot run until the control plane has materialized ready rule
commits into durable outbox effects. A complete local validation loop needs both:

```text
source + instance event -> ready rule evaluation -> rule commit -> effect outbox
effect outbox -> harness run start -> provider run -> completion event
completion event -> derived facts -> next ready rules
```

`whip run` may remain a "start an instance" command, but local validation needs an
explicit driver such as `whip step`, `whip worker`, or `whip dev` that advances
ready rules and runs configured local providers until idle, stopped, or blocked.

## Agent Turn Lifecycle

```text
effect queued
effect run started
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

Completion derives the durable, **rule-matchable** lifecycle facts:

```text
agent.turn.started
agent.turn.completed
agent.turn.failed
agent.turn.timed_out
agent.turn.cancelled
```

These are the only `agent.turn.*` values rules may match. They are the durable
lifecycle of a turn under the canonical terminal union
(`Completed<O> | Failed<E> | TimedOut | Cancelled`; see
[`admission-and-idempotency.md`](admission-and-idempotency.md) and
[`expression-kernel.md`](expression-kernel.md)).

In-turn observations are recorded as **evidence**, not rule-matchable facts.
This is Proposal A in [`admission-and-idempotency.md`](admission-and-idempotency.md):
turn-internal activity is inspectable evidence, never event-sourced facts that
later rules pattern-match. The following are evidence kinds, not facts:

```text
agent.turn.streamed          (evidence: streamed model progress)
agent.turn.tool_requested    (evidence: in-turn tool call)
agent.turn.artifact_captured (evidence: captured artifact/diff)
```

Rules cannot react to a per-tool or per-stream event; they observe only the
terminal lifecycle facts above plus the recorded evidence on the turn. We
explicitly rejected an event-sourced in-turn harness for v0.

Native adapters normalize provider-specific event names into these canonical
lifecycle facts and evidence kinds. The canonical payload includes `effect_id`, `run_id`, `agent`,
`provider`, `status`, `terminal`, provider session/turn ids when available, the
provider event type, and only a redacted provider payload shape. Raw provider
transcript, tool arguments, diffs, and error text belong in bounded evidence or
artifact refs, not the lifecycle event payload.

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

Once a provider run has started, any adapter, launch, stream, provider, or
artifact-capture failure must append a canonical terminal effect event whenever
the store is reachable:

```text
effect.terminal
```

The terminal event payload must include:

```text
status                 # failed | timed_out | cancelled
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
credentials, or native enforcement are `blocked_by_capability` or
`blocked_by_profile`; unavailable declared capacity is `blocked_by_capacity`.
They also append `effect.blocked` with a structured reason such as
`provider_config_missing` until corrected. Workspace and adapter failures after
a provider run starts are provider runtime failures and should produce
`agent.turn.failed` with evidence.

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
  RPC subprocess configuration
  provider/model defaults
  --tools allowlist mapped from WhippleScript profile
  extension/skill/resource refs
  session store/export path
  completion and abort observation policy
```

The status view for a blocked effect must distinguish missing provider config,
missing credentials, insufficient native enforcement, capacity exhaustion, and
provider runtime failure.

## Profiles

Profiles are semantic authority bundles. The canonical preset list is pinned
once by `std.agent` in
[`decision-records/0009-agent-package.md`](decision-records/0009-agent-package.md);
this layer references that list rather than restating its own:

```text
repo-reader
repo-writer
internet-research
issue-triager
human-review
release-operator
no-repo
```

Provider names are not profile names. The capability registry binds profiles to
providers and enforcement options.

Turn-scoped authority narrowing on top of the profile is expressed in source as
`with access to <resource> { … }`. Per Proposal A it is authority-narrowing
metadata on the `agent.tell` effect, not a profile and not a durable child
effect; the harness folds it into the LaunchPlan's effective intersection (see
Provider Adapter Contract).

## Skills And Context

Before a turn starts, the harness assembles:

```text
base prompt
rule-provided message
attached skills
package/provider context bundles
tracker/repo/memory artifacts, if requested
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
