# 0017: Claude Agent Provider Package

Status: proposed

## Decision

`std.agent.claude` is the first-party provider package that maps the shared
`std.agent` contract onto Claude Code through the Claude Agent SDK.

The package depends on `std.agent` and contributes:

```text
provider kind: claude
primary native surface: claude_agent_sdk
source provider name: claude
provider package feature report
Claude profile/tool/permission mapping
Claude SDK sidecar compatibility checks
Claude skill/plugin/hook/subagent feature metadata
Claude evidence summary schema
```

Core still owns `agent`, `AgentRef`, `tell`, `agent.tell`, capacity/readiness,
provider-run lifecycle, terminal status, cancellation state, evidence storage,
and replay. `std.agent.claude` does not introduce a separate workflow engine.

## Native Surface

The primary integration surface is the Claude Agent SDK. The existing adapter
strategy chooses a TypeScript sidecar around `@anthropic-ai/claude-agent-sdk`.

The package should not implement native Claude turns by parsing ordinary
interactive CLI output. `claude -p --output-format stream-json` may remain a
compatibility probe and diagnostic fallback, but the provider package contract
is SDK-oriented.

Minimum SDK expectations:

```text
start one turn with prompt, cwd/workspace policy, model, profile, tools, MCP,
  hooks, and timeout policy
stream SDK message events
observe tool requests/completions and hook events
surface usage/cost when available
preserve Claude session identity
support session resume/fork/continue when configured
support user input/approval interactions where the SDK exposes them
request cancellation/interruption only when live validation proves semantics
```

The package should validate required SDK/CLI features and locked sidecar
dependencies. It should not reject harmless Claude Code version drift by exact
version unless strict review mode asks for that.

## Source Surface

Ordinary source should stay small:

```whip
use std.agent
use std.agent.claude

agent reviewer {
  provider claude
  profile human-review
  skills ["code-reviewer"]
}
```

Provider package imports make `claude` a known provider kind and expose Claude's
feature report to source validation. Importing the package does not grant
runtime authority. Authority still comes from runtime provider bindings,
credential refs, profile allowlists, workspace policy, permission mode, tool
policy, MCP policy, and effect capabilities.

## Feature Map

`std.agent.claude` should publish a versioned feature report using the shared
taxonomy from
[`0015-agent-harness-feature-semantics.md`](0015-agent-harness-feature-semantics.md).

Initial feature classes to report:

```text
context.compact
context.auto_compact
session.resume
session.fork
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
command.list
feature.report
```

Claude SDK slash commands are native features, not WhippleScript syntax. The SDK
documentation says available commands are discoverable from the system init
message and only commands marked suitable for non-interactive use should be
dispatched programmatically. The package should preserve that distinction:

```text
discovered command
non-interactive safe
interactive-only
SDK-dispatchable
configuration-only
observable but not dispatchable
```

No workflow should depend on a Claude slash command unless the accepted feature
report says it is available and dispatchable through the selected SDK surface.

## Profile, Tool, And Permission Mapping

Claude has a rich tool and permission policy surface. `std.agent.claude` should
translate shared profiles into explicit SDK options:

```text
allowedTools
disallowedTools
permissionMode
MCP config refs
settings sources
workspace/cwd policy
hook policy
model and effort
```

Profiles are the canonical `std.agent` presets (pinned once in
[`0009-agent-package.md`](0009-agent-package.md); not restated here). Initial
mapping intent:

```text
repo-reader       -> read/search/list tools only, no edit/write/bash mutation
repo-writer       -> edit/write/bash posture only with explicit permission mode
human-review      -> read/search plus review-oriented prompt/context
internet-research -> web/MCP/search posture when configured
```

Turn-scoped `with access to <resource> { … }` grants narrow these mappings
further: per Proposal A they are authority-narrowing metadata on the
`agent.tell` effect, folded into the LaunchPlan's effective intersection
(store/grant/provider/profile) before being translated into SDK
`allowedTools`/`permissionMode`. In-turn tool/hook activity under the grant is
recorded as evidence, not rule-matchable facts.

The package should reject destructive capabilities in read-only workspaces and
reject destructive tools without an explicit approval/permission mode. The
profile mapper should be centralized in the package contract, not scattered as
string conventions in adapter code.

## Skills, Plugins, Hooks, MCP, And Subagents

