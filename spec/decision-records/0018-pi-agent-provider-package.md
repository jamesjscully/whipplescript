# 0018: Pi Agent Provider Package

Status: proposed

## Decision

`std.agent.pi` is the first-party provider package that maps the shared
`std.agent` contract onto Pi.

The package depends on `std.agent` and contributes:

```text
provider kind: pi
primary native surface: pi_rpc
source provider name: pi
provider package feature report
Pi variant declarations and validation
Pi RPC compatibility checks
Pi extension/package command and tool discovery
Pi evidence summary schema
```

Core still owns `agent`, `AgentRef`, `tell`, `agent.tell`, capacity/readiness,
provider-run lifecycle, terminal status, cancellation state, evidence storage,
and replay. `std.agent.pi` does not introduce a second workflow engine.

## Native Surface

The primary integration surface is `pi --mode rpc` over stdin/stdout JSONL.

The Pi SDK remains a useful reference and possible fallback if RPC lacks a
validated control, but the first package contract should be RPC-first.
Ordinary text/print mode is not the native durable adapter boundary.

Minimum RPC expectations:

```text
get_state
prompt
follow_up
abort
get_commands
session selection/resume
fork or clone session, when available
model/thinking selection or observation
compaction state and command support
event stream with agent/turn/message terminal observations
tool and extension event summaries
```

The package should validate required RPC commands and event shapes by probing
the installed Pi binary plus selected variant. It should not rely on exact Pi
binary version except in strict review mode.

## Pi Variants

Pi behavior depends materially on installed packages, extension files, skills,
prompt templates, enabled resources, and CLI flags. `std.agent.pi` should model
these as `pi_variant`, not `environment`.

`environment` should stay reserved for a future package manager or deployment
environment concept.

Example:

```whip
use std.agent
use std.agent.pi

pi_variant research_team {
  pi ">=0.73 <0.80"
  packages [
    "npm:@yzlin/pi-subagents@0.6.0"
  ]
  extensions [
    "./pi/extensions/project-policy.ts"
  ]
  skills [
    "./pi/skills/research"
  ]
  tools [read, grep, find, ls]
}

agent scout {
  provider pi
  variant research_team
  profile repo-reader
}
```

The accepted provider report for a variant must include:

```text
resolved Pi binary version
package/install sources and versions
extension paths/package ids and content hashes
skill/template/theme/resource refs, where relevant
enabled tool list
command list from get_commands
model/provider/thinking posture
session storage policy
feature map
provenance hash
```

Source validation should fail or warn when a workflow requires a feature that
the selected variant cannot report, using the one severity enum
`error | warning | info | hint` (unreportable required feature -> `error`).

## Source Surface

Ordinary source can stay small when no variant-specific behavior is needed:

```whip
agent analyst {
  provider pi
  profile repo-reader
}
```

Use `variant` only when the workflow depends on a concrete Pi package/extension
set or a non-default tool/command surface.

Provider package imports make `pi` and `pi_variant` known source constructs.
Importing the package does not grant runtime authority. Authority still comes
from runtime provider bindings, credentials, profile allowlists, workspace
policy, tool policy, variant acceptance, and effect capabilities.

## Feature Map

`std.agent.pi` should publish a versioned feature report using the shared
taxonomy from
[`0015-agent-harness-feature-semantics.md`](0015-agent-harness-feature-semantics.md).

