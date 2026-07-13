# Final Audit

Status: active v0 audit log; workflow revision audit complete; v0.2 release
readiness signed off (baseline green, 2026-07-05).

This file collects the audit findings from the staged implementation plan and
classifies them for v0.

## v0.2 Release Readiness (2026-07-05)

Release plan: the `spec/TRACKERS.md` Release column. v0.2 =
language + std packages + native runtime, polished; v0.3 = cloud + owned harness;
v0.4 = improve/evals + version control.

**Code baseline: GREEN and verified.** `WHIPPLESCRIPT_RELEASE_READINESS_FULL=1
scripts/check-release-readiness.sh` exits 0 at commit `2bafd44` — Required checks
failed: 0 (shell, trackers, docs snippets/site, IR goldens, report schemas,
artifact admission, fmt, clippy `-D warnings`, whitespace, native
adapters/contract/policy-denials, control-plane, workspace records, provider
scheduling, expression routing, incident UX, cancellation matrix, store replay,
provider doctor, artifact redaction, and — under FULL — workspace tests, Maude,
TLA, e2e). Two required failures were fixed en route: clippy (`cf2f64a`: box
`Item::Source`; `.expect` over `.unwrap` in the context-assembly test) and a stale
IR golden (`2bafd44`: `package-memory.ir` `tools=[]`).

**Only remaining gate red = external native-provider surface probe** — the live
tooling is absent in the build environment; non-blocking (`STRICT_EXTERNAL=0`).
Becomes blocking for the v0.2 ship gate under the "advertise production native
support" decision (see below), which requires a live run.

**Distribution: mechanically ready.** cargo-dist 0.32.0 + `dist-workspace.toml`
(5 targets, shell/powershell installers); `dist plan` resolves 15 artifacts
clean. Crates carry versioned path deps for ordered crates.io publish. **All
crates are at `0.1.0`; a bump to `0.2.0` is the required last-before-tag step.**

**Ship gate — remaining items (owner-gated, not code):**

| Item | Status | Owner action |
| --- | --- | --- |
| S2/S3 std-package renames + S2b model-id effect-key fold | **DONE** — built, runtime-verified, gate-green (e57be7d / bd940e4 / 8abc7aa); DR-0014 → accepted | None |
| Native-provider live validation (G-008) | **Codex + Claude live-validated 2026-07-05** — app-server / Agent SDK / native source-workflow / artifact / error smokes + endpoint-health + config, strict gate green (external failures 0). En route the surface probe was switched off `rg`→`grep` and two native-workflow smokes that predated the agent-provider validation were unbroken. | At cut: re-run `WHIPPLESCRIPT_RELEASE_STRICT_EXTERNAL=1 scripts/check-release-readiness.sh` with `codex` + `claude` logged in (the gate self-supplies config + disposable acks). |
| Publish | Plumbing verified; version at `0.2.0` | Tag push (triggers release workflow), crates.io (dependency order) + Homebrew credentials |

Not v0.2: `consume` removal (deprecation-window gated — stays through v0.2,
removed in v0.3); B1a lowering-move (deferred to v0.3 with the DO sans-IO
refactor); expression-kernel is already thoroughly tested (206 parser tests) — the
tracker's remaining rows are deferred-with-cause polish, not release blockers.

## Workflow Revision Final Audit

Status: stable for the in-repo v0 runtime and CLI.

Date: May 30, 2026.

Workflow revision is implemented as an explicit control-plane operation for
non-terminal running instances. It supports compatibility dry-runs, activation
epochs, active-version stepping, old-effect attribution, queued cancellation,
running cancellation requests, parent/child invocation attribution, evidence,
diagnostics, trace conformance, docs, examples, and companion authoring
guidance.

Final 0-9 workflow revision audit pass: May 30, 2026. This pass verified the
tracker claims against the specs, store schema, runtime/kernel behavior, CLI
control plane, formal models, docs, examples, and companion authoring guidance.

Verification run for this audit:

