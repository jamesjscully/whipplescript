# Real Provider Validation Tracker

Status: active shipping tracker for a usable native-provider system.

Date: May 31, 2026.

## Closed In This Slice

- [x] Command-backed harness plans can declare adapter identity separately from
  provider identity.
- [x] Command-backed harnesses preflight required environment keys and classify
  missing values as `provider.config.missing` without launching the provider.
- [x] Command-backed harnesses preflight adapter command availability and
  classify missing commands as `adapter.resolve.failed`.
- [x] Command-backed harnesses preflight configured working directories and
  classify missing workspaces as `workspace.prepare.failed`.
- [x] Command-backed harnesses can enforce a bounded provider timeout and return
  `provider.timeout` with `ProviderRunStatus::TimedOut`.
- [x] Command-backed harnesses can validate stdout as JSON for adapters that
  promise structured output and classify invalid output as
  `provider.result.invalid`.
- [x] Real-provider readiness writes a JSONL preflight artifact with provider,
  phase, check, status, and redacted message fields.
- [x] Real-provider smoke reports embed the preflight JSONL artifact alongside
  the command output and set/unset environment posture.
- [x] Kernel provider failure payloads expose provider, adapter, workspace,
  missing config keys, session/thread ids, retry metadata, and raw provider
  details where available.

## Shipping Scope

This tracker is for shipping a usable native-provider WhippleScript system, not a
mock-provider or command-wrapper v0. A shippable system must run real Codex,
Claude, and Pi turns through supported native surfaces, preserve provider
lifecycle evidence, handle cancellation and recovery without duplicate terminal
outcomes, and validate those paths against real or isolated provider fixtures.

Day-to-day implementation work is tracked in
`spec/native-provider-implementation-tracker.md`. This file remains the broader
shipping checklist and gap register.

Progress states:

```text
[ ] not started
[~] in progress
[x] complete
[!] blocked
```

## Native Adapter Checklist

| Area | Status | Owner | Done Criteria | Validation |
| --- | --- | --- | --- | --- |
| Provider surface validation | [x] | Runtime/provider maintainer | External Codex, Claude, and Pi integration surfaces are validated from current local CLIs and official docs before runtime plumbing is built. | `scripts/check-native-provider-surfaces.sh` plus `spec/native-provider-surfaces.md`. |
| Shared native adapter contract | [x] | Runtime/provider maintainer | Adapter trait represents provider capabilities, auth/config refs, workspace policy, request submission, stream events, cancellation, artifact manifests, usage, terminal status, and structured boundary failures without leaking secrets. | `scripts/check-native-provider-contract.sh` covers distinct built-in provider surfaces, binding config redaction, native turn request shape redaction, provider event shape redaction, boundary error redaction, fake adapter start/stream/cancel events, and the kernel bridge from native adapter events into durable runs, lifecycle facts, and artifact rows. Provider-specific implementations remain tracked in the Codex, Claude, and Pi adapter rows. |
| Codex native adapter | [~] | Codex integration maintainer | Uses a supported Codex App Server or SDK surface, not ad hoc command stdout. Records session/thread ids, stream events, approvals/tool calls, diffs, changed files, artifacts, usage, model/profile, and auth/config reference. | `scripts/check-codex-native-adapter.sh` covers deterministic app-server adapter startup, policy payloads, approval-capable server-request responses, remote start-error mapping, stream notification normalization, changed-file artifact refs, and `turn/interrupt` acknowledgement without fabricating terminal cancellation. Live isolated Codex lifecycle, cancellation, diff artifact capture, invalid-turn error response shape, source workflow -> native bridge execution, and replay-after-restart no-op recovery pass; live Codex CLI `0.128.0` file-change probes emitted `item/fileChange/outputDelta`, `turn/diff/updated`, and `fileChange` item notifications rather than server-side approval requests under `workspace-write`/`on-request`. |
| Claude native adapter | [~] | Claude integration maintainer | Uses the Claude Agent SDK with explicit API/provider auth, profile-to-tool mapping, streaming messages/tool events, artifacts, usage, model/profile, and structured errors. | `scripts/check-claude-native-adapter.sh` covers deterministic sidecar adapter startup, Agent SDK tool-policy payloads, stream event normalization, redacted payload evidence, remote start/event error mapping, and cooperative `run/cancel` acknowledgement without fabricating terminal cancellation. Live isolated Claude success, artifact capture, cancellation, config-error shape, source workflow -> native bridge execution, and replay-after-restart no-op recovery pass with local Claude auth; live tool/profile denial smokes remain open. |
| Pi native adapter | [~] | Pi integration maintainer | Uses the Pi RPC/extension session path with durable correlation from WhippleScript effect/run ids to Pi sessions, turns, terminal detection, and evidence export. | `scripts/check-pi-native-adapter.sh` covers deterministic RPC adapter startup, state/session/model capture, `prompt` policy payloads, stream/terminal event normalization, redacted payload evidence, remote prompt error mapping, and native `abort` acknowledgement without fabricating terminal cancellation. Live isolated Pi success, artifact capture, cancellation acknowledgement, RPC error shape, source workflow -> native bridge execution, and replay-after-restart no-op recovery pass; `scripts/check-provider-scheduling-capacity.sh` now includes deterministic restart recovery from Pi RPC-shaped native terminal evidence. |
| Provider binding configuration | [~] | Runtime maintainer | Config file/schema can declare provider kind, adapter surface, credentials ref, profile ids, model/runtime defaults, workspace policy, timeout, cancellation depth, artifact policy, and health checks. | `whip doctor --provider-config` validates native example configs, redacts secret-adjacent values, and reports unsupported policies; `scripts/check-native-provider-endpoint-health.sh` now aggregates provider endpoint/session probes, with broader workspace/model access checks still in provider-specific live suites. |
| Provider lifecycle normalization | [x] | Runtime maintainer | Native provider events normalize into `agent.turn.started`, `agent.turn.streamed`, `agent.turn.tool_requested`, `agent.turn.artifact_captured`, `agent.turn.completed`, `agent.turn.failed`, `agent.turn.timed_out`, and `agent.turn.cancelled` facts/events. | `native_lifecycle` maps Codex, Claude, and Pi events into canonical lifecycle kinds; store/kernel tests record redacted native-event evidence and expose lifecycle through status/runs/trace/evidence JSON. |

