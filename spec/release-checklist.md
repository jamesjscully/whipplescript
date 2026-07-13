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

## v0.2.0 Cut Runbook

The 0.2 baseline is verified green (full readiness gate, 0 required failures) and
the version is staged at 0.2.0. Below is the ordered sequence to cut and publish.
Every step requires a maintainer decision or maintainer credentials and is not
automatable from inside the build.

1. **Std-package renames — DONE.** S3 `queue.*`→`tracker.*` (e57be7d), S2
   `coerce`→`schema.coerce` (bd940e4), and the S2b model-id effect-key fold
   (8abc7aa) are built, runtime-verified, and gate-green. DR-0014 can move
   `proposed`→`accepted`. No scope decision remains here.

2. **Live provider validation (G-008) — Codex + Claude.** Endpoint-health is
   codex+claude-only.
   With Codex logged in (`codex`) and Claude authed (`claude` — claude.ai login or
   `ANTHROPIC_API_KEY`), and a provider config for config-validation:
   `WHIPPLESCRIPT_RELEASE_STRICT_EXTERNAL=1 WHIPPLESCRIPT_PROVIDER_CONFIGS=examples/provider-configs/native/native.example.json scripts/check-release-readiness.sh`
   Codex + Claude were validated live 2026-07-05 (app-server + Agent SDK + both
   native source-workflow smokes; surface probe uses grep, not rg). Required to
   advertise production native support.

3. **Final green + version.** Working tree clean; `whip --version` →
   `whipplescript 0.2.0`; `dist plan` reports 0.2.0; set the CHANGELOG date.

4. **Tag → GitHub Release (CI-built artifacts).** A tag push is the only path that
   creates a GitHub Release; the cargo-dist workflow builds/uploads the five
   platform archives, shell/PowerShell installers, and checksums:
   `git tag v0.2.0 && git push origin v0.2.0`
   Watch `.github/workflows/release.yml`: confirm all platform archives + the
   packaged-binary smoke job pass and the release body is correct.

5. **Publish to crates.io in dependency order** (each must be live before the next
   resolves; path deps already pin `version = "0.2.0"`):
   `cargo publish -p whipplescript-core`
   `cargo publish -p whipplescript-parser`
   `cargo publish -p whipplescript-store`
   `cargo publish -p whipplescript-kernel`
   `cargo publish -p whipplescript`
   (`whipplescript-host-do` is the Durable-Object host on the 0.3 track — not
   published for 0.2.)

6. **Homebrew formula.** From the tagged release assets take the macOS/Linux
   tarball URLs + SHA256s and update `Formula/` in `jamesjscully/homebrew-tap`;
   smoke with `brew install jamesjscully/tap/whip`.

7. **Post-publish smoke.**
   `cargo install --git https://github.com/jamesjscully/whipplescript --tag v0.2.0 --locked -p whipplescript`
   then `whip --version` (0.2.0) · `whip doctor --json` ·
   `whip check examples/minimal-noop.whip`.

Deferred past 0.2 (not blockers): macOS Developer ID signing + notarization,
Windows Authenticode, WinGet/Scoop/Chocolatey, release provenance attestations.

## v0.3.0 Cut Runbook

0.3 = cloud (Durable Object runtime) + owned harness. Scope decisions (Jack,
2026-07-09): **publish via the public mirror + crates.io**; **skip Homebrew**;
ship **containers-disabled** (the Class-A/B compute plane is built + locally
live-proven — enabling it in production is a follow-on once Cloudflare Containers
billing + image push are done, and is tracked as v0.4 in the DO runtime tracker).
Web search/fetch tools and the deferred DO `[~]` residuals were moved to v0.4.

1. **Version + gate.** Workspace bumped `0.2.0 → 0.3.0` (all crates + path-dep
   pins + `Cargo.lock`); IR goldens unchanged. Full readiness gate
   (`WHIPPLESCRIPT_RELEASE_READINESS_FULL=1`) green — note the toolchain bump to
   rustc/clippy 1.95.0 newly flagged one pre-existing `let-else` under
   `-D warnings` (fixed, 8f80248). Set the CHANGELOG date.