Initial feature classes to report:

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
command.list
feature.report
```

Pi extension commands are native features, not WhippleScript syntax. A command
or tool should be usable from workflow validation only when the selected
variant's accepted feature report includes it.

Each reported feature must say whether it is:

```text
provided by Pi core
provided by a package
provided by an extension file
provided by a skill/template/resource
available headless through RPC
interactive-only
observable but not dispatchable
```

This is stricter than Codex/Claude because Pi extensions can register commands,
tools, shortcuts, flags, lifecycle hooks, prompt mutation, and subagent-like
capabilities.

## Tool, Profile, And Permission Mapping

Pi uses lower-case tool names and extension/custom tools. `std.agent.pi` should
translate the canonical `std.agent` profile presets (pinned once in
[`0009-agent-package.md`](0009-agent-package.md); not restated here) into
explicit Pi tool selection:

```text
repo-reader -> read, grep, find, ls
repo-writer -> read, grep, find, ls, edit, write, bash with explicit policy
human-review -> read/search/list plus review context
no-repo -> no filesystem tools
internet-research -> provider/model/tool posture only when configured
```

Turn-scoped `with access to <resource> { … }` grants narrow this further: per
Proposal A they are authority-narrowing metadata on the `agent.tell` effect,
folded into the LaunchPlan's effective intersection
(store/grant/provider/profile) before Pi tool selection. In-turn tool/extension
activity under the grant is recorded as evidence, not rule-matchable facts.

The package should reject destructive tools in read-only workspaces and should
make extension-provided tools visible in the feature report before any workflow
depends on them.

## Sessions, Steering, Follow-Up, And Cancellation

Pi exposes session and control concepts directly through CLI/RPC:

```text
session selection/resume
fork/clone
follow_up
steering mode
auto-compaction state
abort
export
```

`std.agent.pi` should model these as feature classes. The adapter's cancel entry
point (the abort/cancel signal on the shared adapter contract; see
[`../agent-harness.md`](../agent-harness.md)) maps to RPC `abort`. The runtime
must preserve Pi's event ordering: a terminal event may arrive before the
`abort` acknowledgement, so an out-of-order ack does not change an
already-observed terminal. Cancellation should be considered provider-native
only when the adapter observes a terminal aborted stop reason or equivalent
validated signal. If a cancel is requested and no terminal is observable, the
turn resolves to the keystone's `uncertain` terminal (a Failed subkind) per the
exactly-once rules in
[`../admission-and-idempotency.md`](../admission-and-idempotency.md); the side
effect is never silently re-executed.

## Evidence

Pi provider evidence should include redacted summaries of:

```text
Pi binary version
package/extension/variant lock hash
RPC protocol posture
session id and parent/fork id
model/provider/thinking state
steering/follow-up/compaction state
command list hash and summary
tool list hash and summary
extension event summaries
tool event summaries
terminal stop reason
abort acknowledgement ordering, when relevant
feature report hash
```

Raw prompts, transcripts, tool inputs, file contents, command output,
credentials, provider payloads, and extension internals should be stored only as
bounded artifacts/evidence refs after redaction.

## Non-Goals

`std.agent.pi` should not:

```text
redefine agent.tell
call Pi variants environments
assume package/extension-installed commands exist without probing
turn every Pi extension command into portable WhippleScript syntax
hide extension-provided prompt mutation or tools
parse ordinary text mode as the native adapter
grant provider authority by import alone
```

## Validation Fixtures

Before implementation, add fixtures/probes for:

```text
bare variant get_state/get_commands probe
variant with extensions disabled
variant with one extension registering a command
variant with one extension registering a tool/subagent-like capability
prompt/follow_up/abort event ordering
session resume/fork/clone/export probes
compaction state and command probe
tool profile mapping for repo-reader/repo-writer/no-repo
variant lock/provenance hash stability
```

Live provider tests should be optional and explicitly gated. They should run in
disposable workspaces and record only redacted metadata.

## Open Questions

- What is the exact lockfile format for a `pi_variant` package/extension set?
- Should `pi_variant` live only in source, or should accepted variants be
  materialized into provider-config/package-lock artifacts?
- Which extension-provided subagent patterns are common enough to map to
  `subagent.spawn` rather than `native.command.dispatch`?
- How should local/project Pi package installation interact with WhippleScript's
  future package manager?

## Sources

- Pi RPC: <https://pi.dev/docs/latest/rpc>
- Pi extensions: <https://pi.dev/docs/latest/extensions>
- Existing strategy: [`pi-rpc-strategy.md`](../pi-rpc-strategy.md)
- Shared feature semantics:
  [`0015-agent-harness-feature-semantics.md`](0015-agent-harness-feature-semantics.md)
- Existing surface notes: [`native-provider-surfaces.md`](../native-provider-surfaces.md)