Claude has native skills, plugins, hooks, MCP servers, and subagents. The
package should treat them as native provider features with provenance.

Required evidence/provenance:

```text
loaded Claude skill/plugin ids and versions/paths
loaded MCP config refs and health posture
hook event summaries and hook source posture
custom agent/subagent definitions used
tool policy actually sent to the SDK
permission mode actually sent to the SDK
redacted setting-source posture
```

Skills/plugins/hooks may affect prompt/context, tools, and lifecycle behavior.
They must not be invisible authority grants. If a workflow requires subagent
capability, source validation should require `subagent.spawn` in the accepted
feature report.

## Sessions And Cancellation

Claude session behavior belongs in the provider package feature map:

```text
session.resume
session.fork
session.continue, represented under session.resume/fork where appropriate
```

Cancellation should remain conservative. The existing strategy explicitly says
not to advertise Claude cancellation as provider-native until live behavior is
validated for the chosen TypeScript SDK path. Until then, the feature report
should mark `turn.cancel` as:

```text
unsupported
request_only
unknown
```

depending on the selected SDK/sidecar version and probe results. It should not
pretend a durable terminal cancellation exists just because WhippleScript can
record a cancellation request.

The adapter's cancel entry point (the abort/cancel signal on the shared adapter
contract; see [`../agent-harness.md`](../agent-harness.md)) carries this
request-only cancel. Because Claude cancellation is request-only/unvalidated, a
cancel without an observed terminal does not produce a `cancelled` terminal: it
resolves to the keystone's `uncertain` terminal (a Failed subkind) per the
exactly-once rules in
[`../admission-and-idempotency.md`](../admission-and-idempotency.md), unless an
idempotent re-query discovers the real terminal.

## Evidence

Claude provider evidence should include redacted summaries of:

```text
Claude Code version
@anthropic-ai/claude-agent-sdk package/version posture
sidecar protocol version
session id
model and effort
permission mode
tool policy summary
MCP config refs
hook event counts/summaries
tool event counts/summaries
subagent event counts/summaries
usage/cost shape, when available
terminal message subtype/result shape
feature report hash
```

Raw prompts, transcripts, tool inputs, file contents, command output, API keys,
cloud auth payloads, and provider payloads should be stored only as bounded
artifacts/evidence refs after redaction.

## Non-Goals

`std.agent.claude` should not:

```text
redefine agent.tell
use local interactive claude.ai login as the only supported product auth model
parse interactive CLI output as the native adapter
advertise cancellation before live validation
hide tool/permission/profile expansion
make every Claude slash command portable WhippleScript syntax
grant provider authority by import alone
```

## Validation Fixtures

Before implementation, add fixtures/probes for:

```text
TypeScript sidecar start and protocol handshake
locked @anthropic-ai/claude-agent-sdk dependency posture
system init slash_commands discovery
plugin-dir load probe
hook event stream probe
tool policy mapping for repo-reader/repo-writer/human-review
MCP config ref pass-through without leaking secrets
subagent/custom-agent probe
session resume/fork probe
cancellation live probe before turn.cancel is marked provider-native
```

Live provider tests should be optional and explicitly gated. They should run in
disposable workspaces and record only redacted metadata.

## Open Questions

- Which SDK slash commands are stable enough to expose under
  `native.command.dispatch`?
- Should Claude custom agents map only to native subagent metadata, or can some
  become source-visible profile presets?
- What is the exact evidence schema for hook events that influence permission or
  tool use?
- Should Python SDK support stay a diagnostic fallback only, or become a second
  accepted surface if TypeScript cancellation/session semantics lag?

## Sources

- Claude Agent SDK overview: <https://code.claude.com/docs/en/agent-sdk/overview>
- Claude SDK slash commands: <https://code.claude.com/docs/en/agent-sdk/slash-commands>
- Claude hooks: <https://code.claude.com/docs/en/hooks>
- Claude subagents: <https://code.claude.com/docs/en/sub-agents>
- Claude SDK plugins: <https://code.claude.com/docs/en/agent-sdk/plugins>
- Existing strategy:
  [`claude-agent-sdk-strategy.md`](../claude-agent-sdk-strategy.md)
- Shared feature semantics:
  [`0015-agent-harness-feature-semantics.md`](0015-agent-harness-feature-semantics.md)
- Existing surface notes: [`native-provider-surfaces.md`](../native-provider-surfaces.md)
