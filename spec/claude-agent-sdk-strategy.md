# Claude Agent SDK Strategy

Status: implementation decision for `NP-031`.

Date: June 1, 2026.

## Decision

Use a TypeScript sidecar around `@anthropic-ai/claude-agent-sdk` as the first
native Claude adapter boundary.

The Rust kernel should not shell directly to `claude -p` for the native adapter.
`claude -p --output-format stream-json` remains a compatibility probe and
diagnostic fallback only.

## Validated Inputs

- Official Agent SDK overview:
  `https://code.claude.com/docs/en/agent-sdk/overview`.
- Official TypeScript Agent SDK reference:
  `https://code.claude.com/docs/en/agent-sdk/typescript.md`.
- Official Python Agent SDK reference:
  `https://code.claude.com/docs/en/agent-sdk/python`.
- Official sessions guide:
  `https://code.claude.com/docs/en/agent-sdk/sessions`.
- Official approvals/user input guide:
  `https://code.claude.com/docs/en/agent-sdk/user-input`.
- Local CLI probe: `claude --version` reports `2.1.116 (Claude Code)`.
- Package probe: `npm view @anthropic-ai/claude-agent-sdk ...` reports latest
  `0.3.159`; `pip index versions claude-agent-sdk` reports latest `0.2.87`.

## TypeScript Sidecar Shape

The sidecar should be a long-lived Node process with a small JSONL protocol:

- `run/start`: start one Claude turn with workspace, model, profile, prompt,
  tool policy, MCP config refs, and timeout policy.
- `run/cancel`: request provider-native cancellation for a running turn.
- `run/input`: answer a pending approval or `AskUserQuestion` request.
- `run/close`: stop the sidecar after all active runs are terminal.

The sidecar emits redacted JSONL events:

- `claude.session.started` with session id and SDK/package posture.
- `claude.stream.message` for SDK message type/subtype and bounded metadata.
- `claude.tool.requested` for tool name, input shape, session/run ids, and
  approval context.
- `claude.tool.completed` for tool name, status, output shape, and artifact refs.
- `claude.usage.updated` for usage/cost fields when SDK messages expose them.
- `claude.turn.completed` for terminal subtype, result shape, and provider
  session id.
- `claude.turn.failed` for structured SDK/CLI errors.
- `claude.turn.cancelled` when cancellation is acknowledged or terminally
  observed.

No raw prompt, transcript, command body, file content, API key, or provider auth
payload should cross into the store as inline evidence. Store refs should carry
sizes, hashes, schemas, event counts, and paths to retained artifacts after
redaction.

## Why TypeScript First

| Concern | TypeScript sidecar | Python sidecar |
| --- | --- | --- |
| Packaging | The TypeScript SDK package bundles a native Claude Code binary as an optional dependency and can be pointed at a local `claude` binary if needed. This fits a Node sidecar shipped alongside Rust artifacts. | Requires Python 3.10+ plus Python package management in release environments. Local Python is available, but it adds a second runtime packaging path. |
| Runtime boundary | Node sidecar can speak JSONL over stdio, matching the Codex app-server transport style already added. | Python can also speak JSONL, but it would add Python-specific dependency bootstrapping and virtualenv management. |
| Streaming | The TypeScript SDK `query()` stream is the current package's primary interface. | Python `query()` streams messages too. |
| Cancellation | TypeScript exposes `AbortController` on `query()` and `interrupt()` in streaming input mode. This must still be live-probed before marking Claude cancellation support. | Python docs say `query()` does not support interrupts; `ClaudeSDKClient` does. That is useful, but it makes the first adapter depend on a stateful Python client object. |
| Sessions | TypeScript uses session options such as `resume`, `fork`, and `continue`; it no longer has the removed V2 session object. | Python has explicit `ClaudeSDKClient` for continuous conversation and session control. |
| Tool policy | TypeScript supports `allowedTools`, permission modes, hooks, MCP, and `canUseTool`. | Python supports the same broad policy concepts, but `can_use_tool` has a documented streaming-mode hook workaround. |
| Testability | A Node fake sidecar can deterministically emit JSONL SDK-message fixtures without importing the SDK in Rust tests. | Same possible, but requires Python harness support. |

## Auth Policy

Strict native Claude mode must require API/provider auth configuration, not only
interactive Claude subscription login. Supported auth postures:

- `ANTHROPIC_API_KEY` through a credentials ref.
- Bedrock, Vertex, Claude Platform on AWS, or Azure Foundry environment/config
  refs where explicitly configured.

Local `claude auth status` can be a diagnostic signal, but it is not sufficient
for a release validation instance.

## Cancellation Policy

Do not mark Claude cancellation supported yet.

Next validation must answer:

- Does TypeScript `AbortController` terminate the provider turn and produce a
  distinct SDK terminal message/error?
- Does streaming-input `interrupt()` produce an acknowledgement or only a final
  observed state?
- Can cancellation race with a successful final result, and how many terminal
  messages can appear?
- Does cancellation preserve a resumable session id?

Until this is live-probed, Claude capabilities should advertise cancellation as
`request_only` or `unknown`, not provider-native stop.

## Next Work

`NP-032` added the minimal TypeScript sidecar/client with:

- deterministic fake JSONL sidecar test;
- optional live read-only smoke gated by `WHIPPLESCRIPT_CLAUDE_AGENT_SDK_LIVE=1`;
- report path `target/claude-agent-sdk-live-smoke.json`;
- redacted capture of session id, message type/subtype counts, result subtype,
  usage shape, and SDK/package version posture.

`NP-033` added the policy mapper from WhippleScript profiles/capabilities to
Claude `allowedTools`, `disallowedTools`, `permissionMode`, settings sources,
MCP config refs, and workspace policy checks. The initial mapper supports
`repo-reader`, `repo-writer`, and `human-review`, rejects destructive tools
without explicit approval mode, and blocks destructive capabilities in
read-only workspaces.

`NP-034` should make the sidecar/store evidence path durable for session ids,
message/event counts, tool/hook event summaries, usage shape, terminal result,
and redacted artifact refs.