2. **Tag → `main` → GitHub Release.** As with 0.2, a tag push is the only path
   that builds the platform archives. If `-src` GitHub Actions billing is still
   broken, publish via the **public mirror** (free CI) exactly as 0.2 did — and
   the mirror publish **MUST preserve the grafted infra files** (the 5
   release-infra files + the ~9 curated-tree files enumerated in the release-plan
   memory), or mirror CI/releases break.

3. **crates.io — DEFERRED to 0.3.1 (Jack, 2026-07-09).** `whipplescript-core
   0.3.0` published successfully; the other four are blocked because the crates
   are **not self-contained for a published tarball**: `whipplescript-parser`'s
   `build.rs` reads `../../std/{manifests,grammars}/*.json`, and
   `whipplescript`/`whipplescript-store` `include_str!("../../../std/manifests/…")`
   — those workspace-root paths don't exist in a published crate, so the verify
   build fails (and `--no-verify` would ship crates nobody can build). Fix
   (0.3.1): vendor the shared std manifests + grammars into each crate that reads
   them, with a gate check keeping the vendored copies in sync with the root SSOT,
   then re-verify all five and publish `core→parser→store→kernel→whipplescript`
   (host-do is the DO host, not a CLI dep — skipped). Prereqs (done):
   verified crates.io email + a `publish-new`-scoped token (`cargo login`).

4. **Post-publish smoke.**
   `cargo install --git https://github.com/jamesjscully/whipplescript --tag v0.3.0 --locked -p whipplescript`
   then `whip --version` (0.3.0) · `whip doctor --json` ·
   `whip check examples/minimal-noop.whip`.

## v0.4.0 Cut Runbook (draft — engineering complete 2026-07-10, decisions pending)

0.4 = **version control (the versioned workspace + untie substrate
capabilities) + improve/evals**, plus the items moved here at the 0.3 cut
(web search/fetch tools; deferred DO `[~]` residuals).

**Engineering state (2026-07-10).** The version-control half is COMPLETE:
untie-substrate readiness tracker Phases 0–5 all shipped on branch `v0.4`
(formal models; the versioned-workspace floor; the 13-op workspace API with
diff/bundles/erasure/chunk transfer/selective verbs/conflict
surface/archaeology/op-undo; the chat fork; policy-epoch consumption with
DR-0036 witnessed workspace cuts + dynamic guarantee reports; auth
simplification via host-resolved provider profiles; the store-seam handles
surface, position-pair cut, and seam-contract draft). The web search + fetch
tools are built per their accepted notes (native owned harness).

**Blocking the cut — maintainer decisions/input:**

1. **Improve/evals scope (Jack).** The second half of 0.4's banner is now
   formally tracked (`experimentation-improve-tracker.md`, registered
   2026-07-10) with the settled design ground, the four open Jack-held
   design questions, and the scope analysis. **Lean recorded there: re-cut
   0.4 as the version-control release; improve/evals becomes 0.5's banner.**
   The tree is staged at 0.4.0 with an Unreleased CHANGELOG entry either
   way; only the scope sentence and the date change at cut.
2. **Sequencing fork** (untie tracker, ⚑): git-backed workspace API in
   gaugedesk first? Analysis written 2026-07-10; whip-build-order
   independent — this gates gaugedesk sequencing, not the whip cut.
3. **Production container enable** (DO tracker; Cloudflare Containers
   billing + image push) — also gates the Phase 2 presigned-transfer
   residual and the web tools' DO build boxes (`NeedsHttp` executor,
   TS-shell guard/egress entries).
4. **Workspace-DO broker, egress deny, P7 object tier** (DO tracker
   decisions).
5. **Seam-contract ratification** rides the un-tie side's co-authoring
   (`store-seam-contract-draft.md` open items: Quint twin, admitted-erasure
   command, handoff format).
6. **crates.io publishing** remains on the 0.3.1 vendoring track (above),
   independent of the 0.4 tree.

**Mechanical steps once scope is decided** (mirror the 0.3 runbook): version
bump + CHANGELOG date; full readiness gate
(`WHIPPLESCRIPT_RELEASE_READINESS_FULL=1`) green; tag → mirror publish
(preserve the grafted infra files); post-publish smoke.

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
