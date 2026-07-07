# DR-0035 — The delegated harness wire protocol

Status: accepted (2026-07-07; Decision 1 ratified as Option A — obligations
over dialects). The D8-6 follow-on under DR-0034 (managed vs.
delegated harnesses): the wire-level formalization of the delegated turn
contract that DR-0034 absorbed "at the architecture level" from the original
candidate scope flagged in `spec/compute-plane-design-note.md`. Cross-refs:
DR-0034 (the class split this protocol serves), DR-0024 (owned harness — the
*other* class, out of scope here), `spec/admission-and-idempotency.md` (the
uncertain-terminal rules the recovery obligations realize),
`spec/native-provider-surfaces.md` + `spec/claude-agent-sdk-strategy.md`
(pre-DR groundwork this record supersedes as contract SSOT),
`spec/compute-plane-design-note.md` (Class-B sidecar turns ride this protocol
when the compute plane ships).

## Problem

WhippleScript drives three delegated runtimes today, and each speaks its own
dialect against one implicit Rust boundary:

| | Claude | Codex | Pi |
|---|---|---|---|
| Transport | whip-owned Node sidecar (`scripts/claude-agent-sdk-sidecar.mjs`), JSONL over stdio | `codex app-server`, JSON-RPC 2.0 over stdio lines | `pi --mode rpc --no-session`, bespoke JSONL |
| Process lifecycle | one process per turn | persistent, per-turn ephemeral thread | persistent, single session |
| Handshake | none | `initialize` (result discarded) | none (`get_state` doubles as liveness) |
| Turn start | `run/start {run_id, request}` | `thread/start` + `turn/start` | `get_state` + `prompt` |
| Policy projection | allowed/disallowed tools + permission mode + setting_sources | sandbox mode + approval policy (+ `networkAccess:false`) | tool allowlist (`read/edit/write/bash`) |
| Terminal signal | `claude.turn.{completed,failed,cancelled}` | `turn/completed` (status field) | `turn_end` (stopReason/errorMessage) |
| Cancel verb | `run/cancel` (cooperative) | `turn/interrupt {threadId,turnId}` (NativeStop, before-terminal) | `abort` (NativeStop, ack-may-follow-terminal) |
| Version on the wire | none | clientInfo sent, server reply ignored | none |

The *shared* contract — the obligations a delegate must meet for the turn to be
admissible as evidence — exists only as conventions in the adapters and the
driver loop. The survey behind this record (2026-07-07, all three adapters +
`provider.rs`/`native_lifecycle.rs`/the kernel driver) found those conventions
load-bearing and leaky:

1. **run_id echo** is required on every inbound frame, enforced so strictly on
   the Claude path that a frame with the *wrong* run id aborts the whole turn
   as a `Protocol` error — and the sidecar's own pre-run `run/error` frames
   carry `run_id: null`, so the error the sidecar is trying to report is
   misrouted into exactly that abort.
2. **Exactly-one-terminal** is assumed, never enforced. The driver stops at the
   first terminal; a second terminal or post-terminal frames are silently
   truncated, and nothing requires a terminal to ever arrive.
3. **There is no liveness clock.** No wall-clock read timeout anywhere; the
   only bound is `max_events` (default 256), and an `Ok(None)` poll *consumes*
   one budget unit, so the budget conflates "events delivered" with "poll
   attempts". A sidecar that holds its pipe open silently blocks the worker
   thread indefinitely.
4. **Cancellation is half-wired.** The ack-is-`Diagnostic`, terminal-follows
   shape is right (and live-validated for Pi), but `cancel_turn` is dead code:
   the kernel driver loop has no path that invokes it.
5. **No version negotiation.** Two of three dialects exchange nothing; the one
   handshake that exists (Codex `initialize`) throws the reply away.
   `ProviderCapability.protocol_version` is a Rust-side string never checked
   against the live peer.
6. **Recovery cannot re-query.** A worker crash between run-start and terminal
   always resolves to the `uncertain` terminal, even for providers whose
   surface could answer "what happened to run X" — the capability is not
   declared, so `spec/admission-and-idempotency.md`'s idempotent-re-query
   branch is unreachable.
7. **Redaction is an implementation, not an obligation.** The shape-only
   boundary (prompt→shape, payload→shape, provider_error capped at 300 chars,
   secret-scrubbed URIs) is enforced kernel-side and mirrored in the Claude
   sidecar's pre-redaction, but nothing states which side *owns* the guarantee
   for an out-of-repo sidecar.

One correction to the record it extends: DR-0034's problem statement says
"(pi retired)". That is stale — `pi` is in the parser's supported-kind set,
its dispatch arm is unconditionally compiled (unlike feature-gated codex and
claude), and it classifies Delegated. This record treats Pi as a live,
conforming dialect.

