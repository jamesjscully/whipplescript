# Native Provider Implementation Tracker

Status: active execution tracker for usable Codex and Claude provider
support.

Date: May 31, 2026.

This tracker turns the native-provider shipping checklist into ordered work. It
is intentionally stricter than the command-backed harness path: no adapter task
is considered complete until its assumptions are validated against the provider
surface being used.

Progress states:

```text
[ ] not started
[~] in progress
[x] complete
[!] blocked
[-] deferred with rationale
```

## Current Focus

Build the shared provider capability and configuration layer first. Codex and
Claude have different native shapes, so the shared layer must describe
capabilities and validation results rather than hide everything behind a generic
command wrapper.

Current next task: expand live disposable workspace/model stress and
provider-generated attachment coverage beyond fixed artifact fixtures. Codex and
Claude source-workflow bridge smokes now pass live and include
store-reopen replay/recovery no-op checks through `whip recover`.
Native provider launch failures now route
through the native boundary path, so an unavailable Codex/Claude command
records a durable failed run and diagnostic instead of leaving the effect queued.
Native fixture stress coverage now proves several source workflow effects can
flow through the native bridge with one terminal and one artifact event each.
`scripts/check-native-provider-endpoint-health.sh` consolidates the existing
Codex app-server and Claude Agent SDK endpoint/session probes into a
single redacted readiness artifact, with live mode controlled by the release
strict-external flag.
Source-declared `repo.write` effects now fail durably at the native boundary for
Codex and Claude when the workflow bridge requests a read-only workspace.
Live Codex and Claude fixed-file artifact fixtures now create artifacts in
temporary disposable workspaces, remove those workspaces, and persist only
redacted metadata/hash evidence; Claude requires Agent SDK `bypassPermissions`
plus the explicit skip-permissions option and an exact disposable artifact path
for this writable fixture, because terminal completion alone did not prove the
file landed in the verified workspace.
Codex app-server approval-capable server requests have
deterministic decline-and-record coverage, and the installed app-server has live
invalid-turn error-shape coverage; previous Codex CLI `0.128.0` file changes were
observed as `item/fileChange/outputDelta`, `turn/diff/updated`, and
`fileChange` item notifications rather than server-side approval requests under
`workspace-write`/`on-request`. Live Claude unavailable-write probes currently
end as `error_max_turns`, so this surface does not expose a stable provider-emitted
tool-denial event shape to gate yet.

## Ground Rules

- Validate external-system behavior before adding runtime plumbing.
- Keep command-backed harnesses as compatibility/test surfaces only.
- Do not persist secret values, raw auth payloads, or unredacted provider
  request/response bodies.
- Treat Codex app-server schema and Claude Agent SDK semantics as separate
  adapter surfaces.
- Every provider feature needs a deterministic fake or isolated fixture before
  it becomes a release gate.
- Native-provider strict mode must fail when only command-wrapper coverage is
  present.

## Milestone Summary

| Milestone | Status | Goal | Exit Criteria |
| --- | --- | --- | --- |
| NP-000 Surface validation | [x] | Confirm supported Codex and Claude integration surfaces. | `scripts/check-native-provider-surfaces.sh` passes and `spec/native-provider-surfaces.md` records the assumptions. |
| NP-010 Capability/config layer | [x] | Represent provider capabilities, binding config, health, and validation without starting provider turns. | Runtime and CLI tests cover valid/invalid provider bindings, health output, cancellation-depth support, artifact support, redacted diagnostics, and the shared native adapter contract. |
| NP-020 Codex native spike | [x] | Start a Codex app-server session and record thread/turn metadata from the native protocol. | Isolated read-only smoke records thread/turn ids, start/completion events, diff artifact evidence, redacted evidence, validated interrupt behavior, live invalid-turn error response shape, and live source workflow execution through the native bridge. |
| NP-030 Claude native spike | [x] | Start a Claude Agent SDK session and record session/stream metadata. | Isolated read-only smoke records session id, stream messages, tool policy, usage/cost shape, artifact fixture metadata, live local-auth cancellation behavior, live config-error shape, live source workflow bridge execution, and redacted evidence. |
| NP-050 Lifecycle normalization | [x] | Normalize native provider events into durable `agent.turn.*` events/facts. | Kernel and CLI tests cover Codex/Claude event normalization, durable lifecycle events/facts, native adapter event bridging into run terminal outcomes/artifact rows, redacted native-event evidence, and status/runs/trace exposure. |
| NP-060 Provider-native cancellation | [x] | Implement capability-declared cancellation per provider. | Cancellation request, acknowledgement, late completion, timeout, unsupported-cancellation, Codex native stop, and Claude live interrupt-normalization tests pass without duplicate terminal outcomes. |
| NP-070 Artifacts and evidence | [~] | Capture artifacts, diffs, transcripts, stream chunks, and capture failures. | Required artifact capture failure prevents false success; redaction fixtures prove no secrets leak; Codex live diff artifact capture and Claude live fixed-file fixture capture are validated, with broader provider-generated attachment coverage still open. |
| NP-080 Recovery | [x] | Reconcile running provider runs after worker/store failure. | Fault injection covers failure after provider completion, artifact capture, and terminal append. |
| NP-090 Real-provider validation | [x] | Validate native adapters beyond command-wrapper smoke coverage. | Strict native mode rejects command-wrapper-only providers and emits per-provider reports, including native source-workflow bridge, replay, error-shape, and policy-denial gates. |
| NP-100 Release readiness | [x] | Make native-provider ship gates enforceable. | Release readiness requires native surface, adapter, cancellation, artifact, recovery, and real-provider reports in strict mode. |

