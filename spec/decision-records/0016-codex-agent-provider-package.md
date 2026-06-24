# 0016: Codex Agent Provider Package

Status: proposed

## Decision

`std.agent.codex` is the first-party provider package that maps the shared
`std.agent` contract onto Codex.

The package depends on `std.agent` and contributes:

```text
provider kind: codex
primary native surface: codex_app_server
source provider name: codex
provider package feature report
Codex profile/policy mapping
Codex skill/plugin/hook/subagent feature metadata
Codex App Server compatibility checks
Codex evidence summary schema
```

Core still owns `agent`, `AgentRef`, `tell`, `agent.tell`, capacity/readiness,
provider-run lifecycle, terminal status, cancellation state, evidence storage,
and replay. `std.agent.codex` does not introduce a second agent-turn semantics.

## Native Surface

The primary integration surface is the Codex App Server or a future supported
Codex SDK surface with equivalent thread/turn/event control.

The package should not implement native turns by parsing ordinary `codex exec`
text output. `codex exec` may remain useful for diagnostics or one-shot
compatibility probes, but it is not the package contract for durable agent
turns.

Minimum App Server expectations:

```text
initialize or equivalent capability handshake
thread/session creation or resume
turn start
turn completion notification
turn interrupt/cancellation request, when supported
stream/event notifications
approval/tool request notification and response path
diff/file-change/artifact notification path
schema or protocol version discovery
```

The provider package should validate the installed Codex surface by required
methods and schema features, not by exact CLI version except in strict review
mode. Different developers will have different Codex versions installed.

Cancellation maps to the adapter's cancel entry point (the abort/cancel signal
on the shared adapter contract; see [`../agent-harness.md`](../agent-harness.md))
which drives Codex `turn/interrupt` where supported. A cancel that is
acknowledged with an observed terminal resolves to `cancelled`; a cancel with no
observable terminal and no idempotent re-query resolves to the keystone's
`uncertain` terminal (a Failed subkind) per the exactly-once rules in
[`../admission-and-idempotency.md`](../admission-and-idempotency.md). The package
must not fabricate a terminal status for an unacknowledged interrupt.

## Source Surface

Ordinary source should stay small:

```whip
use std.agent
use std.agent.codex

agent implementer {
  provider codex
  profile repo-writer
  skills ["whipplescript-author"]
}
```

Advanced endpoint routing may still use named harnesses or provider bindings,
but `provider codex` is the normal authoring vocabulary.

Provider package imports make `codex` a known provider kind and expose Codex's
feature report to source validation. Importing the package does not grant
runtime authority. Authority still comes from runtime provider bindings,
profile allowlists, credentials/session posture, workspace policy, and effect
capabilities.

## Feature Map

`std.agent.codex` should publish a versioned feature report using the shared
taxonomy from
[`0015-agent-harness-feature-semantics.md`](0015-agent-harness-feature-semantics.md).

Initial feature classes to report:

```text
context.compact
context.auto_compact
goal.track
session.resume
session.fork
turn.cancel
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

Codex slash commands are native features, not WhippleScript syntax. `/compact`,
`/goal`, `/agent`, `/plugins`, `/hooks`, `/skills`, `/permissions`, `/model`,
`/plan`, `/fork`, and `/resume` should appear in reports only when the selected
Codex surface can observe or dispatch them truthfully.

Each reported feature must say whether it is:

```text
headless-safe
interactive-only
dispatchable through App Server or SDK
available only as CLI/TUI slash command
configuration-only
observable but not dispatchable
```

This distinction matters. For example, a Codex feature may exist in the
interactive CLI but not be controllable through the App Server surface used by
WhippleScript.

## Profile And Authority Mapping

The package maps the canonical `std.agent` profile presets (pinned once in
[`0009-agent-package.md`](0009-agent-package.md); not restated here) to Codex
policy/config, keeping the mapping visible:

```text
repo-reader       -> read-only workspace/tool posture
repo-writer       -> workspace-write posture with explicit approval policy
human-review      -> review-oriented prompt/context plus read-oriented tools
internet-research -> web/search-enabled posture when configured
release-operator  -> explicit high-authority posture, never default
```

Turn-scoped `with access to <resource> { … }` grants narrow this further: per
Proposal A they are authority-narrowing metadata on the `agent.tell` effect, and
the harness folds them into the LaunchPlan's effective intersection
(store/grant/provider/profile) before mapping to Codex policy. In-turn tool
invocations under the grant are recorded as evidence, not rule-matchable facts.

The feature report should expose:

```text
supported profiles
unsupported profiles
approval/sandbox modes available
native enforcement level
required credentials/session posture
workspace policy support
artifact/diff capture support
subagent limits
```

`std.agent.codex` should not silently add broad filesystem, network, or approval
authority because a source agent says `provider codex`.

## Skills, Plugins, Hooks, And Subagents

Codex has native skills, plugins, hooks, and subagents. The package should treat
them as native provider features with provenance, not hidden behavior.

Required evidence/provenance:

```text
loaded Codex skill ids and versions/paths
loaded plugin ids and versions/paths
hook sources and trust posture
subagent type/name when a subagent is spawned
feature flags relevant to the run
redacted config posture
```

Subagents are useful for parallel work, but WhippleScript should not infer
subagent use from prompt text. A workflow that requires subagent capability
should require `subagent.spawn` in the provider feature report.

## Evidence

Codex provider evidence should include redacted summaries of:

```text
Codex CLI/app-server version
App Server schema/protocol posture
thread id
turn id
item ids, when exposed
model/reasoning/fast-mode posture
approval/tool request summaries
diff/file-change summaries
artifact refs
usage/token summary, when available
terminal status and native stop reason
feature report hash
```

Raw prompts, full transcripts, tool arguments, diffs, files, credentials, and
provider payloads should be stored only as bounded artifacts/evidence refs after
redaction.

## Non-Goals

`std.agent.codex` should not:

```text
redefine agent.tell
make every Codex slash command portable WhippleScript syntax
depend on the private desktop app server being externally reachable
parse interactive TUI output as the native adapter
hide profile expansion or skill/plugin injection
grant provider authority by import alone
```

## Validation Fixtures

Before implementation, add fixtures/probes for:

```text
app-server schema generation and required method detection
feature flag report for goals/hooks/plugins/multi_agent enabled and disabled
basic thread/turn success through a deterministic fake App Server
turn interrupt acknowledgement without fabricated terminal status
approval/tool request normalization
diff/file-change artifact summary normalization
skill/plugin/hook provenance report
```

Live provider tests should be optional and explicitly gated. They should run in
disposable workspaces and record only redacted metadata.

## Open Questions

- Is there a supported programmatic inventory for Codex slash commands, or must
  the package maintain a versioned known-command table plus App Server probes?
- Which Codex goal semantics are controllable headlessly versus only in the
  interactive CLI?
- How much of Codex plugin/hook trust state is available to App Server clients?
- Should `std.agent.codex` expose custom Codex agents as a native subagent
  feature only, or should selected custom agents become source-visible profile
  presets?

## Sources

- Codex slash commands: <https://developers.openai.com/codex/cli/slash-commands>
- Codex goals: <https://developers.openai.com/codex/use-cases/follow-goals>
- Codex hooks: <https://developers.openai.com/codex/hooks>
- Codex plugins: <https://developers.openai.com/codex/plugins>
- Codex plugin authoring: <https://developers.openai.com/codex/plugins/build>
- Codex subagents: <https://developers.openai.com/codex/subagents>
- Codex App Server: <https://developers.openai.com/codex/app-server>
- Shared feature semantics:
  [`0015-agent-harness-feature-semantics.md`](0015-agent-harness-feature-semantics.md)
- Existing surface notes: [`native-provider-surfaces.md`](../native-provider-surfaces.md)
