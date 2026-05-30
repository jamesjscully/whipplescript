# Real Provider Validation Tracker

Status: active tracker for G-008/G-009.

Date: May 30, 2026.

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

## Still Open

- [ ] Codex must move from command-wrapper smoke coverage to a supported Codex
  App Server or SDK adapter with native thread/session, stream, approval, diff,
  artifact, and auth handling.
- [ ] Claude must move from command-wrapper smoke coverage to the Claude Agent
  SDK with explicit API/provider auth, profile-to-tool mapping, stream handling,
  artifact capture, and usage capture.
- [ ] Pi must use the Pi extension path with durable correlation between
  WhippleScript effect/run ids and Pi conversation threads.
- [ ] Terminal-event append recovery needs explicit tests for store failure after
  provider completion evidence has been gathered.
- [ ] Artifact capture failures need first-class classification after real
  adapter artifact manifests exist.
- [ ] Out-of-band cancellation support remains provider-specific work beyond the
  current pre-launch cancellation check and command timeout handling.

## Validation Commands

```sh
cargo test -p whipplescript-kernel harness::
WHIPPLESCRIPT_REAL_PROVIDER_PREFLIGHT_ONLY=1 \
WHIPPLESCRIPT_E2E_REAL_PROVIDERS=1 \
WHIPPLESCRIPT_REAL_PROVIDERS=codex \
scripts/check-real-providers.sh
scripts/check-real-providers-report.sh
```