## Assumption Ledger

| Provider | Assumption | Status | Validation |
| --- | --- | --- | --- |
| Codex | Native adapter should target app-server JSON-RPC, not `codex exec` text output. | [x] | Local schema generation plus Codex app-server docs in `spec/native-provider-surfaces.md`. |
| Codex | Cancellation can map to `turn/interrupt`. | [x] | Live interrupt smoke records `{}` acknowledgement, terminal `interrupted` status, and exactly one `turn/completed` notification. |
| Codex | Diffs/artifacts can be observed through app-server notifications. | [x] | `WHIPPLESCRIPT_CODEX_APP_SERVER_ARTIFACT_LIVE=1 scripts/check-codex-app-server-artifact-smoke.sh` validated live `turn/diff/updated` notifications for a provider-created file; the report persists only redacted basename, diff byte count, hash, and fixture metadata. |
| Claude | Native adapter should target Agent SDK semantics, not plain `claude -p` text mode. | [x] | Anthropic Agent SDK docs and local CLI shape captured in `spec/native-provider-surfaces.md`. |
| Claude | Local developer validation should reuse installed Claude auth, while CI/strict live jobs stay opt-in. | [x] | Live Claude Agent SDK smoke, interrupt, and artifact gates accept either embedded auth posture (`ANTHROPIC_API_KEY`, Bedrock, Vertex) or local `claude auth status` login. CI does not run live harnesses unless explicitly dispatched. Reports record auth posture without email/org values. |
| Claude | Cancellation and interruption semantics need SDK-level validation. | [x] | Live local-auth Agent SDK interrupt returns a `result` with subtype `error_during_execution`; the sidecar normalizes that terminal to `claude.turn.cancelled` when a cancel request is in flight and records acknowledgement `interrupt` without duplicate terminal outcomes. |

## NP-010 Capability And Config Layer

| Task | Status | Description | Validation |
| --- | --- | --- | --- |
| NP-011 | [x] | Add provider capability structs for surface kind, protocol/schema version, session identity fields, stream event kinds, tool policy shape, cancellation depth, artifact support, health checks, and auth requirements. | Kernel provider tests cover built-in Codex, Claude, and fixture capabilities. |
| NP-012 | [x] | Add provider binding config shape with provider id, provider kind, adapter surface, credentials ref, profile ids, model/runtime defaults, workspace policy, timeout, cancellation policy, artifact policy, and health-check settings. | Config parser tests cover valid Codex config, invalid enum values, invalid mixed-surface configs, and unsupported cancellation depth. |
| NP-013 | [x] | Add redacted provider validation results: pass/fail/skip, phase, code, message, provider, surface, retryability, and missing config refs. | Kernel and CLI tests prove extra/secret config values are not emitted in validation JSON. |
| NP-014 | [x] | Wire provider validation into `whip doctor` and JSON output. | `doctor --provider-config` parser and validation tests cover redacted provider config validation. |
| NP-015 | [x] | Teach release readiness strict mode to require native capability validation, not just command availability. | `scripts/check-native-provider-configs.sh` validates Codex and Claude native bindings; release readiness runs it with strict mode tied to `WHIPPLESCRIPT_RELEASE_STRICT_EXTERNAL`. |
| NP-016 | [x] | Record provider capability and validation evidence in store/evidence surfaces without starting a provider run. | Store test verifies provider validation evidence refs and reopen stability; `whip doctor --record-provider-evidence` records parsed binding evidence without starting provider runs. |

## NP-020 Codex Native Spike

