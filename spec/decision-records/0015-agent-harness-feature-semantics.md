# 0015: Agent Harness Feature Semantics

Status: proposed, research-validated baseline

## Decision

WhippleScript should standardize the agent-provider boundary, not the full
native semantics of each harness.

The package shape is:

```text
core
  agent declarations, AgentRef, tell, capacity/readiness, agent.tell effects,
  provider-run lifecycle, cancellation, terminal status, evidence, replay

std.agent
  shared feature taxonomy, profile preset vocabulary, skill/context contract,
  provider feature-report schema, health/status vocabulary

std.agent.codex
  Codex-specific App Server/SDK/CLI feature map, slash commands, plugins,
  hooks, subagents, skills, goals, permissions, Codex evidence shape

std.agent.claude
  Claude Agent SDK feature map, SDK-dispatchable slash commands, skills,
  hooks, plugins, subagents, sessions, permission/tool policy, evidence shape

std.agent.fixture
  deterministic test harnesses that exercise the same durable lifecycle
```

Provider packages are allowed to expose native features, but portable workflow
code may depend only on declared feature classes and accepted capability
reports. Provider-specific workflow code may opt into native features explicitly.

The word `environment` should stay reserved for a future package manager or
deployment environment concept.

## What Is Standardized

Amendment 2026-07-15 (spec/std-agent.md, slice 5 shipped): v1 ships the
MINIMAL report — schema `whipplescript.agent_feature_report.v0`, the
feature-class taxonomy below verbatim, each entry carrying
`class`/`support`/`source` plus `native_name` + `dispatch` when support is
stated. The full per-entry field list below (versions, headless flags, event
mappings, …) moves to the probed-report re-entry and is NOT required of v1
manifests. The taxonomy's single compiled source is
`whipplescript_core::AGENT_FEATURE_CLASS_TAXONOMY`.

`std.agent` should standardize these contract fields:

```text
provider kind
provider package id and version
native harness version
runtime surface: CLI | SDK | app-server | RPC | broker | fixture
headless support
interactive-only features
feature class inventory
native feature names and dispatch mechanisms
authority/profile mapping
capability report schema
health check schema
evidence/provenance schema
failure/blocking taxonomy
```

The feature taxonomy should be small and descriptive:

```text
context.compact
context.auto_compact
session.resume
session.fork
session.clone
session.export
turn.cancel
turn.steer
turn.follow_up
subagent.spawn
subagent.observe
subagent.steer
skill.attach
plugin.load
hook.lifecycle
native.command.dispatch
permission.policy
model.select
reasoning.select
goal.track
command.list
feature.report
```

Each feature class entry should record:

```text
native name
dispatch mechanism: slash_prompt | SDK option | CLI flag | RPC command |
  config file | extension API | plugin manifest | not dispatchable
introduced version
removed version, if known
requires interactive UI
available headless
mutates provider session state
mutates model context
can affect filesystem or tools
emits observable native events
canonical WhippleScript events/facts produced
evidence captured
failure modes
security/authority requirements
```

This makes feature support inspectable without pretending `/goal` and Claude
hooks have identical behavior.

## Provider-Specific Semantics

Provider packages own native details:

```text
exact slash command names
which slash commands are dispatchable headlessly
native hook events and hook return schemas
plugin/skill/extension discovery paths
subagent definition formats and delegation rules
session persistence and fork/resume semantics
compaction triggers and result payloads
model/reasoning controls
provider-specific approval/permission models
variant or extension package installation
native evidence payload shape before redaction
```

Portable code can require a feature:

```whip
agent reviewer {
  provider claude
  profile repo-reader
  requires [context.compact, subagent.spawn]
}
```

## Validation Findings

The split was checked against current primary documentation and local harness
probes on June 14, 2026.

Codex:

```text
local version: codex-cli 0.137.0
observed CLI surfaces: features, plugin management, app-server tooling,
  resume/archive/fork, exec/review, MCP, doctor
observed feature flags: goals, hooks, plugins, multi_agent, apps, browser_use,
  fast_mode, memories, personality, shell_tool, tool_suggest
docs: slash commands include /compact, /goal, /agent, /plugins, /hooks,
  /skills, /permissions, /model, /plan, /fork, /resume
docs: plugins can bundle skills, apps/MCP config, and lifecycle hooks
docs: subagents are explicit parallel-agent workflows with custom agent files
```

This supports `std.agent.codex` as a provider package with a rich native command
and plugin surface. It does not support treating every Codex command as a
portable workflow primitive.

Claude:

```text
local version: Claude Code 2.1.172
observed CLI surfaces: --agents, --agent, --plugin-dir, --plugin-url,
  --include-hook-events, --allowed-tools, --permission-mode, MCP management,
  plugin management, safe/bare modes
docs: Agent SDK exposes built-in tools, hooks, subagents, MCP, permissions,
  sessions, structured output, and usage tracking
docs: SDK slash commands are discoverable from the system init message and only
  non-interactive-safe commands are dispatchable
docs: plugins can include skills, agents, hooks, MCP servers, and legacy commands
```

This supports `std.agent.claude` as an SDK-oriented provider package. Claude has
overlap with Codex at the product level, but the programmatic boundary is
different enough that shared semantics must be feature-class based.

## Sources

- Codex slash commands: <https://developers.openai.com/codex/cli/slash-commands>
- Codex goals: <https://developers.openai.com/codex/use-cases/follow-goals>
- Codex hooks: <https://developers.openai.com/codex/hooks>
- Codex plugins: <https://developers.openai.com/codex/plugins>
- Codex plugin authoring: <https://developers.openai.com/codex/plugins/build>
- Codex subagents: <https://developers.openai.com/codex/subagents>
- Codex App Server: <https://developers.openai.com/codex/app-server>
- Claude Agent SDK overview: <https://code.claude.com/docs/en/agent-sdk/overview>
- Claude SDK slash commands: <https://code.claude.com/docs/en/agent-sdk/slash-commands>
- Claude hooks: <https://code.claude.com/docs/en/hooks>
- Claude subagents: <https://code.claude.com/docs/en/sub-agents>
- Claude SDK plugins: <https://code.claude.com/docs/en/agent-sdk/plugins>

## Design Consequences

- `std.agent` remains small and shared.
- Codex and Claude provider packages are separate package contracts.
- Feature maps are versioned artifacts and should be emitted in provider health
  or package-check reports.
- Native slash commands are not imported into WhippleScript as portable syntax.
- `native.command.dispatch` is a provider-specific escape hatch unless a command
  maps to a declared feature class with a checked contract.
- Compatibility checks must include harness version, provider package version,
  installed variant/package set, and headless/interactive availability.

## Next Validation Work

Before implementation, add explicit provider feature-report fixtures:

```text
std.agent.codex fixture:
  goals/hooks/plugins/multi_agent enabled and disabled cases
  app-server schema/version probe
  slash command inventory, if exposed programmatically

std.agent.claude fixture:
  system init slash_commands probe
  plugin-dir load probe
  hook event stream probe
  agents/subagents probe
```

These probes should not assert that native behavior is identical. They should
assert that every provider can produce a truthful feature report and that source
validation rejects workflows whose required feature classes are unavailable.
