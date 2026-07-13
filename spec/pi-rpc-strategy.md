# Pi RPC Adapter Strategy

Status: validated initial adapter choice.

Date: June 1, 2026.

## Decision

Use `pi --mode rpc` as the first native Pi adapter surface.

The Pi SDK remains a useful reference and fallback path, but the first
WhippleScript adapter should treat Pi as a subprocess with a JSONL RPC protocol
over stdin/stdout.

## Validation

Local validation command:

```sh
scripts/check-pi-rpc-surface.sh
WHIPPLESCRIPT_PI_RPC_INTERRUPT_LIVE=1 scripts/check-pi-rpc-interrupt-smoke.sh
```

Observed locally:

```text
pi 0.73.0
@earendil-works/pi-coding-agent 0.78.0
@mariozechner/pi-coding-agent 0.73.1
```

The probe starts:

```sh
pi --mode rpc --no-session --offline
```

and sends:

```json
{"id":"state-1","type":"get_state"}
```

The local CLI returns a successful `response` for `get_state` with a session id,
model provider, model id, and `isStreaming` state. In this environment, `pi
--help` and `pi --version` write their human-readable output to stderr while
exiting successfully, so readiness checks must capture both stdout and stderr.

The live interrupt probe starts RPC mode with tools disabled, accepts a prompt,
sends `abort` at `turn_start`, and observes assistant `stopReason: "aborted"`
with exactly one `turn_end` before the successful abort acknowledgement.

## Rationale

- WhippleScript is Rust, and Pi RPC mode gives us a language-neutral process
  boundary without embedding Pi internals into the main runtime.
- The Pi docs describe RPC mode as the headless JSON protocol intended for
  embedding in other applications.
- The local RPC `get_state` probe already proves session identity and model
  selection are observable without starting a real streaming turn.
- The CLI exposes `--tools`, `--no-session`, `--session`, `--provider`,
  `--model`, extension, skill, and resource-loading flags that map cleanly to
  WhippleScript provider binding policy.
- The SDK package can still be used later if RPC lacks a required control, but
  it should not be introduced before a concrete gap is validated.

## Open Questions

- Artifact and edit-diff surfaces need live validation with a disposable
  workspace before they become release gates.
- Runtime lifecycle normalization must preserve Pi's ordering where the
  terminal `turn_end` can arrive before the `abort` command acknowledgement.

## Next Work

The next runtime work is lifecycle normalization:

- map `agent_start`, `turn_start`, message events, `turn_end`, and `agent_end`
  into canonical `agent.turn.*` events,
- treat `message.stopReason: "aborted"` as the Pi terminal cancellation signal,
- preserve abort acknowledgement as a cancellation acknowledgement even when it
  arrives after `turn_end`,
- keep redacted summaries as the durable evidence surface instead of raw
  transcripts or tool payloads.
