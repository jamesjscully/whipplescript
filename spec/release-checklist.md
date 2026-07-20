# Release Checklist

Status: draft

Before declaring v0 complete:

- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --workspace --all-targets -- -D warnings`
- [x] `cargo test --workspace`
- [x] `scripts/check-report-schemas.sh`
- [x] `scripts/check-formal-models.sh`
- [x] `scripts/check-tla-models.sh`
- [x] `scripts/check-e2e.sh`
- [x] `scripts/check-real-providers.sh` result recorded, or skipped with
      rationale
- [x] `whip doctor` output reviewed
- [x] checked examples compile
- [x] generated model search passes for examples with dependencies
- [x] e2e trace conformance passes
- [x] companion skill reviewed
- [x] quickstart and operator guide reviewed from a clean checkout
- [x] final audit gaps classified as blocking, deferred, or fixed
- [x] deferred v0 gaps have owner, rationale, and follow-up location
- [x] obsolete transition notes removed from the release docs
- [x] `dist plan` reviewed for target platforms, installers, checksums, and
      release body
- [x] local Linux x64 `dist build` archive smoke-tested with packaged `whip`
- [x] generated release workflow includes the packaged binary smoke job
- [x] generated release workflow includes a manual non-publishing artifact
      build dry run
- [x] GitHub release artifacts built by CI for macOS, Windows, and Linux

Known v0 deferrals must include owner, rationale, and follow-up location.

## v0.1.0 Cut Runbook

The one and only public release. Decision (Jack, 2026-07-16): the earlier
v0.2/v0.3/v0.4 tags were ceremony to establish a cadence; with no users, they
are collapsed into a single feature-complete, ready-to-use **v0.1.0**. Campaign
phases + open items live in [`v0.1-release-tracker.md`](v0.1-release-tracker.md);
this is the mechanical cut.

1. **Version.** Workspace + crates staged at `0.1.0` (done 2026-07-16); `whip
   --version` → `whipplescript 0.1.0`. The prior 0.2/0.3/0.4 CHANGELOG sections
   are collapsed into a single `[0.1.0]` entry.
2. **Feature completeness + reviews.** Tracker Phases 2–4 done: every built
   capability wired/surfaced/documented, code + security review passes applied,
   docs + companion skill validated from a clean checkout.
3. **Final green.** Working tree clean; full readiness gate
   (`WHIPPLESCRIPT_RELEASE_READINESS_FULL=1`) green; the pre-cut checklist above
   re-verified for this cut.
4. **Tag → GitHub Release (CI-built artifacts).** A tag push is the only path
   that builds the platform archives, installers, and checksums:
   `git tag v0.1.0 && git push origin v0.1.0`. Watch
   `.github/workflows/release.yml` for all platform archives + the packaged-binary
   smoke job. (Jack pushes.)
5. **Post-publish smoke.** From the tagged assets:
   `cargo install --git https://github.com/jamesjscully/whipplescript --tag v0.1.0 --locked -p whipplescript`,
   then `whip --version` (0.1.0) · `whip doctor --json` · a quickstart example.
6. **Follow-ons (not cut-blocking):** crates.io publish (dependency-order
   `core→parser→store→kernel→whipplescript`), Homebrew formula from the tagged
   assets, and retiring the old `v0.2.0` git tags.

## v0.1.1 Patch Cut Runbook

Scope (Jack, 2026-07-20): the openai-generic provider reachability fixes +
spend/cache-economics correctness (G2). Same mechanics as the v0.1.0 cut,
abbreviated:

1. **Version.** Workspace + path-dep pins `0.1.0` → `0.1.1`; `Cargo.lock`
   regenerated; `whip --version` → `whipplescript 0.1.1`; CHANGELOG `[0.1.1]`
   dated (and `[0.1.0]` retro-dated 2026-07-17).
2. **Final green.** Working tree clean; full readiness gate
   (`WHIPPLESCRIPT_RELEASE_READINESS_FULL=1`) green.
3. **Tag → Release.** `git tag -a v0.1.1 && git push origin v0.1.1 && git push
   mirror v0.1.1`. ⚠ Until the GitHub account billing block is cleared, the
   mirror CI builds artifacts but its `gh release create` step 403s — publish
   manually from the CI artifacts with an owner-scoped `gh` token (the v0.1.0
   procedure).
4. **Post-publish smoke.** `cargo install --git … --tag v0.1.1 --locked -p
   whipplescript`; `whip --version` → 0.1.1.

## Native Provider Release Gate