| Task | Status | Description | Validation |
| --- | --- | --- | --- |
| NP-021 | [x] | Generate and pin/check Codex app-server schema metadata for the installed CLI. | `scripts/check-codex-app-server-schema.sh` pins reviewed `codex-cli 0.137.0` schema metadata, accepts other installed Codex versions when the adapter methods still exist, and fails if required methods disappear: `initialize`, `thread/start`, `turn/start`, `turn/started`, `turn/completed`, `turn/interrupt`, `turn/diff/updated`. Exact metadata matching is available with `WHIPPLESCRIPT_CODEX_APP_SERVER_SCHEMA_STRICT_PIN=1`. |
| NP-022 | [x] | Implement a minimal app-server transport client for stdio JSONL or unix socket. | Deterministic fake transport tests cover request/response, notification buffering, malformed response, remote error response, timeout, and adapter-level remote start-error mapping without raw message leakage; `WHIPPLESCRIPT_CODEX_APP_SERVER_ERROR_LIVE=1 scripts/check-codex-app-server-error-smoke.sh` validates the installed app-server's JSON-RPC error response shape. |
| NP-023 | [x] | Start a read-only Codex thread/turn with explicit workspace and profile policy. | `WHIPPLESCRIPT_CODEX_APP_SERVER_LIVE=1 scripts/check-codex-app-server-live-smoke.sh` records thread id, turn id, lifecycle notifications, and terminal status in a redacted report. |
| NP-024 | [x] | Capture Codex approvals/tool requests and diff notifications as evidence. | Kernel evidence summarizer has deterministic fake coverage for approval requests, tool requests, diff notifications, and item notifications without raw transcript/diff capture; the app-server adapter now answers approval-capable provider requests conservatively while recording them as `agent.turn.tool_requested`; store helper records the redacted summary with provider/thread/turn links; live artifact smoke validates Codex `turn/diff/updated` for a provider-created file in a disposable workspace. |
| NP-025 | [x] | Implement Codex `turn/interrupt` cancellation path after live behavior is validated. | `WHIPPLESCRIPT_CODEX_APP_SERVER_LIVE=1 scripts/check-codex-app-server-interrupt-smoke.sh` records request, `{}` acknowledgement, terminal `interrupted` status, and no duplicate terminal outcome. |

## NP-030 Claude Native Spike

| Task | Status | Description | Validation |
| --- | --- | --- | --- |
| NP-031 | [x] | Choose TypeScript or Python Agent SDK embedding strategy and document why. | `spec/claude-agent-sdk-strategy.md` chooses a TypeScript sidecar and compares process boundary, auth, streaming, cancellation, packaging, and testability; `scripts/check-claude-agent-sdk-surface.sh` records local/package posture, requires declared TypeScript SDK configuration, and treats registry metadata as informational unless `WHIPPLESCRIPT_CLAUDE_AGENT_SDK_STRICT_REGISTRY=1`. |
| NP-032 | [x] | Add minimal Claude sidecar/client that streams one read-only turn. | Rust JSONL client has deterministic fake tests; `scripts/claude-agent-sdk-sidecar.mjs` streams fake or live Agent SDK events; `scripts/check-claude-agent-sdk-live-smoke.sh` validates the fake path by default and gates live read-only smoke behind `WHIPPLESCRIPT_CLAUDE_AGENT_SDK_LIVE=1`. |
| NP-033 | [x] | Map WhippleScript profiles/capabilities to Claude tools, permission mode, hooks, and MCP config. | `claude_agent_sdk` policy tests cover read-only mapping, writer/Bash mapping, MCP config ref propagation, forbidden tool/profile mismatch, forbidden workspace, missing approval, and missing profile. |
| NP-034 | [x] | Capture Claude session id, stream messages, hooks/tool events, usage/cost, and terminal result. | Kernel event summarizer redacts result/usage payloads; store helper records Claude Agent SDK evidence by provider session with provider/run links and reopen tests; deterministic adapter coverage proves remote start/event errors map to redacted native boundary failures without raw provider messages. |
| NP-035 | [x] | Validate and implement Claude cancellation/interruption semantics. | Anthropic TypeScript Agent SDK docs expose both `Query.interrupt()` and `Options.abortController`; the sidecar retains the live Query handle, requests native interrupt before abort fallback, and normalizes the observed live `error_during_execution` terminal after interrupt into one `claude.turn.cancelled` terminal with acknowledgement `interrupt`. Fake and live local-auth smokes pass. |
| NP-036 | [x] | Route source workflow effects to the Claude Agent SDK native bridge. | `whip worker/dev --provider claude` builds a read-only Agent SDK turn request, launches the existing sidecar through `RuntimeKernel::run_native_agent_turn`, and `WHIPPLESCRIPT_CLAUDE_NATIVE_WORKFLOW_LIVE=1 scripts/check-claude-native-workflow-smoke.sh` passes with local Claude auth. |