## Cancellation Checklist

| Area | Status | Owner | Done Criteria | Validation |
| --- | --- | --- | --- | --- |
| Capability-declared cancellation depth | [x] | Runtime/provider maintainer | Provider bindings declare supported cancellation depth: none, cooperative request, native stop, hard process stop, or remote session cancel. Runtime never asks for a deeper mode than the binding supports. | `scripts/check-native-provider-contract.sh` covers unsupported configured depths, distinct built-in provider capabilities, and runtime cancellation-depth denial when a request exceeds the binding's configured depth. |
| Provider-native cancellation requests | [x] | Provider maintainers | Codex, Claude, and Pi adapters map WhippleScript cancellation requests to native provider stop/abort/session APIs when supported. | Codex/Pi interrupt smokes exist; Claude sidecar uses SDK `Query.interrupt()` before `AbortController` and live local-auth smoke validates that provider `error_during_execution` after interrupt is normalized to one `claude.turn.cancelled` acknowledgement. |
| Cancellation acknowledgement model | [x] | Runtime maintainer | Store represents request pending, acknowledgement received, native stopped, hard-stopped, timed out, and late provider completion states without fabricating duplicate terminal outcomes. | `scripts/check-cancellation-policy-matrix.sh` covers idempotent request recording, request resolution on terminal completion, native acknowledgement ordering, timeout-after-request resolution, duplicate terminal rollback, and contradictory terminal rejection. |
| Late completion handling | [x] | Runtime maintainer | Late provider completion after cancellation request or acknowledgement is classified deterministically as accepted, ignored, or diagnostic-only by policy. | `scripts/check-cancellation-policy-matrix.sh` covers completion after request-only cancellation, rejection of cancellation after terminal completion, timeout before late success, supported-provider cancellation acknowledgement, and duplicate/contradictory late terminal outcomes. |
| Timeout process cleanup | [x] | Runtime maintainer | Command compatibility harness terminates Unix descendant processes on timeout; native adapters must implement equivalent provider/session cleanup. | `cargo test -p whipplescript-kernel harness::` includes descendant cleanup coverage. |

## Artifact And Evidence Checklist

