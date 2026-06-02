# Release Checklist

Status: draft

Before declaring v0 complete:

- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --workspace --all-targets -- -D warnings`
- [x] `cargo test --workspace`
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

## Native Provider Release Gate

For a release that advertises usable native Codex, Claude, and Pi providers,
the following checks are ship-blocking:

- [ ] Native surface probes pass:
      `scripts/check-native-provider-surfaces.sh`
- [ ] Codex app-server schema pin passes:
      `scripts/check-codex-app-server-schema.sh`
- [ ] Codex live app-server turn and interrupt smokes pass against a disposable
      read-only workspace:
      `WHIPPLESCRIPT_CODEX_APP_SERVER_LIVE=1 scripts/check-codex-app-server-live-smoke.sh`
      and
      `WHIPPLESCRIPT_CODEX_APP_SERVER_LIVE=1 scripts/check-codex-app-server-interrupt-smoke.sh`
- [ ] Codex live app-server diff artifact smoke passes against a disposable
      workspace:
      `WHIPPLESCRIPT_CODEX_APP_SERVER_ARTIFACT_LIVE=1 WHIPPLESCRIPT_CODEX_DISPOSABLE_TARGET=codex-artifact-fixture WHIPPLESCRIPT_CODEX_DISPOSABLE_ACK=I_UNDERSTAND_THIS_PROVIDER_TARGET_IS_DISPOSABLE scripts/check-codex-app-server-artifact-smoke.sh`
- [ ] Codex live app-server error smoke passes and records a redacted JSON-RPC
      error response shape:
      `WHIPPLESCRIPT_CODEX_APP_SERVER_ERROR_LIVE=1 scripts/check-codex-app-server-error-smoke.sh`
- [ ] Codex live source workflow smoke passes through the native bridge:
      `WHIPPLESCRIPT_CODEX_NATIVE_WORKFLOW_LIVE=1 scripts/check-codex-native-workflow-smoke.sh`
- [ ] Claude Agent SDK surface and sidecar smokes pass, including live
      cancellation before native cancellation is advertised:
      `scripts/check-claude-agent-sdk-surface.sh`,
      `WHIPPLESCRIPT_CLAUDE_AGENT_SDK_LIVE=1 scripts/check-claude-agent-sdk-live-smoke.sh`,
      and
      `WHIPPLESCRIPT_CLAUDE_AGENT_SDK_LIVE=1 scripts/check-claude-agent-sdk-interrupt-smoke.sh`
- [ ] Claude live source workflow smoke passes through the native bridge using
      local Claude auth or embedded API/provider auth:
      `WHIPPLESCRIPT_CLAUDE_NATIVE_WORKFLOW_LIVE=1 scripts/check-claude-native-workflow-smoke.sh`
- [ ] Claude live artifact smoke passes against a disposable workspace using
      local Claude auth or embedded API/provider auth:
      `WHIPPLESCRIPT_CLAUDE_AGENT_SDK_ARTIFACT_LIVE=1 WHIPPLESCRIPT_CLAUDE_DISPOSABLE_TARGET=claude-artifact-fixture WHIPPLESCRIPT_CLAUDE_DISPOSABLE_ACK=I_UNDERSTAND_THIS_PROVIDER_TARGET_IS_DISPOSABLE scripts/check-claude-agent-sdk-artifact-smoke.sh`
- [ ] Pi RPC surface and interrupt smokes pass, including live in-flight abort:
      `scripts/check-pi-rpc-surface.sh` and
      `WHIPPLESCRIPT_PI_RPC_INTERRUPT_LIVE=1 scripts/check-pi-rpc-interrupt-smoke.sh`
- [ ] Pi live source workflow smoke passes through the native bridge:
      `WHIPPLESCRIPT_PI_NATIVE_WORKFLOW_LIVE=1 scripts/check-pi-native-workflow-smoke.sh`
- [ ] Pi live artifact smoke passes against a disposable workspace:
      `WHIPPLESCRIPT_PI_RPC_ARTIFACT_LIVE=1 WHIPPLESCRIPT_PI_DISPOSABLE_TARGET=pi-artifact-fixture WHIPPLESCRIPT_PI_DISPOSABLE_ACK=I_UNDERSTAND_THIS_PROVIDER_TARGET_IS_DISPOSABLE scripts/check-pi-rpc-artifact-smoke.sh`
- [ ] Native provider config validation passes for configs containing
      `codex-main`, `claude-main`, and `pi-main`:
      `WHIPPLESCRIPT_NATIVE_PROVIDER_CONFIGS=examples/provider-configs/native/native.example.json scripts/check-native-provider-configs.sh`
- [ ] Strict real-provider validation passes and emits per-provider reports:
      `WHIPPLESCRIPT_E2E_REAL_PROVIDERS=1 WHIPPLESCRIPT_REAL_PROVIDER_NATIVE_STRICT=1 WHIPPLESCRIPT_NATIVE_PROVIDER_CONFIGS=examples/provider-configs/native/native.example.json scripts/check-real-providers-report.sh`
- [ ] Destructive provider suites, if enabled, use disposable target markers
      and the exact acknowledgement
      `I_UNDERSTAND_THIS_PROVIDER_TARGET_IS_DISPOSABLE`.
- [ ] The `Native Provider Validation` GitHub Actions workflow has run with
      `strict=true`, uploaded `native-provider-*-reports`, and failed no matrix
      or strict gate job.

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
target/pi-rpc-surface.json
target/pi-rpc-interrupt-smoke.json
target/pi-rpc-artifact-smoke.json
target/pi-rpc-error-smoke.json
target/pi-native-workflow-smoke.json
target/native-provider-config-validation.json
target/real-provider-smoke-report.md
target/real-provider-preflight.jsonl
target/real-provider-reports/
target/distrib/native-provider-config-examples.tar.gz
```