## NP-050 Lifecycle Normalization

| Task | Status | Description | Validation |
| --- | --- | --- | --- |
| NP-051 | [x] | Define canonical native lifecycle event names and payload fields. | `native_lifecycle` maps Codex and Claude provider events into canonical `agent.turn.started`, `agent.turn.streamed`, `agent.turn.tool_requested`, `agent.turn.artifact_captured`, and terminal event names with redacted payload shapes. |
| NP-052 | [x] | Derive `agent.turn.*` facts from native provider events. | `RuntimeKernel::record_native_agent_turn_observation` appends the canonical lifecycle event and derives a same-name fact; `RuntimeKernel::run_native_agent_turn` bridges native adapter events into durable runs, artifact rows, and terminal outcomes; kernel tests cover a fake native adapter completing through the bridge without raw payload leakage. |
| NP-053 | [x] | Preserve provider-specific raw metadata through bounded/redacted evidence refs. | Native lifecycle recording writes `agent.turn.native_event` evidence with provider event type, session/turn ids, terminal flag, status, and redacted provider payload shape; event/fact payloads carry the evidence id and tests prove raw text is not persisted. |
| NP-054 | [x] | Expose lifecycle in `status`, `runs`, `trace`, diagnostics, and evidence outputs. | `status --json` and `trace --json` include `native_lifecycle`; `runs --json` includes the latest native lifecycle status/evidence id per run; evidence output includes `agent.turn.native_event`; CLI helper tests cover redacted lifecycle summaries. Diagnostics remain terminal/provider-failure focused and link through evidence ids. |

## NP-060 Cancellation

| Task | Status | Description | Validation |
| --- | --- | --- | --- |
| NP-061 | [x] | Model cancellation depth in provider capabilities and binding policy. | Provider capability/config tests cover validated Codex `native_stop`, Claude cooperative/fake-only posture, and unsupported-depth failures before launch. |
| NP-062 | [x] | Add cancellation acknowledgement states to store/trace if current request-only model is insufficient. | Current store request model is sufficient for the validated Codex path: terminal completion marks requests `terminal`, and worker metadata records provider acknowledgement order; no new trace state needed yet. |
| NP-063 | [x] | Implement provider-specific cancellation for Codex and Claude only after each behavior is validated. | Worker cancellation policy distinguishes Codex/fixture ack-before-terminal; Claude sidecar live interrupt is validated through local Claude auth and normalized to cancellation after the provider emits `error_during_execution`. |
| NP-064 | [x] | Handle late completions after cancellation request/acknowledgement by explicit policy. | Tests cover unsupported-provider late completion after cancel request, supported-provider acknowledgement, duplicate worker acknowledgement suppression, timeout-after-cancellation request resolution, duplicate terminal rejection, and rejection of cancellation requests after terminal completion. |

## NP-070 Artifacts And Evidence

| Task | Status | Description | Validation |
| --- | --- | --- | --- |
| NP-071 | [x] | Define artifact manifest schema and retention/redaction policy. | `artifact_manifest` defines the canonical manifest schema, validates entry ids, path/ref URI shape, retention policy, redaction status, content hash, and entry counts; provider evidence now includes `artifact_manifest`, and kernel replay coverage proves artifact rows relink after projection rebuild. |
| NP-072 | [~] | Capture provider diffs, changed files, attachments, command/tool output refs, and transcript refs. | Command-backed providers now emit transcript/stdout/stderr reference artifacts into the canonical manifest; Codex live artifact smoke validates diff notifications for a provider-created file without storing patch bodies; Claude live artifact fixture mode passes with disposable acknowledgements and records only metadata/hash. The Claude live report also records redacted content-block/tool-use counts so completed-without-file responses cannot masquerade as artifact success. Remaining gap is provider-generated attachments beyond fixed file/diff fixtures. |
| NP-073 | [x] | Classify artifact failures as `artifact.capture.failed`. | `artifact_capture_failed_payload` classifies missing, unreadable, oversized, hash mismatch, and redaction-failure cases with redacted message metadata; `RuntimeKernel::record_artifact_capture_failure` appends the canonical event and diagnostic. |
| NP-074 | [x] | Prevent successful terminal completion when required artifact capture fails. | Kernel e2e-style regression proves a provider result that reports completed plus canonical `artifact.capture.failed` is normalized to failed terminal and `agent.turn.failed`, with no completed lifecycle event. |
| NP-075 | [x] | Add secret-seeded redaction fixture for durable provider records, artifact metadata, and real-provider reports. | `provider_secret_seed_never_reaches_durable_records` seeds an `sk-...` token through provider summary, stdout, stderr, transcript, failure message, raw JSON, artifact path, and artifact hash; the regression proves stored events, terminal metadata, facts, evidence, diagnostics, artifact rows, and artifact manifest metadata do not contain the raw secret. `whip artifacts --json <run-id>` lists metadata only and defensively redacts legacy/raw artifact path and hash values. `check-real-providers-report.sh` now also redacts preflight JSONL, Markdown selected-provider display, per-provider JSON checks, and report filenames; `scripts/check-real-provider-report-redaction.sh` verifies no raw token reaches report artifacts. |

