# Final Audit

Status: draft

This file collects the audit findings from the staged implementation plan and
classifies them for v0.

## Workflow Revision Final Audit

Status: stable for the in-repo v0 runtime and CLI.

Date: May 30, 2026.

Workflow revision is implemented as an explicit control-plane operation for
non-terminal running instances. It supports compatibility dry-runs, activation
epochs, active-version stepping, old-effect attribution, queued cancellation,
running cancellation requests, parent/child invocation attribution, evidence,
diagnostics, trace conformance, docs, examples, and companion authoring
guidance.

Verification run for this audit:

| Check | Result |
| --- | --- |
| `cargo test --workspace` | Passed. |
| `scripts/check-formal-models.sh` | Passed, including `workflow-revision.maude` and TLA+/Apalache bounded check to length 6. |
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
| G-005 | Loft | Deferred with rationale | WhippleScript maintainer, Loft maintainer | Loft repo is not added as a submodule with conformance fixtures. | The Loft repo currently has `spec/loft-v0.1.md` and `fixtures/whipplescript/v0.1` untracked, so a submodule would not preserve the spec/fixtures. WhippleScript implements the Loft v0.1 command contract locally and carries the manifest-driven compatibility fixtures under `examples/loft-fixtures/v0.1`; submodule fixture wiring should wait for a Loft commit containing the spec/fixtures. `scripts/stage-loft-fixtures.sh` stages the manifest and compatibility fixtures into Loft, `scripts/export-loft-source-patch.sh` emits a reviewable Loft-side patch, `scripts/loft-handoff-report.sh` records current blockers and next commands, `scripts/check-loft-source-repo.sh` centralizes tracked spec/fixture and clean-worktree preflight, and `scripts/add-loft-submodule.sh` guards the eventual submodule add. `scripts/check-loft-fixtures.sh` validates fixture shape against an override, submodule fixtures, or local compatibility fixtures, while `WHIPPLESCRIPT_REQUIRE_LOFT_SUBMODULE_FIXTURES=1` rejects fallback fixtures when source-of-truth CI is required. Follow up in `spec/loft-integration.md` and Stage 8 of `spec/implementation-plan.md`. |
| G-006 | Real providers | Deferred with rationale | WhippleScript integration maintainer | Real-provider destructive e2e flows remain manual. | `scripts/check-real-providers.sh` gates and documents prerequisites, supports selected provider smoke runs with `WHIPPLESCRIPT_REAL_PROVIDERS`, probes tool versions, checks Loft fixture readiness through `scripts/check-loft-source-repo.sh` when Loft is selected, verifies BAML endpoint reachability when BAML is selected, and runs no-mock read-only Loft show plus BAML coerce smoke tests when configured. `scripts/check-real-providers-report.sh` records smoke output and set/unset environment posture without exposing values, and `scripts/check-release-readiness.sh` records real-provider and Loft handoff reports as CI artifacts. Automated destructive Loft flows still need isolated external fixtures. Follow up in `spec/e2e.md` and Stage 11/DoD of `spec/implementation-plan.md`. |
| G-007 | Control plane | Blocking for validation | WhippleScript runtime maintainer | `whip run` starts an instance but does not drive ready rules into durable facts/effects. | Validation workflows can compile and record `external.started`, but no `PhaseReviewRequest` facts or `agent.tell` effects are materialized from source rules. Add an explicit `whip step` / `whip dev` driver that evaluates ready rules from IR and commits fact/effect rewrites before provider workers claim effects. |
| G-008 | Agent harnesses | Blocking for real provider validation | WhippleScript integration maintainer | Codex, Claude, and Pi real adapters are overspecified as simple command wrappers. | Codex should use Codex App Server or Codex SDK surfaces; the Codex desktop app's private app server should not be assumed externally reachable. Claude should use Claude Agent SDK with API/provider auth and explicit tool/profile mapping. Pi should use the Pi extension system, with conversation thread ids and snapshots recorded as first-class evidence. |
| G-009 | Harness failures | Blocking for real provider validation | WhippleScript runtime maintainer | Harness boundary failures are only partially implemented. | Once a run starts, mock/command harness failures are normalized into terminal effect events and `agent.turn.*` facts. The full system still needs explicit worker coverage for pre-run and boundary failures: provider config, credentials, workspace preparation, adapter resolution, launch, request submission, streaming, timeout/cancel, result validation, artifact capture, and terminal-event append recovery. |
| G-010 | Language | Partially fixed | WhippleScript language maintainer | Deterministic provider routing initially required duplicate provider-specific schemas. | Guarded `when` clauses now let `provider-language-e2e.whip` route one shared `LanguageTask` schema by `task.provider`, provider/model identity is no longer delegated to BAML review output, and source assertions check provider/effect counts. Remaining ergonomic work: full expression-kernel typing/evaluation, typed dynamic agent references, static matrices, and static action/template expansion. |

No blocking audit gaps are currently identified for the in-repo mock-provider
v0 spine. G-007, G-008, and G-009 block real local validation with external coding
agents. G-010 no longer blocks the provider-language matrix, but the remaining
items still matter before the language is treated as comfortable for broader
provider matrices.

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
  events, human review, Loft, BAML, skills, and retry paths.
- Duplicate terminal completion is rejected atomically.
- Lease expiry, retry, and recovery are covered by unit and e2e tests.
- Dependency release correctness is covered by store tests, trace conformance,
  generated Maude searches, and repeated e2e stress.
- Pause/resume/cancel now gate provider starts at the store boundary.
- Multi-instance isolation is covered by CLI and kernel e2e tests.
- Loft/BAML integration semantics use deterministic fake clients and command
  contract tests, including Loft claim/renew/release, structured evidence,
  resource intent, complete, and fail command shapes.

Result: no blocking distributed-systems gaps identified. External fixture
coverage remains deferred under G-005 and G-006.

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

Required before declaring final v0:

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