| Area | Status | Owner | Done Criteria | Validation |
| --- | --- | --- | --- | --- |
| Artifact manifest schema | [x] | Runtime/provider maintainer | Provider outputs declare artifact id, kind, path/ref, content hash, MIME type, size, redaction status, source provider event, and retention policy. | `artifact_manifest` schema validation covers entry identity, URI type, content hash, MIME type, redaction status, retention policy, required flag, and replay-stable artifact row relinking. |
| Diff and changed-file capture | [~] | Codex/Claude maintainers | Real coding turns capture diffs, changed files, command/tool outputs, and provider-generated attachments as evidence without relying only on transcript text. | Command-backed providers emit transcript/stdout/stderr ref artifacts, Codex live app-server artifact smoke validates provider-created files with redacted changed-file/diff metadata, and Claude/Pi live artifact fixtures create fixed files in temporary disposable workspaces and record metadata/hash only. Claude's writable fixture uses Agent SDK `bypassPermissions` with explicit skip-permissions and an exact disposable artifact path, and the report includes redacted tool-use counts for completed-without-file diagnostics. Live provider-generated attachment fixtures beyond fixed file/diff creation remain open. |
| Artifact capture failure classification | [x] | Runtime maintainer | Capture failures classify as `artifact.capture.failed` with provider, adapter, artifact ref, error kind, recoverability, and transcript/stderr refs. Terminal result is not marked successful when required artifact capture fails. | Kernel tests cover known failure kinds, redacted failure payloads, linked diagnostics, and required artifact capture failure forcing a failed terminal outcome. |
| Transcript and stream evidence | [~] | Runtime/provider maintainer | Provider transcript, streaming chunks, tool calls, approvals, diagnostics, stdout/stderr compatibility logs, and native error payloads are stored as bounded evidence refs. | Native lifecycle, Codex, Claude, and Pi evidence summaries preserve redacted payload shapes and provider/session ids; Codex approval-capable app-server requests are declined/responded to while being recorded as redacted `agent.turn.tool_requested` events; full real-provider transcript export fixtures remain open. |
| Redaction and retention | [~] | Security/runtime maintainer | Secrets, tokens, prompt-injected credentials, and provider raw payloads are redacted or reference-only according to policy before persistence. | Kernel secret-seeded fixture now fails if a raw `sk-...` token reaches stored facts, events, terminal metadata, diagnostics, provider evidence, artifact rows, or artifact manifest metadata; `whip artifacts --json <run-id>` lists metadata only and redacts legacy/raw artifact path and hash values; `scripts/check-real-provider-report-redaction.sh` verifies preflight JSONL, Markdown, per-provider JSON, stdout/stderr, and filenames are redacted, and release readiness runs it as a required local check. Raw artifact-content export is not implemented yet and will need explicit redaction fixtures when added. |

## Recovery Checklist

| Area | Status | Owner | Done Criteria | Validation |
| --- | --- | --- | --- | --- |
| Terminal-event append recovery | [x] | Store/runtime maintainer | If a provider returns after the run starts but the store fails before appending terminal event, recovery can reconcile evidence and append exactly one terminal outcome or an explicit recovery diagnostic. | Fault-injection tests cover failure after command-backed provider evidence, native-provider terminal evidence, artifact capture, and terminal append; recovery appends exactly one terminal outcome. |
| Idempotent provider completion | [x] | Runtime maintainer | Replayed or retried provider completion uses idempotency keys bound to instance, effect, run, provider session, and terminal payload. | Provider terminal metadata includes provider correlation and terminal payload hash; duplicate and contradictory terminal completions roll back without appending a second terminal event. |
| Worker restart recovery | [x] | Runtime maintainer | Worker restart can find running effects, provider sessions, cancellation requests, artifacts, and leases, then either resume, reconcile, timeout, or emit recovery diagnostics. | `recover_running_provider_runs` scans running runs after store reopen and recovers those with persisted provider evidence or terminal native-provider evidence; restart regressions prove exactly one terminal event, including a Pi RPC-shaped native session. `whip recover <instance>` exposes the path, and live Codex/Claude/Pi workflow smokes assert completed native runs replay after restart with zero recovered duplicate terminals. |
| Store/replay conformance | [x] | Store/runtime maintainer | Replay reconstructs native lifecycle, cancellation, artifacts, diagnostics, and recovery decisions from append-only events. | `scripts/check-store-replay-conformance.sh` covers active-revision cancellation replay, terminal runs and resolved cancel requests, expired leases with cancel-requested effects, terminal diagnostics replay, artifact relinking, provider terminal recovery, artifact-before-terminal recovery, and worker-restart recovery from persisted provider evidence. Native workflow smokes include a store-reopen `whip recover` check for live Codex, Claude, and Pi runs. |

## Real-Provider Validation Checklist