| Check | Result |
| --- | --- |
| `cargo fmt --all -- --check` | Passed. |
| `cargo clippy --workspace --all-targets -- -D warnings` | Passed after narrow mechanical lint cleanup in parser/store/kernel/CLI helpers. |
| `cargo test --workspace` | Passed. |
| `scripts/check-formal-models.sh` | Passed, including `workflow-revision.maude` and TLA+/Apalache bounded check to length 6. |
| `scripts/check-tla-models.sh` | Passed with Apalache bounded check to length 6. |
| `scripts/check-e2e.sh` | Passed 17 kernel e2e tests and 54 CLI control-plane tests. |
| `cargo test -p whipplescript-kernel --test e2e revision` | Passed 5 revision e2e tests covering keep, queued cancel, running cancel request, parent revision with child running, and independent child revision. |
| `cargo test -p whipplescript -- reconstructs_revision_trace_records_from_store_events renders_revision_log_event_details` | Passed revision trace/log reconstruction checks. |

Non-blocking revision follow-ups:

| Area | Owner | Rationale / Follow-up |
| --- | --- | --- |
| Root workflow retargeting | WhippleScript runtime maintainer | Retargeting is intentionally out of scope for v0. Add an explicit retarget control-plane operation if operators need it later. |
| Live fact migration | WhippleScript language maintainer | Current compatibility checks reject or classify incompatible active facts. Add source-declared migration plans before allowing schema-breaking revision. |
| Provider cancellation depth | WhippleScript integration maintainer | Running cancellation requests are persisted and observable. Individual providers may still lack out-of-band cancellation support and should add provider-specific acknowledgements over time. |
| Broader destructive policies | WhippleScript control-plane maintainer | Current policies are explicit `keep`, `queued`, and `running`. Any more destructive future policy must add a dedicated confirmation flag. |

Result: no release-blocking workflow revision gaps remain for the local
deterministic runtime, SQLite store, CLI, formal model, and fixture-provider
e2e surface.

## Gap List

| ID | Area | Classification | Owner | Finding | Rationale / Follow-up |
| --- | --- | --- | --- | --- | --- |
| G-001 | Verification | Already fixed | WhippleScript maintainer | Add an intentionally unsafe generated-check fixture. | The CLI test suite now compiles a real example, injects an unsafe dependency-release rule into the generated Maude module, and verifies Maude produces the expected counterexample when available. |
| G-002 | Language | Partially fixed | WhippleScript language maintainer | Full expression-kernel guard typing is incomplete. | Equality guards in `when` clauses now parse and run for the provider-routing validation path, and assertions can check fact/effect projections. Remaining work is tracked in `spec/expression-kernel-tracker.md`: typed expression AST, boolean logic, ordering, membership, exists/empty, optional presence proofs, enum/literal domain checks, shared guard/assertion evaluation, and generated guard-gated Maude checks. |
| G-003 | Language | Already fixed | WhippleScript language maintainer | `as binding` after a closing multiline string is unsupported. | The parser now emits a targeted diagnostic that tells authors to move `as <binding>` onto the effect line. |
| G-004 | Companion skill | Already fixed | WhippleScript maintainer | Companion skill package/install automation is not implemented. | First-party skill content exists at `skills/whipplescript-author/SKILL.md`, with local install automation in `scripts/install-whipplescript-skill.sh` and package automation in `scripts/package-whipplescript-skill.sh`. |
| G-006 | Real providers | Deferred with rationale | WhippleScript integration maintainer | Real-provider destructive e2e flows remain manual. | `scripts/check-real-providers.sh` gates and documents prerequisites, supports selected provider smoke runs with `WHIPPLESCRIPT_REAL_PROVIDERS`, probes tool versions, verifies coerce endpoint reachability when coerce is selected, and runs no-mock read-only coerce smoke tests when configured. `scripts/check-real-providers-report.sh` records smoke output and set/unset environment posture without exposing values, and `scripts/check-release-readiness.sh` records real-provider handoff reports as CI artifacts. Automated destructive real-provider flows still need isolated external fixtures. Follow up in `spec/e2e.md` and Stage 11/DoD of `spec/implementation-plan.md`. |
| G-007 | Control plane | Already fixed | WhippleScript runtime maintainer | `whip run` started an instance but did not drive ready rules into durable facts/effects. | `whip step`, `whip worker`, and `whip dev` now share a durable rule/effect driver that materializes source rules into facts/effects before provider workers start runs. `scripts/check-control-plane-driver.sh` covers startup, step materialization, worker execution, provider-matrix dev routing, and source workflow execution through the native fixture bridge. |
| G-008 | Agent harnesses | Partially fixed | WhippleScript integration maintainer | Codex and Claude real adapters were overspecified as simple command wrappers. | Native Codex app-server and Claude Agent SDK adapter paths now exist with deterministic validation, redacted lifecycle evidence, cancellation, artifact metadata, and source workflow bridge coverage. Live isolated provider smokes are environment-gated and remain a strict external release gate before advertising production native-provider support. Track remaining live/provider-specific work in `spec/real-provider-validation-tracker.md`. |
| G-009 | Harness failures | Already fixed | WhippleScript runtime maintainer | Harness boundary failures were only partially implemented. | Command-backed and native-provider paths now classify boundary failures, artifact-capture failures, provider-native cancellation, terminal-event append recovery, and restart recovery into durable diagnostics without leaking raw secrets. Validation is covered by `scripts/check-real-provider-report-redaction.sh`, `scripts/check-native-provider-contract.sh`, `scripts/check-cancellation-policy-matrix.sh`, and `scripts/check-store-replay-conformance.sh`. |
| G-010 | Language | Partially fixed | WhippleScript language maintainer | Deterministic provider routing initially required duplicate provider-specific schemas. | Guarded `when` clauses now let `provider-language-e2e.whip` route one shared `LanguageTask` schema by `task.provider`, provider/model identity is no longer delegated to coerce review output, and source assertions check provider/effect counts. Remaining ergonomic work: full expression-kernel typing/evaluation, typed dynamic agent references, static matrices, and static action/template expansion. |