## NP-080 Recovery

| Task | Status | Description | Validation |
| --- | --- | --- | --- |
| NP-081 | [x] | Fault-inject store failure after provider completion but before terminal append. | Provider evidence now records terminal status; `RuntimeKernel::recover_provider_terminal_from_evidence` appends exactly one recovered terminal event and lifecycle fact from persisted provider evidence, and a second recovery pass is a no-op. |
| NP-082 | [x] | Fault-inject store failure after artifact capture but before terminal append. | Regression simulates artifact row plus artifact-manifest provider evidence before terminal append; recovery preserves the artifact link and recovered terminal metadata references the captured artifact. |
| NP-083 | [x] | Reconcile running runs after worker restart. | `RuntimeKernel::recover_running_provider_runs` scans running runs after store reopen and recovers those with persisted command-backed provider evidence or terminal `agent.turn.native_provider` evidence; restart regressions prove exactly one terminal event is appended. `whip recover <instance>` exposes this path for operators, and the live Codex/Claude native workflow smokes reopen the store and assert recovery is an idempotent no-op after normal provider completion. |
| NP-084 | [x] | Bind completion idempotency keys to provider session/turn ids and terminal payload hash. | Provider terminal metadata now records provider correlation and terminal payload hash; completion idempotency keys include both, and duplicate/contradictory terminal completions roll back without appending a second terminal event. |

## NP-090 Validation And CI

| Task | Status | Description | Validation |
| --- | --- | --- | --- |
| NP-091 | [x] | Extend `scripts/check-real-providers.sh` with native-provider strict mode. | `WHIPPLESCRIPT_REAL_PROVIDER_NATIVE_STRICT=1` defaults to Codex/Claude, rejects coerce command-wrapper selections, requires native provider configs, and runs native surface probes instead of command-wrapper smoke gates. |
| NP-092 | [x] | Add isolated fixture requirements for destructive provider tests. | `scripts/check-real-providers.sh` now refuses destructive provider suites unless global or provider-specific disposable target markers are present with the exact acknowledgement string; reports record marker posture without values. |
| NP-093 | [x] | Emit per-provider validation reports with redacted environment posture and evidence refs. | `scripts/check-real-providers-report.sh` writes `target/real-provider-reports/<provider>.json` with set/unset environment posture, evidence refs, check summaries, and redacted preflight records. |
| NP-094 | [x] | Add optional CI matrix for native Codex and Claude validation. | `.github/workflows/native-provider-validation.yml` provides a workflow-dispatch Codex/Claude matrix in native-surface mode, uploads per-provider reports, and has a strict job that fails when required native configs or live prerequisites are absent. |

## NP-100 Docs And Release

| Task | Status | Description | Validation |
| --- | --- | --- | --- |
| NP-101 | [x] | Document native provider setup, credentials refs, health checks, workspace policy, cancellation, artifacts, and troubleshooting. | `docs/providers.md` now covers native surfaces, config/credential refs, validation commands, cancellation, artifacts, recovery, destructive-test gates, and CI reports; `docs/troubleshooting.md` covers native strict failures and provider report inspection. |
| NP-102 | [x] | Update API reference for provider capabilities, binding config, lifecycle events, and evidence/artifact JSON. | `docs/api-reference.md` now documents native provider config/capability JSON, validation result shape, lifecycle event payloads, terminal metadata hashes, artifact manifests, capture failures, and Rust module/method entrypoints. |
| NP-103 | [x] | Update release checklist with strict native-provider ship gates. | `spec/release-checklist.md` now has a native-provider release gate with exact Codex/Claude commands, strict real-provider validation command, disposable target requirement, workflow dispatch requirement, and required report artifacts. |
| NP-104 | [x] | Package provider config examples and doctor checks with release artifacts. | Added `examples/provider-configs/native/native.example.json`, package it as `native-provider-config-examples.tar.gz` in release workflow artifacts, and extended archive smoke to run packaged `whip doctor --provider-config` against it. |

## Current Work Log