| Area | Status | Owner | Done Criteria | Validation |
| --- | --- | --- | --- | --- |
| Isolated provider fixtures | [~] | Integration maintainer | Real-provider tests use isolated workspaces/accounts/projects so destructive actions cannot affect maintainer machines or production repos. | Native/destructive provider suites require explicit disposable target markers and acknowledgements before launch; `scripts/check-real-provider-destructive-gate.sh` validates skip, missing-marker failure, and acknowledged-marker pass behavior without contacting providers. Codex, Claude, and Pi live fixed-artifact fixtures now use temporary disposable workspaces and remove them after validation; provider-specific account/project fixtures for broader destructive workflows remain open. |
| Beyond command-wrapper smoke | [x] | Integration maintainer | `scripts/check-real-providers.sh` can select native Codex, Claude, and Pi adapter suites and fails when only compatibility command wrappers are configured for shipping mode. | `WHIPPLESCRIPT_REAL_PROVIDER_NATIVE_STRICT=1` defaults to Codex/Claude/Pi, rejects Loft/BAML command-wrapper selections, requires native provider configs, and runs native surface probes. |
| Real-provider CI matrix | [x] | Release maintainer | CI supports opt-in native provider jobs with clear secret requirements, skip rationale, artifact upload, and redacted reports. | `Native Provider Validation` workflow-dispatch runs Codex/Claude/Pi native-surface matrix jobs, uploads per-provider reports, and includes a strict native gate that fails when required configs or live prerequisites are absent. |
| Provider health checks | [~] | Runtime/provider maintainer | Health checks validate credentials, endpoint/session availability, required extension/server versions, model access, workspace access, and cancellation/artifact support. | `whip doctor --providers --json` reports deterministic non-live CLI and credential-reference posture without exposing secret values; `scripts/check-native-provider-endpoint-health.sh` now emits one redacted readiness artifact covering Codex app-server turn notifications, Claude Agent SDK session/terminal events, Pi RPC state, and Pi abort ordering, with live mode tied to strict external readiness. Broader workspace/model access checks remain in provider-specific live suites. |
| Destructive e2e gates | [x] | Integration maintainer | Tests that mutate external systems require explicit fixture identity, confirmation, cleanup, and audit evidence. | `scripts/check-real-providers.sh` refuses destructive provider tests without disposable target markers and the exact acknowledgement string; `scripts/check-real-provider-destructive-gate.sh` covers skip, fail, pass, and target-value redaction, and release readiness runs it as a required local check. |
| Release readiness gate | [~] | Release maintainer | `scripts/check-release-readiness.sh` has a strict native-provider mode that requires native adapters, real-provider validation reports, redaction checks, artifact checks, cancellation checks, and recovery checks. | Release checklist references strict native-provider readiness as a ship gate; report redaction, destructive fixture gates, provider doctor posture, metadata-only artifact redaction, and Codex/Claude/Pi native artifact smokes are required local readiness checks. Strict live-provider readiness remains open. |

## Other Usable-System Gaps