For a release that advertises usable native Codex and Claude providers,
the following checks are ship-blocking:

- [x] Native surface probes pass:
      `scripts/check-native-provider-surfaces.sh`
- [x] Codex app-server schema pin passes:
      `scripts/check-codex-app-server-schema.sh`
- [x] Codex live app-server turn and interrupt smokes pass against a disposable
      read-only workspace:
      `WHIPPLESCRIPT_CODEX_APP_SERVER_LIVE=1 scripts/check-codex-app-server-live-smoke.sh`
      and
      `WHIPPLESCRIPT_CODEX_APP_SERVER_LIVE=1 scripts/check-codex-app-server-interrupt-smoke.sh`
- [x] Codex live app-server diff artifact smoke passes against a disposable
      workspace:
      `WHIPPLESCRIPT_CODEX_APP_SERVER_ARTIFACT_LIVE=1 WHIPPLESCRIPT_CODEX_DISPOSABLE_TARGET=codex-artifact-fixture WHIPPLESCRIPT_CODEX_DISPOSABLE_ACK=I_UNDERSTAND_THIS_PROVIDER_TARGET_IS_DISPOSABLE scripts/check-codex-app-server-artifact-smoke.sh`
- [x] Codex live app-server error smoke passes and records a redacted JSON-RPC
      error response shape:
      `WHIPPLESCRIPT_CODEX_APP_SERVER_ERROR_LIVE=1 scripts/check-codex-app-server-error-smoke.sh`
- [x] Codex live source workflow smoke passes through the native bridge:
      `WHIPPLESCRIPT_CODEX_NATIVE_WORKFLOW_LIVE=1 scripts/check-codex-native-workflow-smoke.sh`
- [x] Claude Agent SDK surface and sidecar smokes pass, including live
      cancellation before native cancellation is advertised:
      `scripts/check-claude-agent-sdk-surface.sh`,
      `WHIPPLESCRIPT_CLAUDE_AGENT_SDK_LIVE=1 scripts/check-claude-agent-sdk-live-smoke.sh`,
      and
      `WHIPPLESCRIPT_CLAUDE_AGENT_SDK_LIVE=1 scripts/check-claude-agent-sdk-interrupt-smoke.sh`
- [x] Claude live source workflow smoke passes through the native bridge using
      local Claude auth or embedded API/provider auth:
      `WHIPPLESCRIPT_CLAUDE_NATIVE_WORKFLOW_LIVE=1 scripts/check-claude-native-workflow-smoke.sh`
- [x] Claude live artifact smoke passes against a disposable workspace using
      local Claude auth or embedded API/provider auth:
      `WHIPPLESCRIPT_CLAUDE_AGENT_SDK_ARTIFACT_LIVE=1 WHIPPLESCRIPT_CLAUDE_DISPOSABLE_TARGET=claude-artifact-fixture WHIPPLESCRIPT_CLAUDE_DISPOSABLE_ACK=I_UNDERSTAND_THIS_PROVIDER_TARGET_IS_DISPOSABLE scripts/check-claude-agent-sdk-artifact-smoke.sh`
- [x] Native provider config validation passes for configs containing
      `codex-main` and `claude-main`:
      `WHIPPLESCRIPT_NATIVE_PROVIDER_CONFIGS=examples/provider-configs/native/native.example.json scripts/check-native-provider-configs.sh`
- [x] Strict real-provider validation passes and emits per-provider reports:
      `WHIPPLESCRIPT_E2E_REAL_PROVIDERS=1 WHIPPLESCRIPT_REAL_PROVIDER_NATIVE_STRICT=1 WHIPPLESCRIPT_NATIVE_PROVIDER_CONFIGS=examples/provider-configs/native/native.example.json scripts/check-real-providers-report.sh`
- [x] Destructive provider suites, if enabled, use disposable target markers
      and the exact acknowledgement
      `I_UNDERSTAND_THIS_PROVIDER_TARGET_IS_DISPOSABLE`.
- [x] The `Native Provider Validation` GitHub Actions workflow has run with
      `strict=true`, uploaded `native-provider-*-reports`, and failed no matrix
      or strict gate job.
      Run `26823071958` passed on 2026-06-02 at commit `597a65c`.

Required native-provider release artifacts:

```text
target/native-provider-surface.jsonl
target/native-provider-endpoint-health.json
target/codex-app-server-schema-report.json
target/codex-app-server-live-smoke.json
target/codex-app-server-interrupt-smoke.json
target/codex-app-server-artifact-smoke.json
target/codex-app-server-error-smoke.json
target/codex-native-workflow-smoke.json
target/claude-agent-sdk-surface.json
target/claude-agent-sdk-live-smoke.json
target/claude-agent-sdk-interrupt-smoke.json
target/claude-agent-sdk-artifact-smoke.json
target/claude-agent-sdk-error-smoke.json
target/claude-native-workflow-smoke.json
target/native-provider-config-validation.json
target/real-provider-smoke-report.md
target/real-provider-preflight.jsonl
target/real-provider-reports/
target/distrib/native-provider-config-examples.tar.gz
```

Command-wrapper-only coerce/Codex checks do not satisfy this gate.

Native provider gate result for v0.1: passed locally on 2026-06-02.
Codex live app-server, interrupt, diff-artifact, error-shape, and source
workflow bridge smokes passed. Claude Agent SDK live, interrupt, artifact,
error-shape, and source workflow bridge smokes passed. Strict
native-provider config/report validation passed with
`examples/provider-configs/native/native.example.json`. The live Codex source
workflow smoke exposed and then validated a fix for duplicate native lifecycle
facts when one provider run emits repeated lifecycle events of the same kind.

## Latest Local Verification

