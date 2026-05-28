# Agent Harness Layer

Status: draft

The harness layer turns durable `agent.tell` effects into real agent turns.

Armature must not pretend an agent turn exists until this layer can claim an
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
Docket readiness, retry policy, or decide when the loop is done. Those decisions
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

Provider adapters are replaceable. The rule language addresses logical agents
and profiles; the registry chooses whether that means Codex, Claude Code, Pi,
an enterprise broker, or a test fixture.

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
Docket/Thoth/memory artifacts, if requested
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