| Area | Status | Owner | Done Criteria | Validation |
| --- | --- | --- | --- | --- |
| Control-plane driver | [x] | Runtime maintainer | `whip run`, `whip step`, `whip worker`, and `whip dev` share a durable rule/effect driver that materializes ready rules into facts/effects before provider run start. | `scripts/check-control-plane-driver.sh` runs deterministic coverage for source workflow startup, `step` fact/effect materialization, `worker` execution after durable effects, `dev` provider-matrix fixture routing, and source workflow -> `native-fixture` execution through `RuntimeKernel::run_native_agent_turn`. `WHIPPLESCRIPT_CODEX_NATIVE_WORKFLOW_LIVE=1 scripts/check-codex-native-workflow-smoke.sh`, `WHIPPLESCRIPT_CLAUDE_NATIVE_WORKFLOW_LIVE=1 scripts/check-claude-native-workflow-smoke.sh`, and `WHIPPLESCRIPT_PI_NATIVE_WORKFLOW_LIVE=1 scripts/check-pi-native-workflow-smoke.sh` validate source workflow -> real Codex/Claude/Pi native bridges. |
| Workspace records and policy | [~] | Runtime maintainer | Workspaces are first-class records with shared checkout, per-effect worktree, per-issue worktree, and remote sandbox policies; providers cannot escape policy. | `scripts/check-workspace-records.sh` covers durable workspace records, accepted policy/status validation, invalid policy/status rejection, v1 store schema repair, provider config rejection for unknown workspace policies, and Codex/Claude/Pi policy denial for unsupported/remote writable modes. Native smoke in each workspace mode remains open. |
| Provider scheduling and capacity | [~] | Runtime maintainer | Scheduler honors provider/profile capacity, health, lease expiry, retry/backoff, cancellation, and blocked policy states. | `scripts/check-provider-scheduling-capacity.sh` runs deterministic coverage for source-declared agent capacity blocks, lease expiry/stale completion/retry, native-provider launch health failure becoming a durable boundary failure instead of a queued effect, multi-effect native fixture stress, Pi native terminal-evidence restart recovery, and idempotent native cancellation acknowledgement. Provider-specific live workspace/model stress remains open. |
| Operator status and trace UX | [x] | CLI/runtime maintainer | `status`, `trace`, diagnostics, and reports explain provider config, health, lifecycle, artifacts, cancellation, recovery, and blocked states in human and JSON forms. | `scripts/check-operator-incident-ux.sh` runs focused coverage for provider health posture, `status`/`trace`/`evidence` inspection, `runs`/`trace` artifact counts, redacted artifact metadata, cancellation request visibility, native lifecycle summaries, and a golden operator incident bundle spanning status, runs, diagnostics, trace conformance, and artifact metadata. |
| Security and approval boundaries | [~] | Security/runtime maintainer | Profile/tool approvals, filesystem/network boundaries, secret refs, prompt/tool output redaction, and provider-specific approvals are enforced before native actions. | Codex, Claude, and Pi native policy builders reject profile mismatch, missing approval, forbidden capability, and forbidden workspace before launch; `scripts/check-native-provider-policy-denials.sh` runs those denial tests plus a source workflow that requests `repo.write` and is durably failed by the native adapter boundary from compiled `required_capabilities` for Codex, Claude, and Pi; Codex app-server approval-capable request handling is covered deterministically with conservative decline responses. Live probes showed Claude unavailable-write prompts currently end as `error_max_turns` and Pi no-tools write prompts complete normally, so there is no stable provider-emitted denial event shape to gate yet. |
| Expression-kernel completeness for routing | [x] | Language maintainer | Guard/assertion expression support is complete enough for provider matrices: boolean logic, ordering, membership, exists/empty, optional presence, enum/literal domains, typed dynamic agent refs. | `scripts/check-expression-provider-routing.sh` runs parser AgentRef/provider-matrix diagnostics, CLI provider-routing expression dev coverage, and generated Maude expression-model checks. |
| Formal lifecycle models | [x] | Formal/runtime maintainer | Maude/TLA+ cover native provider lifecycle, cancellation acknowledgement, artifact capture failure, terminal append recovery, and replay invariants. | `scripts/check-formal-models.sh` runs Maude native lifecycle searches plus Apalache checks for `ControlPlaneLifecycle.tla` and `NativeProviderLifecycle.tla`; the native fixture covers cancellation acknowledgement, provider terminal evidence recovery, required artifact-capture failure, and duplicate terminal outcome prevention. |
| Documentation and setup | [x] | Docs/release maintainer | Install, provider configuration, credentials, health checks, workspace policy, cancellation, artifacts, troubleshooting, and recovery docs are accurate for native providers. | Native provider docs include strict prerequisites, setup expectations, validation commands, troubleshooting, report locations, and destructive fixture gates. |
| Packaging and release artifacts | [x] | Release maintainer | macOS, Linux, and Windows artifacts include provider config examples, doctor checks, release notes, checksums, and smoke-tested packaged binary behavior. | Release workflow uploads `native-provider-config-examples.tar.gz`; archive smoke runs packaged `whip doctor --provider-config` against the example config. |

## Validation Commands

```sh
cargo test -p whipplescript-kernel harness::
scripts/check-native-provider-surfaces.sh
WHIPPLESCRIPT_REAL_PROVIDER_PREFLIGHT_ONLY=1 \
WHIPPLESCRIPT_E2E_REAL_PROVIDERS=1 \
WHIPPLESCRIPT_REAL_PROVIDERS=codex \
scripts/check-real-providers.sh
scripts/check-real-providers-report.sh
scripts/check-native-provider-contract.sh
scripts/check-workspace-records.sh
scripts/check-operator-incident-ux.sh
scripts/check-cancellation-policy-matrix.sh
scripts/check-store-replay-conformance.sh
scripts/check-release-readiness.sh
```