| Date | Entry |
| --- | --- |
| 2026-05-31 | Validated local Codex and Claude surfaces and added `scripts/check-native-provider-surfaces.sh`. |
| 2026-05-31 | Added `spec/native-provider-surfaces.md` with provider-specific adapter implications. |
| 2026-05-31 | Added this execution tracker. |
| 2026-05-31 | Added kernel provider capability/config validation and wired redacted provider config checks into `whip doctor --provider-config`. |
| 2026-06-01 | Added strict native-provider config validation for release readiness. |
| 2026-06-01 | Added provider validation evidence recording for doctor config checks without starting provider runs. |
| 2026-06-01 | Added pinned Codex app-server schema metadata check for the installed Codex CLI. |
| 2026-06-01 | Added minimal Codex app-server JSON-RPC client with stdio transport and deterministic fake tests. |
| 2026-06-01 | Added and ran an opt-in live Codex app-server smoke for read-only thread/turn lifecycle validation. |
| 2026-06-01 | Added redacted Codex app-server evidence summaries and durable store evidence refs for approval requests, tool requests, diffs, and item lifecycle notifications. |
| 2026-06-01 | Added and ran an opt-in live Codex app-server interrupt smoke validating `turn/interrupt` acknowledgement and terminal ordering. |
| 2026-06-01 | Chose the TypeScript Claude Agent SDK sidecar strategy and added a local/package surface report check. |
| 2026-06-01 | Added the minimal Claude Agent SDK JSONL sidecar/client and deterministic fake read-only smoke. |
| 2026-06-01 | Added Claude profile/capability to Agent SDK tool-policy mapping with denial tests. |
| 2026-06-01 | Added redacted Claude Agent SDK event summaries and durable store evidence refs for session/run evidence. |
| 2026-06-01 | Added fake Claude Agent SDK cancellation smoke; live cancellation validation remains pending API/provider auth. |
| 2026-06-01 | Added native lifecycle normalization for Codex and Claude provider event shapes and store event/fact recording for canonical `agent.turn.*` observations. |
| 2026-06-01 | Added bounded `agent.turn.native_event` evidence refs for normalized native lifecycle observations without raw provider payload leakage. |
| 2026-06-01 | Exposed native lifecycle summaries through status, runs, trace, and evidence JSON surfaces. |
| 2026-06-01 | Added provider-native cancellation policy classification for Codex/fixture acknowledgement ordering; Claude remains unsupported until live Agent SDK cancellation validation. |
| 2026-06-01 | Added worker-level cancellation idempotency coverage: repeated worker passes after acknowledgement produce no duplicate terminal outcome. |
| 2026-06-01 | Added store coverage that cancellation requests after terminal completion are rejected without creating open requests. |
| 2026-06-01 | Completed cancellation timeout matrix coverage: timeout after an open cancellation request resolves the request, releases the lease, and rejects later duplicate terminal completion. |
| 2026-06-01 | Added canonical provider artifact manifest metadata with retention/redaction policy fields, validation tests, and replay relink coverage. |
| 2026-06-01 | Started artifact capture implementation: command providers emit transcript/stdout/stderr ref artifacts, and Codex App Server evidence records structured changed-file metadata from diff notifications without retaining raw diffs. |
| 2026-06-01 | Added `artifact.capture.failed` classification for missing, unreadable, oversized, hash mismatch, and redaction-failure cases, with redacted event payloads and linked diagnostics. |
| 2026-06-01 | Added required-artifact capture gating: completed provider results carrying canonical artifact capture failure are forced to failed terminal outcomes. |
| 2026-06-01 | Added provider-terminal append recovery from persisted `agent.turn.provider` evidence, covering the crash window after provider completion but before terminal append. |
| 2026-06-01 | Added artifact-before-terminal recovery coverage: captured artifact rows and manifest evidence survive terminal recovery and remain inspectable for the run. |
| 2026-06-01 | Added worker-restart reconciliation for running provider runs with persisted provider evidence, including idempotent duplicate recovery behavior. |
| 2026-06-01 | Bound provider terminal completion keys to provider correlation ids and terminal payload hashes, with contradiction tests proving later terminal outcomes roll back cleanly. |
| 2026-06-01 | Added native strict mode to real-provider validation so command-wrapper-only selections cannot satisfy Codex/Claude native-provider gates. |
| 2026-06-01 | Added disposable target marker gates for destructive provider tests and documented the required acknowledgement contract. |
| 2026-06-01 | Added per-provider real-provider validation reports with redacted environment posture and evidence refs. |
| 2026-06-02 | Added optional native-provider CI matrix for Codex and Claude, including report artifact uploads and strict dispatch gating. |
| 2026-06-02 | Documented native provider setup, credential/config expectations, validation modes, cancellation/artifact/recovery behavior, destructive fixture gates, and troubleshooting. |
| 2026-06-02 | Updated API reference for native provider capability/config, validation, lifecycle, terminal metadata, artifact manifest, and capture-failure JSON shapes. |
| 2026-06-02 | Added strict native-provider ship gates to the release checklist with exact validation commands and required report artifacts. |
| 2026-06-02 | Added packaged native provider config examples and archive-smoke doctor validation for the example config. |
| 2026-06-02 | Added secret-seeded durable-record redaction coverage for provider summaries, failure payloads, artifact paths/hashes, terminal metadata, facts, evidence, diagnostics, and manifests. |
| 2026-06-02 | Added metadata-only `whip artifacts <run-id>` inspection with defensive redaction and a reusable real-provider report redaction fixture in release readiness. |
| 2026-06-02 | Added non-live `whip doctor --providers` health posture for Codex and Claude CLI/credential checks without starting provider turns or exposing secret values. |
| 2026-06-02 | Added deterministic destructive-provider fixture gate regression for skip, missing-marker failure, acknowledged pass, and target-value redaction. |
| 2026-06-02 | Added Codex app-server profile/capability policy mapping with denial tests, matching Claude approval/workspace boundary coverage. |
| 2026-06-02 | Added a native-provider Maude lifecycle fixture for cancellation acknowledgement, terminal evidence recovery, required artifact-capture failure, and duplicate-terminal safety. |
| 2026-06-02 | Added provider scheduling/capacity readiness checks and tightened workspace policy validation before native provider launch. |
| 2026-06-02 | Added expression provider-routing readiness checks covering parser AgentRef diagnostics, CLI provider matrices, and generated Maude expression searches. |
| 2026-06-02 | Added control-plane driver readiness checks for `run`, `step`, `worker`, and `dev` fixture execution paths. |
| 2026-06-02 | Added a source workflow -> `native-fixture` control-plane E2E proving `whip dev` can drive a durable agent effect into native lifecycle observations and artifact metadata without using command-wrapper-only assertions. |
| 2026-06-02 | Added `RuntimeKernel::run_native_agent_turn`, which starts durable runs from native adapters, records lifecycle observations and artifact refs, and completes/fails/times out/cancels the effect from the native terminal event; the `native-fixture` CLI path now uses this bridge directly. |
| 2026-06-02 | Wired `whip worker/dev --provider codex` to Codex app-server through the native bridge for read-only source workflow turns, fixed Codex `sessionStartSource` to the validated `startup` value, and added a live source workflow -> Codex native smoke. |
| 2026-06-02 | Wired `whip worker/dev --provider claude` to the Claude Agent SDK sidecar through the native bridge for read-only source workflow turns and added a live source workflow -> Claude native smoke using local Claude auth without requiring an API key. |
| 2026-06-02 | Added `whip recover <instance>` and extended Codex and Claude native workflow smokes with store-reopen replay/recovery checks that assert no duplicate terminal recovery after live provider completion. |
| 2026-06-02 | Fixed native CLI turn requests to propagate compiled effect `required_capabilities`, added durable native boundary failure handling for adapter policy denials, and gated a source workflow denial regression in `scripts/check-native-provider-policy-denials.sh`. |
| 2026-06-02 | Extended the source workflow native policy-denial regression to Codex and Claude. The same read-only bridge plus source-required `repo.write` effect now proves both native adapters fail durably with `whip.native.boundary_error.workspace_denied`. |
| 2026-06-02 | Added Codex app-server request-response handling for provider-emitted approval/tool requests: approvals are declined by default, user-input/tool calls receive empty/failed responses, unknown requests receive a JSON-RPC error, and the request is still recorded as redacted `agent.turn.tool_requested` evidence. Live Codex CLI `0.128.0` probes showed file edits currently emit `item/fileChange/outputDelta`, `turn/diff/updated`, and `fileChange` item notifications without server-side approval requests under `workspace-write`/`on-request`. |
| 2026-06-02 | Added Codex app-server remote start-error coverage proving JSON-RPC errors are mapped to provider-specific recoverable native boundary failures and durable redacted error shapes do not include raw remote messages or secret-looking tokens. |
| 2026-06-02 | Added `scripts/check-codex-app-server-error-smoke.sh`, wired it into real-provider native mode and release readiness, and validated live Codex CLI `0.128.0` returns JSON-RPC error code `-32600` with a redacted message shape for an invalid `turn/start` request. |
| 2026-06-02 | Added Claude Agent SDK remote-error adapter coverage proving sidecar remote failures during start or prompt submission map to provider-specific recoverable native boundary failures without leaking raw provider messages into redacted evidence shapes. |
| 2026-06-02 | Added `scripts/check-claude-agent-sdk-error-smoke.sh`, wired it into real-provider native mode and release readiness, and validated live Claude SDK invalid-executable error shapes with no raw error payload capture. |
| 2026-06-02 | Added operator incident UX readiness checks and a golden incident-bundle fixture for provider doctor posture, status/trace/evidence inspection, artifact metadata, cancellation visibility, diagnostics, and lifecycle summaries. |
| 2026-06-02 | Added a cancellation policy matrix gate covering request idempotency, request-only late completion, native acknowledgement ordering, timeout-after-request, and duplicate/contradictory terminal rejection. |
| 2026-06-02 | Added a store replay conformance gate for cancellation replay, terminal diagnostics, artifact relinking, provider terminal recovery, and restart recovery from persisted evidence. |
| 2026-06-02 | Added the shared native provider adapter contract and gate for turn requests, stream events, cancellation events, artifact refs, and redacted boundary failures. |
| 2026-06-02 | Added durable workspace records with policy/status validation, v1 schema repair, and release-readiness coverage for workspace policy denial paths. |
| 2026-06-02 | Added a native-provider TLA+ lifecycle fixture and included it in the formal-model gate alongside the control-plane lifecycle model. |
| 2026-06-02 | Added a Codex app-server native adapter boundary that starts threads/turns, normalizes notifications into native events, records changed-file artifact refs, and sends non-terminal `turn/interrupt` acknowledgements. |
| 2026-06-02 | Added and ran a live disposable Codex app-server artifact smoke that creates a fixed file, validates `turn/diff/updated` evidence, normalizes absolute diff paths to redacted basenames, and records metadata/hash without raw patch content. |
| 2026-06-02 | Added a Claude Agent SDK sidecar native adapter boundary that starts runs, normalizes sidecar stream/terminal events, records redacted payload evidence, and sends non-terminal cooperative `run/cancel` acknowledgements. |
| 2026-06-02 | Tightened Claude Agent SDK live smoke and interrupt gates so strict live validation requires embedded auth posture before SDK launch and records only redacted auth availability. |
| 2026-06-02 | Added Claude native artifact-ref extraction for explicit provider artifact metadata, including lifecycle normalization for artifact events and redaction regressions proving raw artifact content is not emitted. |
| 2026-06-02 | Added Claude Agent SDK artifact smoke gates, report artifacts, native-validation wiring, and release-readiness coverage for metadata-only artifact capture. |
| 2026-06-02 | Validated live Claude Agent SDK interruption using local Claude auth; observed SDK terminal subtype `error_during_execution` after interrupt and normalized it to `claude.turn.cancelled` with acknowledgement `interrupt`. |
| 2026-06-14 | Made Claude Agent SDK surface validation capability-based rather than registry-version based: local Claude CLI drift is accepted when required flags remain present, the declared/locked TypeScript SDK dependency is the package contract, and npm/pip registry metadata is non-fatal unless strict registry mode is enabled. |
| 2026-06-02 | Validated live Claude artifact fixtures with disposable acknowledgements; they created fixed files in temporary workspaces and reports recorded only metadata and SHA-256 hashes. |