Command-wrapper-only Loft/BAML/Codex checks do not satisfy this gate.

## Latest Local Verification

The full non-real-provider verification suite was rerun after final-audit
updates:

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
scripts/check-formal-models.sh
scripts/check-tla-models.sh
scripts/check-e2e.sh
```

Result: passed.

Additional release checks:

```text
cargo run -q -p whipplescript -- doctor
cargo run -q -p whipplescript -- check --model-search examples/*.whip
scripts/check-loft-fixtures.sh
WHIPPLESCRIPT_REQUIRE_LOFT_SUBMODULE_FIXTURES=1 scripts/check-loft-fixtures.sh
scripts/stage-loft-fixtures.sh <temp-loft-repo>
scripts/export-loft-source-patch.sh <temp-loft-repo>
scripts/loft-handoff-report.sh <temp-loft-repo>
scripts/check-loft-source-repo.sh <temp-loft-repo>
scripts/check-loft-submodule-readiness.sh
scripts/check-native-provider-surfaces.sh
scripts/check-codex-app-server-schema.sh
scripts/check-claude-agent-sdk-surface.sh
scripts/check-claude-agent-sdk-live-smoke.sh
scripts/check-claude-agent-sdk-interrupt-smoke.sh
scripts/check-pi-rpc-surface.sh
scripts/check-pi-rpc-interrupt-smoke.sh
WHIPPLESCRIPT_PI_RPC_INTERRUPT_LIVE=1 scripts/check-pi-rpc-interrupt-smoke.sh
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
examples with dependencies; the full workspace tests, clippy, Maude checks, TLA
checks, and e2e suite passed through the readiness wrapper. Loft
compatibility-fixture shape checks passed against the manifest-driven rich
issue, evidence, resource intent, lifecycle, lease, and retryable-error
fixtures; Loft readiness scripts share the fixture manifest contract from
`scripts/loft-fixtures-lib.sh`; Loft source-repo preflight was validated against
a temporary committed fixture repo; Loft source patch export was validated
against a temporary local repo; Loft handoff reporting was validated with
shell-quoted generated commands and is generated by release readiness; strict
Loft submodule fixture mode and Loft submodule readiness both passed against
`vendor/loft`. `scripts/check-openai-coerce.sh` passed against the local
OpenAI-backed Coerce bridge using `OPENAI_API_KEY` from `.env`; real-provider
report generation was validated against both the skipped-provider path and the
OpenAI Coerce smoke path. CI runs fast release readiness and uploads
`target/release-readiness-report.md`, `target/real-provider-smoke-report.md`,
and `target/loft-handoff-report.md` as the `readiness-reports` artifact.
Native-provider surface validation writes
`target/native-provider-surface.jsonl`; strict external readiness should treat
missing Codex, Claude, or Pi native surfaces as ship-blocking for the
native-provider release track.
Codex app-server schema validation writes
`target/codex-app-server-schema-report.json` and compares against
`spec/codex-app-server-schema.pin.json`; update the pin only after reviewing
Codex CLI/schema changes and confirming the required lifecycle methods still
exist.
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
sidecar strategy, local Claude CLI version/flags, Node/npm posture, and current
TypeScript/Python Agent SDK package versions.
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
Pi RPC surface validation writes `target/pi-rpc-surface.json`; it records the
selected RPC subprocess strategy, local Pi CLI version/flags, current Pi SDK
package posture, and an offline RPC `get_state` response with session/model
metadata.
Pi RPC interrupt smoke validation writes `target/pi-rpc-interrupt-smoke.json`;
by default it validates the `abort` command response shape in offline RPC mode.
With `WHIPPLESCRIPT_PI_RPC_INTERRUPT_LIVE=1`, it validates a live in-flight
prompt abort with tools disabled, assistant `stopReason: "aborted"`, exactly one
`turn_end`, and abort acknowledgement. Strict external release readiness runs
the live path.
Pi RPC artifact smoke validation writes `target/pi-rpc-artifact-smoke.json`; by
default it validates deterministic metadata-only artifact extraction. With
`WHIPPLESCRIPT_PI_RPC_ARTIFACT_LIVE=1`, it requires
`WHIPPLESCRIPT_PI_DISPOSABLE_TARGET` and
`WHIPPLESCRIPT_PI_DISPOSABLE_ACK=I_UNDERSTAND_THIS_PROVIDER_TARGET_IS_DISPOSABLE`,
then asks Pi RPC to create a fixed artifact file in a temporary disposable
workspace and records only metadata/hash.
Pi RPC error smoke validation writes `target/pi-rpc-error-smoke.json`; by default
it validates deterministic adapter remote-error mapping. With
`WHIPPLESCRIPT_PI_RPC_ERROR_LIVE=1`, it validates the installed Pi RPC error
response shape using an intentionally invalid command and records only the error
payload shape.
Pi native workflow smoke validation writes `target/pi-native-workflow-smoke.json`;
by default it runs deterministic native bridge coverage, and with
`WHIPPLESCRIPT_PI_NATIVE_WORKFLOW_LIVE=1` it drives a source `.whip` workflow
through `whip dev --provider pi` into the Pi RPC adapter and records durable
native lifecycle events from the RPC stream. The smoke reopens the store through
`whip recover <instance>` and records `replayAfterRestart.recoveredCount`,
which must be zero after normal completion.
Native-provider config validation writes
`target/native-provider-config-validation.json`; strict external readiness
requires `WHIPPLESCRIPT_NATIVE_PROVIDER_CONFIGS` to point at configs containing
`codex-main`, `claude-main`, and `pi-main` native bindings.
`whip doctor --record-provider-evidence <instance-id>` records parsed provider
binding validation results as `provider.validation` evidence without starting
provider runs; use it after creating or selecting a release-validation instance.

The `Release` workflow has a manual `workflow_dispatch` dry-run mode. Dispatch
it with `build_artifacts=true` to build and upload all configured cargo-dist
artifacts as GitHub Actions artifacts without creating a GitHub Release. Tag
pushes remain the only publishing path.

`Release` workflow run `26820271988` passed on 2026-06-02 at commit `f8c8170`
with `build_artifacts=true`. It built and uploaded all configured local platform
artifact bundles for Apple Silicon macOS, Intel macOS, ARM64 Linux, x64 Linux,
and x64 Windows, built the global installer/checksum/source artifacts, and
passed the packaged Linux x64 archive smoke job. A previous dry run,
`26820226807`, failed during checkout because the generated release workflow
attempted to recursively clone the private/local Loft source submodule. Loft is
now public, fresh recursive submodule clones can fetch
`https://github.com/jamesjscully/loft`, and CI plus release checkout steps fetch
submodules so validation keeps exercising the source-of-truth Loft fixture path.

`scripts/check-release-readiness.sh` writes
`target/release-readiness-report.md` by default. Set
`WHIPPLESCRIPT_RELEASE_READINESS_FULL=1` for the full local verification suite and
`WHIPPLESCRIPT_RELEASE_STRICT_EXTERNAL=1` when Loft submodule and real-provider
prerequisites should fail the readiness command.

Real-provider readiness now also fails explicitly when provider tools,
`WHIPPLESCRIPT_LOFT_TEST_ISSUE`, `WHIPPLESCRIPT_BAML_TEST_ENDPOINT`, BAML endpoint
reachability, BAML smoke-test function metadata, tracked Loft spec/fixture
files, or a clean Loft fixture repo are missing. Selected provider smoke runs
can be scoped with `WHIPPLESCRIPT_REAL_PROVIDERS=loft`, `baml`, or `loft,baml`.
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
Codex/Claude/Pi matrix with
`WHIPPLESCRIPT_REAL_PROVIDER_NATIVE_SURFACE=1` and uploads those reports.
Dispatch it with `strict=true` to enforce the all-provider native gate; missing
native provider config paths or live prerequisites fail that strict job.