## Decision 1 — Formalize obligations, not one dialect

**The protocol is a conformance contract over the existing dialects, not a
single wire format all providers must speak.**

The fork considered:

- **Option A — obligations over dialects.** Keep each adapter's native
  transport (Codex stays JSON-RPC app-server, Pi stays RPC-mode JSONL, Claude
  stays the whip sidecar). Formalize the *class-level obligations* (Decisions
  2–8) that every dialect must witness, enforce them once in the kernel driver
  where they are checkable, and keep the per-dialect mapping in the adapter.
- **Option B — one whip sidecar protocol.** Generalize the Claude sidecar's
  JSONL dialect (`run/start`, `run/cancel`, `run/error`, typed events) into
  *the* protocol; Codex and Pi get wrapping sidecars or in-process shims.

Option B buys one fake for all tests, one framing to document, and symmetric
transport for the compute plane's Class-B containers. But it wraps Codex's
JSON-RPC — a real protocol with ids, server-initiated requests, and approvals —
inside a second protocol, adds a process hop to Pi for no capability gain, and
makes the wrapping sidecar itself load-bearing infrastructure per provider
(the thing DR-0034 argued we should stop pretending we control). The delegate's
value is that it brings its own runtime; forcing its surface through our
framing repeats the setting-sources mistake at the transport layer.

Option A matches how the guarantees actually flow: the *kernel driver* is the
single enforcement point (it already normalizes all three dialects into one
lifecycle), so obligations enforced there are enforced for every dialect at
once, including future ones. The whip JSONL sidecar dialect remains — as **the
dialect of choice for SDK-shaped providers** (surfaces that are a library, not
a protocol; Claude today), documented in §Sidecar dialect below, but as *one
conforming transport*, not the universal one.

## Decision 2 — The turn envelope is the Rust boundary type

`NativeProviderTurnRequest` is the canonical turn envelope; every dialect's
turn-start message is a *projection* of it, and nothing may cross that the
envelope does not carry. Canonical fields (all shipped today): identity
(`provider_id`, `provider_kind`, `surface`, `run_id`, `effect_id`, `agent`,
`profile`), payload (`prompt_json`), authority (`workspace_policy`,
`required_capabilities`, `credential_ref`), behavior (`cancellation_depth`,
`artifact_policy`, `provider_options` — which carries `cwd`, `model`, and the
DR-0034 `settings` knob).

The **policy projection** is dialect-specific by design (Decision 7 of
DR-0034: authority is WhippleScript's; the *encoding* is the provider's):
Claude projects to allowed/disallowed tools + permission mode +
setting_sources; Codex to sandbox mode + approval policy with
`networkAccess: false`; Pi to a tool allowlist. The projection function per
dialect (`build_claude_agent_tool_policy`, `build_codex_app_server_policy`,
`build_pi_rpc_tool_policy`) is part of the adapter's conformance surface, and
its output is what the D8-2 attestation's `policy_hash` covers — the hash
commits to *what was projected*, so the attestation is checkable against the
projection function.

Obligation: a projection may only **narrow** the granted authority, never
widen it (the DR-0024/skills allowed-tools rule, restated at the wire). The
`delegated-settings-authority.maude` bite already covers the ambient-config
half; the projection half is enforced by the builders' error paths
(`profile_denied`, `workspace_denied`, `missing_approval`).

## Decision 3 — Event taxonomy, ordering, and the terminal contract

Every dialect normalizes to the eight lifecycle kinds (`started`, `streamed`,
`tool_requested`, `artifact_captured`, `completed`, `failed`, `timed_out`,
`cancelled`) plus the kernel-side non-terminal `Diagnostic`. The obligations:

- **T1 — exactly one terminal.** A run emits at most one terminal frame, and
  the driver *enforces* it: after a terminal is recorded, further frames for
  that run are recorded as protocol-violation diagnostics (evidence, kind
  `agent.turn.protocol_violation`), never as lifecycle events, and never
  reopen the run. (Today: assumed, silently truncated.)
- **T2 — a terminal must arrive or the kernel synthesizes one.** The
  `TimedOut` backstop stays; see Decision 4 for the clock that triggers it.