## Validation Commands

```sh
scripts/check-native-provider-surfaces.sh
whip --json doctor --providers
scripts/check-native-provider-endpoint-health.sh
scripts/check-codex-app-server-schema.sh
scripts/check-claude-agent-sdk-surface.sh
scripts/check-claude-agent-sdk-live-smoke.sh
scripts/check-claude-agent-sdk-interrupt-smoke.sh
scripts/check-real-provider-destructive-gate.sh
scripts/check-real-provider-report-redaction.sh
scripts/check-native-provider-contract.sh
scripts/check-codex-native-adapter.sh
scripts/check-claude-native-adapter.sh
scripts/check-workspace-records.sh
scripts/check-native-provider-policy-denials.sh
scripts/check-control-plane-driver.sh
scripts/check-provider-scheduling-capacity.sh
scripts/check-expression-provider-routing.sh
scripts/check-operator-incident-ux.sh
scripts/check-cancellation-policy-matrix.sh
scripts/check-store-replay-conformance.sh
scripts/check-formal-models.sh
cargo test -p whipplescript-kernel provider_secret_seed_never_reaches_durable_records
cargo test -p whipplescript --test control_plane artifacts_command_lists_metadata_without_raw_content
WHIPPLESCRIPT_CODEX_APP_SERVER_LIVE=1 scripts/check-codex-app-server-live-smoke.sh
WHIPPLESCRIPT_CODEX_APP_SERVER_LIVE=1 scripts/check-codex-app-server-interrupt-smoke.sh
bash -n scripts/check-native-provider-surfaces.sh scripts/check-release-readiness.sh
git diff --check
```