The full non-real-provider verification suite was rerun after final-audit
updates:

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
scripts/check-report-schemas.sh
scripts/check-artifact-admission-differential.sh
scripts/check-formal-models.sh
scripts/check-tla-models.sh
scripts/check-e2e.sh
```

Result: passed.

The report-schema gate includes native and schema validation for verified
artifact bundles, including graph-only `--emit construct-graph` round trips and
rejection when a graph-only bundle is later requested as `lowered-ir` or full
`artifacts`.
The artifact-admission differential mutates full check reports, compile
reports, graph-only verified bundles, lowered-IR verified bundles, and full
verified artifact bundles so Python and native admission stay aligned across
the partial and full artifact surfaces. It covers verified-bundle envelope and
entry closure, emit-specific `construct_graph`/`lowered_ir_report` presence,
supported event/projection/diagnostic lowered-IR handoff entrypoints with
missing or stale explicit refs, rehashed locked-package schema fragments, rehashed
construct/input-field coverage, rehashed package construct vocabulary, and
rehashed package construct/effect capability references, semantic package
construct keyword duplicates, and shared report/package-contract fields such as
source and IR hashes, snapshots, contract-registry diagnostics,
package-contract digests, and platform catalogs. It also includes
schema-valid semantic duplicate IDs for graph nodes, ports, edge refs,
dependency refs, and lowered core objects.
The report-schema gate additionally includes schema-valid stale construct graph
fixtures for catalog-required lowering interfaces whose `node.interfaces`
evidence has been rewritten to match the tampered graph, and duplicate platform
catalog vocabulary plus duplicate verifier-catalog lowering IDs so standalone
bridge runs cannot trust an ambiguous catalog path. It also includes a
model-search fixture matrix where the ledger and IR-obligation artifact agree on
stale guard, terminal-branch, dependency, assertion, and revision source spans,
proving validators compare graph-backed IR spans against their unique
construct-graph anchors. A native-only coordinated-tamper fixture also rewrites
the guard ledger row, the embedded IR-obligation artifact, and the compiler-owned
guard source anchor to the same stale span; standalone artifact validation
accepts that self-consistent report, while `whip verify-report` rejects it
through readable-source artifact re-emission. The same source-backed path is
exercised for package-locked reports with a dependency-span fixture that
rewrites the ledger row, IR-obligation artifact, and construct-graph dependency
span together. Lowered-IR re-emission is covered by a schema/admission-valid
fixture that changes only `lowered_ir_report.lowerer_version`, which ordinary
artifact admission permits but native source-backed verification rejects.

Additional release checks:

```text
cargo run -q -p whipplescript -- doctor
cargo run -q -p whipplescript -- check --model-search examples/*.whip
scripts/check-native-provider-surfaces.sh
scripts/check-codex-app-server-schema.sh
scripts/check-claude-agent-sdk-surface.sh
scripts/check-claude-agent-sdk-live-smoke.sh
scripts/check-claude-agent-sdk-interrupt-smoke.sh
WHIPPLESCRIPT_CODEX_APP_SERVER_LIVE=1 scripts/check-codex-app-server-live-smoke.sh
WHIPPLESCRIPT_CODEX_APP_SERVER_LIVE=1 scripts/check-codex-app-server-interrupt-smoke.sh
WHIPPLESCRIPT_NATIVE_PROVIDER_CONFIGS=examples/provider-configs/native/native.example.json scripts/check-native-provider-configs.sh
cargo run -q -p whipplescript -- --store state.sqlite --json doctor --provider-config examples/provider-configs/native/native.example.json --record-provider-evidence <instance-id>
scripts/check-real-providers.sh
scripts/check-real-providers-report.sh
scripts/check-release-readiness.sh
dist plan --output-format=json --no-local-paths
dist build --artifacts=local --target=x86_64-unknown-linux-gnu
dist build --artifacts=global
scripts/check-dist-archive.sh target/distrib/whipplescript-x86_64-unknown-linux-gnu.tar.xz
```

Result: `WHIPPLESCRIPT_RELEASE_READINESS_FULL=1
scripts/check-release-readiness.sh` passed on 2026-06-02. Doctor output was
reviewed; checked examples compiled; generated model searches passed for
examples with dependencies; report schemas and emitted check/compile artifact
reports validated; schema/native artifact-admission parity was checked against a
malformed-artifact matrix, including duplicate scalar construct-graph
resolution, reserved construct-graph output vocabulary, construct-graph
lifecycle profile mismatch, stale construct-graph evidence rejected by direct
lowered-IR bridge admission, spoofed construct-graph providers rejected by
direct lowered-IR bridge admission, and stale lowered output vocabulary; the full
workspace tests,
clippy, Maude checks, TLA checks, and e2e suite passed through the readiness
wrapper. (The earlier local OpenAI Coerce bridge script was removed — the
real coerce integration is provider-native structured outputs, a separate
credential-gated build; see `spec/coerce.md`.) Real-provider report generation
was validated against the skipped-provider path. CI runs fast release readiness and uploads
`target/release-readiness-report.md` and `target/real-provider-smoke-report.md`
as the `readiness-reports` artifact.
Native-provider surface validation writes
`target/native-provider-surface.jsonl`; strict external readiness should treat
missing Codex or Claude native surfaces as ship-blocking for the
native-provider release track.
Codex app-server schema validation writes
`target/codex-app-server-schema-report.json`, always requires `initialize`,
`thread/start`, `turn/start`, `turn/started`, `turn/completed`,
`turn/interrupt`, and `turn/diff/updated`, and compares the generated metadata
against `spec/codex-app-server-schema.pin.json`. Exact pin drift is accepted by
default when those adapter methods still exist, because developers may have
different Codex CLI versions installed. Use
`WHIPPLESCRIPT_CODEX_APP_SERVER_SCHEMA_STRICT_PIN=1` when reviewing a pin update
or deliberately checking one exact Codex schema.
Codex app-server live smoke validation writes
`target/codex-app-server-live-smoke.json`; run it with
`WHIPPLESCRIPT_CODEX_APP_SERVER_LIVE=1` only against a disposable read-only
workspace/profile because it starts a real Codex turn.
Codex app-server interrupt smoke validation writes
`target/codex-app-server-interrupt-smoke.json`; it validates `turn/interrupt`
acknowledgement, terminal `interrupted` status, and duplicate terminal
suppression for the native Codex cancellation path.
Codex app-server error smoke validation writes
`target/codex-app-server-error-smoke.json`; by default it runs deterministic
adapter remote-error mapping coverage, and with
`WHIPPLESCRIPT_CODEX_APP_SERVER_ERROR_LIVE=1` it validates that the installed
Codex app-server returns a redacted JSON-RPC error shape for an invalid
`turn/start` request.
Claude Agent SDK surface validation writes
`target/claude-agent-sdk-surface.json`; it records the selected TypeScript
sidecar strategy, local Claude CLI version/required flags, declared and locked
TypeScript SDK dependency, Node/npm posture, and optional TypeScript/Python
Agent SDK registry metadata. Registry metadata is informational by default so
different developer machines and temporary registry outages do not fail the
ordinary gate; use `WHIPPLESCRIPT_CLAUDE_AGENT_SDK_STRICT_REGISTRY=1` when
reviewing package freshness deliberately.
Claude Agent SDK sidecar smoke validation writes
`target/claude-agent-sdk-live-smoke.json`; by default it uses the deterministic
fake sidecar path, and with `WHIPPLESCRIPT_CLAUDE_AGENT_SDK_LIVE=1` it starts a
real read-only Agent SDK turn using local Claude CLI auth or embedded
API/provider auth.
Claude Agent SDK interrupt smoke validation writes
`target/claude-agent-sdk-interrupt-smoke.json`; by default it validates the fake
sidecar cancellation terminal ordering, and live mode must be run before marking
Claude provider-native cancellation shipped.
Claude Agent SDK artifact smoke validation writes
`target/claude-agent-sdk-artifact-smoke.json`; by default it validates
metadata-only artifact capture through the fake sidecar. With
`WHIPPLESCRIPT_CLAUDE_AGENT_SDK_ARTIFACT_LIVE=1`, it requires local Claude auth
or embedded auth plus `WHIPPLESCRIPT_CLAUDE_DISPOSABLE_TARGET` and
`WHIPPLESCRIPT_CLAUDE_DISPOSABLE_ACK=I_UNDERSTAND_THIS_PROVIDER_TARGET_IS_DISPOSABLE`,
then creates a fixed artifact file in a temporary disposable workspace and
records only metadata/hash.
Claude Agent SDK error smoke validation writes
`target/claude-agent-sdk-error-smoke.json`; by default it validates deterministic
adapter remote-error mapping. With `WHIPPLESCRIPT_CLAUDE_AGENT_SDK_ERROR_LIVE=1`,
it validates the live SDK config-error envelope using an intentionally invalid
Claude executable path and records only code/message shape.
Claude native workflow smoke validation writes
`target/claude-native-workflow-smoke.json`; by default it runs deterministic
native bridge coverage, and with `WHIPPLESCRIPT_CLAUDE_NATIVE_WORKFLOW_LIVE=1`
it drives a source `.whip` workflow through `whip dev --provider claude` into
the Claude Agent SDK adapter and records redacted local-auth posture plus
durable native lifecycle events. The smoke reopens the store through
`whip recover <instance>` and records `replayAfterRestart.recoveredCount`,
which must be zero after normal completion.
Native-provider config validation writes
`target/native-provider-config-validation.json`; strict external readiness
requires `WHIPPLESCRIPT_NATIVE_PROVIDER_CONFIGS` to point at configs containing
`codex-main` and `claude-main` native bindings.
`whip doctor --record-provider-evidence <instance-id>` records parsed provider
binding validation results as `provider.validation` evidence without starting
provider runs; use it after creating or selecting a release-validation instance.

The `Release` workflow has a manual `workflow_dispatch` dry-run mode. Dispatch
it with `build_artifacts=true` to build and upload all configured cargo-dist
artifacts as GitHub Actions artifacts without creating a GitHub Release. Tag
pushes remain the only publishing path.

`Release` workflow run `26823079397` passed on 2026-06-02 at commit `597a65c`
with `build_artifacts=true`. It built and uploaded all configured local platform
artifact bundles for Apple Silicon macOS, Intel macOS, ARM64 Linux, x64 Linux,
and x64 Windows, built the global installer/checksum/source artifacts, and
passed the packaged Linux x64 archive smoke job. Earlier dry run `26821656011`
passed at commit `23873be`.

`scripts/check-release-readiness.sh` writes
`target/release-readiness-report.md` by default. Set
`WHIPPLESCRIPT_RELEASE_READINESS_FULL=1` for the full local verification suite and
`WHIPPLESCRIPT_RELEASE_STRICT_EXTERNAL=1` when real-provider
prerequisites should fail the readiness command.

Real-provider readiness now also fails explicitly when provider tools,
`WHIPPLESCRIPT_COERCE_TEST_ENDPOINT`, coerce endpoint reachability, or coerce
smoke-test function metadata are missing. Selected provider smoke runs
can be scoped with `WHIPPLESCRIPT_REAL_PROVIDERS=coerce`.
Provider-destructive tests additionally require an explicit disposable target
marker. Set `WHIPPLESCRIPT_REAL_PROVIDER_DESTRUCTIVE_TESTS=1` or the
provider-specific destructive flag, then set a disposable target marker and an
acknowledgement equal to
`I_UNDERSTAND_THIS_PROVIDER_TARGET_IS_DISPOSABLE`; otherwise
`scripts/check-real-providers.sh` exits before running those tests.
`scripts/check-real-providers-report.sh` also writes per-provider JSON reports
under `target/real-provider-reports/` with redacted environment posture,
evidence refs, check counts, and provider-specific preflight records.
The optional GitHub Actions workflow `Native Provider Validation` runs a
Codex/Claude matrix with
`WHIPPLESCRIPT_REAL_PROVIDER_NATIVE_SURFACE=1` and uploads those reports.
Dispatch it with `strict=true` to enforce the all-provider native gate; missing
native provider config paths or live prerequisites fail that strict job.