No blocking audit gaps are currently identified for the in-repo v0 spine.
G-008 remains the only partially fixed native-provider audit gap: deterministic
native adapter coverage exists, but live Codex and Claude validation is
still environment-gated and must pass before the release advertises production
native-provider support. G-010 no longer blocks the provider-language matrix,
but the remaining items still matter before the language is treated as
comfortable for broader provider matrices.

## Security Boundaries

- Capability/profile enforcement is implemented before provider runs, including
  e2e coverage for denied effects.
- Provider credentials are not stored in workflow source; real-provider checks
  are opt-in and environment-gated.
- Prompt/input/output retention is explicit through events, artifacts, facts,
  and evidence. Operators should treat the SQLite store as sensitive.
- Plugin manifests register capabilities, providers, profiles, and bindings;
  plugins do not gain new control-flow semantics.
- Local filesystem and network access are provider concerns and must be bounded
  by profile/capability policy.

Result: no blocking security gaps in the mock-provider spine. Real external
provider isolation remains deferred under G-006.

## Efficiency And Performance

- Parser and static checks are linear over small source files and covered by
  snapshot fixtures.
- Scheduler/status paths use SQLite projections and indexes from migrations.
- Event replay is supported for rule-commit projections and covered by e2e
  restart/rebuild tests.
- Evidence/artifact storage is append-only and auditable; retention policy is an
  operator/deployment concern.
- CLI inspection commands are direct projection reads.

Result: no blocking efficiency gaps identified. Large-program and long-running
store benchmarks are future hardening work.

## Distributed-Systems Integrity

- Idempotency keys are used for program events, rule commits, provider terminal
  events, human review, coerce, skills, and retry paths.
- Duplicate terminal completion is rejected atomically.
- Lease expiry, retry, and recovery are covered by unit and e2e tests.
- Dependency release correctness is covered by store tests, trace conformance,
  generated Maude searches, and repeated e2e stress.
- Pause/resume/cancel now gate provider starts at the store boundary.
- Multi-instance isolation is covered by CLI and kernel e2e tests.
- Coerce integration semantics use deterministic fake clients and command
  contract tests, including structured evidence and fail command shapes.

Result: no blocking distributed-systems gaps identified. Destructive external
provider fixture coverage remains deferred under G-006.

## Reliability And Operability

- Critical store operations are transactional and covered by rollback tests.
- `doctor`, `status`, `effects`, `runs`, `evidence`, and `trace --check`
  provide operator visibility.
- E2E tests export trace artifacts before conformance checks.
- Migrations create the current runtime schema from a clean store.
- Quickstart, operator, plugin author, troubleshooting, release, and migration
  docs exist under `spec/`.

Result: no blocking reliability gaps identified.

## Verification Rerun

Completed for the May 30, 2026 workflow revision final audit:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
scripts/check-formal-models.sh
scripts/check-tla-models.sh
scripts/check-e2e.sh
```

Optional, environment-gated:

```sh
WHIPPLESCRIPT_E2E_REAL_PROVIDERS=1 scripts/check-real-providers.sh
```