- **T3 — run_id echo, tolerant routing.** Every frame carries the run id.
  A frame with an *unknown or mismatched* run id is dropped and recorded as a
  protocol-violation diagnostic — it does not abort the turn. A frame with a
  `null` run id is routed as a *channel-level* error if it is an error frame
  (fixing the Claude sidecar's misrouted pre-run `run/error`), else dropped
  with a diagnostic.
- **T4 — ordering is arrival order.** `started` before terminal is *not*
  required (a turn may fail before starting); `sequence` is optional and
  informational; in-turn kinds (`streamed`, `tool_requested`,
  `artifact_captured`) derive evidence only, never rule-matchable facts (the
  existing `derives_rule_matchable_fact` line holds).
- **T5 — unknown event types are skipped**, not errors (already true in all
  three normalizers; now stated). This is the forward-compatibility valve in
  place of per-event versioning.

## Decision 4 — Liveness is a two-clock bound

Replace the single `max_events` bound with two independent clocks:

- **Inactivity clock (wall time):** if no frame arrives within
  `WHIPPLESCRIPT_NATIVE_PROVIDER_INACTIVITY_TIMEOUT` (default: 300s), the
  driver synthesizes the `TimedOut` terminal. This requires the transports'
  blocking `read_line` to gain a read timeout — today a silent-but-open pipe
  blocks the worker thread forever, unkillable by any budget.
- **Event budget (`max_events`):** stays as the runaway-stream backstop, but
  counts only *delivered frames* — an `Ok(None)` poll no longer consumes
  budget (today's conflation turns 256 empty polls into a spurious timeout).

Rationale for two clocks rather than one: wall time catches the hung delegate
(the dangerous case — a blocked worker thread in a capacity-bounded pool);
the event budget catches the pathological delegate that streams forever at
high frequency, which a wall clock alone never trips.

## Decision 5 — Cancellation: ack, then terminal, then uncertainty rules

The validated shape becomes the contract: a cancel request produces a
**non-terminal acknowledgement** (`Diagnostic`), and the run still ends with
exactly one terminal frame (normally `cancelled`), which may — per dialect —
arrive before or after the ack (Codex: before-terminal; Pi: ack may trail the
terminal; both already declared via `ProviderCancellationPolicy`). Depth
gating stays Rust-side per `CancellationDepth` (the wire carries no depth).

Two consequences:

- **Wire the driver.** `cancel_turn` exists on the trait, is tested, and is
  never called: the kernel loop has no cancel path. The build work plumbs an
  external cancel signal into `run_native_agent_turn_with_metadata` so a
  workflow-initiated cancel actually reaches the delegate. Until that lands,
  cancellation-depth declarations are honest but inert.
- **Ack without terminal resolves by the recovery rules.** If the ack arrives
  and the terminal never does, the inactivity clock fires and the
  started-without-terminal run resolves per
  `spec/admission-and-idempotency.md`: re-query if the dialect supports it
  (Decision 6), else the explicit `uncertain` terminal.

## Decision 6 — Declare re-query; recovery uses it before `uncertain`

`ProviderCapability` gains a **re-query declaration**: whether the surface can
answer "what is the terminal state of run/thread/turn X" idempotently, and by
which identity fields. `recover_running_provider_runs` consults the
declaration: re-query and admit the discovered terminal when supported,
resolve `uncertain` otherwise — making the admission spec's re-query branch
reachable for the first time. A dialect that declares re-query and answers it
wrongly (two different terminals for one run) violates T1 and is a
protocol-violation diagnostic, not a second admission (the idempotency-key
unique index already absorbs the duplicate).

**Build finding (2026-07-07, B4 deferred with cause):** every delegated
surface today is a *subprocess of the worker* — the Claude sidecar, the codex
app-server, and the pi RPC process all die with the worker whose crash
recovery would want to re-query them. There is no peer left to answer
`run/query`, so a re-query declaration would be machinery with zero possible
implementations (the original "codex thread/turn ids survive" hunch is wrong
for the same reason: the app-server is our child process). Re-query becomes
real exactly when a delegate outlives the worker — the compute plane's
remote Class-B containers (`spec/compute-plane-design-note.md`) — so B4 is
deferred to that build, where the capability declaration, the `run/query`
verb, and the recovery integration land together against a surface that can
actually answer.

## Decision 7 — Version is exchanged and pinned, not assumed

Every dialect exchanges a protocol identifier at its natural handshake point,
and the adapter checks it against `ProviderCapability.protocol_version`:

- Whip sidecar dialect: `run/start` gains `protocol: "whip-sidecar/1"`; the
  first frame back echoes it (a sidecar that doesn't echo is treated as `/1`
  for one release, then required).
- Codex: keep sending `initialize`, and **consume the reply** — record the
  server's advertised info as evidence and fail fast on a schema-incompatible
  peer (the schema pin already exists in the surface gate; this moves the
  check onto the live connection).
- Pi: `get_state` already returns version-adjacent state; pass through what it
  exposes as evidence; no hard pin until the surface exposes one.

Mismatch policy: incompatible → `provider_health` binding block (recoverable,
pre-turn), never a mid-turn failure. The doctor's declarative `health_checks`
gain the live version probe.

## Decision 8 — Redaction is the kernel's obligation; sidecar pre-redaction is defense in depth

The shape-only boundary is **owned by the kernel**: prompt and payloads cross
into evidence as shapes, provider errors as capped (300-char) secret-scrubbed
strings, artifact URIs/hashes scrubbed — regardless of what the peer sends.
A whip-owned sidecar (the Claude `.mjs` today) must *additionally* pre-redact
before emitting (raw SDK content never touches its stdout) — that is a
conformance requirement for sidecars we ship, and unenforceable-by-design for
foreign processes, which is exactly why the kernel-side enforcement is the
guarantee and the sidecar-side is depth. This is the wire-level restatement of
DR-0034 Decision 5: the delegated evidence model is attestation + shapes, not
transcripts.

## The whip sidecar dialect (normative for whip-owned sidecars)

Frames are newline-delimited JSON objects over stdio, stderr ignored.
Client→sidecar: `run/start {type, run_id, protocol, request}`,
`run/cancel {type, run_id}`, `run/query {type, run_id}` (optional, Decision 6),
`run/close {type}`. Sidecar→client: events
`{type, run_id, payload}` with types `claude.session.started`-style
provider-prefixed names normalized per Decision 3, and channel errors
`run/error {type, run_id|null, payload:{code, message}}`. Error frames with
`run_id: null` are channel-level (Decision 3 T3). One process may serve many
runs; the Rust adapter currently drives one run per process and `run/close` is
optional. The request object inside `run/start` is the Decision 2 projection
(for Claude: prompt, cwd, model, allowed/disallowed tools, permission mode,
setting_sources [omitted = provider default], mcp_config_ref).

## Formal coverage (model before the enforcement build)

Per repo discipline, the enforcement changes land model-first. One Maude model
(`models/maude/delegated-wire-lifecycle.maude`) covering: exactly-one-terminal
(a second terminal is unreachable as an *admitted* lifecycle event — T1 bite),
ack-is-non-terminal (cancel ack never completes a run — bite), post-terminal
frames land as diagnostics not events (bite), and the two-clock timeout always
produces a terminal (coverage: every non-terminated run reaches `timedOut`).
Keep the state space finite: consume trigger tokens (the
delegated-settings-authority lesson).

## Build items (ratified 2026-07-07; built same day except B4)

- [x] B1 — T1/T3 enforcement in the kernel driver: post-terminal and
      misrouted frames → `agent.turn.protocol_violation` diagnostics; Claude
      client stops aborting on unexpected run ids; null-run_id `run/error`
      routed as channel error. (d88579e)
- [x] B2 — Two-clock liveness: reader-thread transports with read timeouts +
      adapter inactivity clock (`WHIPPLESCRIPT_NATIVE_PROVIDER_INACTIVITY_TIMEOUT`,
      default 300s) synthesizing the TimedOut terminal; `max_events` counts
      delivered frames only. Residual: codex/pi turn-start JSON-RPC requests
      remain blocking (fast path). (551bb09)
- [x] B3 — Cancel plumb-through: the driver consults the durable
      `effect_cancellation_requests` surface each iteration and calls
      `cancel_turn` at the authorized depth; adapters poll in ≤1s slices so
      the check is responsive; refused cancels are diagnostics. Residual:
      pre-start cancellation short-circuit (owned harness has it; delegated
      does not yet). (95bb399)
- [ ] B4 — Re-query: capability declaration + `run/query` in the sidecar
      dialect + recovery integration (admission spec's re-query branch).
      **DEFERRED with cause** (see Decision 6 build finding): impossible for
      subprocess delegates; lands with the compute plane's remote Class-B
      containers.
- [x] B5 — Version exchange: `hello` handshake + `protocol` field in the
      sidecar dialect (answered-mismatch → binding block; legacy and dead
      sidecars tolerated — liveness stays a start_turn concern); Codex
      `initialize` reply consumed as started-event evidence; pi declares no
      version surface yet. Residual: doctor live version probe. (c96d966)
- [x] B6 — Maude `delegated-wire-lifecycle.maude`, 3 coverage + 3 bites,
      landed before B1–B3. (a5a8618)

## Open questions

- **`tool_requested`/`hook.event` on the Claude path**: the normalizer
  understands them; the shipped sidecar never emits them (tool_use is folded
  into stream shapes). Require the sidecar to emit them (richer attestation of
  tool activity inside the delegate) or drop them from the Claude dialect?
- **Usage/token capture**: Codex and Pi capture none today; the Claude summary
  keeps only a usage *shape*. Worth an optional `usage` field on terminal
  frames (spend evidence feeds `std.spend` gauges) or out of scope?
- **Artifact dedup**: the driver records every artifact ref on every event;
  content-hash dedup is deferred to the versioned-workspace boundary work.
- **Fixture placement** (inherited from DR-0034): `native-fixture` dispatches
  as Delegated but reports the Managed `fixture` capability — the test-adapter
  split should be confirmed when B1 lands, since protocol-violation evidence
  will be exercised through it.
