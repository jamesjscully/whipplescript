# Workflow Revision Transition Tracker

Status: draft tracker

This tracker covers dynamic workflow revision for running instances:

- a source bundle plus selected root workflow still compiles into an immutable
  program version
- a running instance may move to a newer active program version through an
  explicit control-plane revision operation
- existing events, facts, effects, runs, invocations, evidence, and diagnostics
  keep their original causal attribution
- queued old-version effects may be cancelled immediately when requested
- running old-version effects may receive cancellation requests, but only become
  terminal after provider/harness confirmation, timeout, or recovery
- ordinary workflow rules may propose patches through normal effects or child
  workflow invocations, but only the control plane activates a revision

This feature extends the composition model in
[workflow-composition-transition-tracker.md](workflow-composition-transition-tracker.md)
without changing the source-level meaning of `pattern`, `apply`, `workflow`,
`invoke`, `complete`, or `fail`.

## Current Position

| Area | Status | Notes |
| --- | --- | --- |
| Conceptual spec | [x] | Stage 0 landed revision epochs, activation policy, cancellation policy, compatibility rules, store objects, transaction boundaries, and observability requirements in the core specs. |
| Formal model | [x] | Stage 1 added Maude revision searches and TLA+ revision/cancel-request actions and invariants; the bounded formal suite passes. |
| Store/runtime implementation | [ ] | Revision rows, active-epoch caching, per-effect version attribution, replay, and cancellation requests are in place; active-version stepper/worker semantics remain open. |
| CLI/control plane | [ ] | `whip revise`, dry-run compatibility reports, activation reports, status revision history, and trace revision projection are in place; evidence/diagnostic previews and final UX audit remain open. |
| Generated checks | [ ] | Per-program checks do not yet assert active-version rule firing or old-effect attribution. |
| Examples/e2e | [ ] | No revision example exercises keep/cancel behavior for queued/running effects or child workflows. |

## Product Target

Revision should let operators and higher-level workflows safely adapt a running
instance without erasing history or pretending old work never happened.

Target flow:

```text
compile source bundle -> validate compatibility -> activate revision epoch
  -> future rule stepping uses the new version
  -> existing old-version effects either continue, cancel, or receive
     cancellation requests according to an explicit policy
```

Non-goals for this feature:

- self-modifying rule bodies
- inline mutation of the active program version from source rules
- automatic schema-breaking fact migration
- hidden cancellation of provider work during revision
- treating provider failure as workflow failure unless source rules choose
  `fail`

## Acceptance Gates

- [ ] Revision activation is an append-only, inspectable event with evidence
  linking old and new program versions.
- [ ] Every rule commit and effect created after activation is attributed to the
  active revision epoch.
- [ ] Effects created before activation keep their original version/epoch
  attribution.
- [ ] `whip revise --dry-run` reports compatibility, cancellation impact, and
  blocked activation reasons without mutating the store.
- [ ] `whip revise` cannot activate a revision for completed, failed, or
  cancelled instances.
- [ ] Revision cannot make old-version rules fire after the new epoch is active.
- [ ] Queued/blocked/claimable old-version effects can be terminal-cancelled when
  policy requests it.
- [ ] Running old-version effects use cancel-request semantics, not immediate
  fake terminal cancellation.
- [ ] Child workflow invocations preserve parent/child links across revision and
  project success/failure/timeout/cancellation at most once.
- [ ] Recovery preserves event order, active revision, cancellation requests,
  and terminal effect uniqueness.
- [ ] Maude and TLA+ checks cover revision/cancellation safety before runtime
  implementation lands.
- [ ] Status, trace, evidence, and diagnostics explain which version/epoch
  produced each visible action.

## Stage 0: Spec Alignment

Goal: define revision without changing the source language's composition
semantics.

- [x] Add `WorkflowRevision`, `RevisionEpoch`, and `CancellationRequest` to the
  core object vocabulary in [control-plane.md](control-plane.md).
- [x] Define revision activation as a control-plane operation, not a rule body
  operation.
- [x] Define allowed cancellation policies:
  - [x] `keep`: do not cancel old-version effects
  - [x] `cancel queued`: terminal-cancel queued, blocked, and claimable effects
  - [x] `request running`: cancel queued effects and request cancellation for
    claimed/running effects
- [x] Define revision compatibility checks:
  - [x] same root workflow name unless an explicit retarget operation is later
    designed
  - [x] input contracts cannot break already-started instance payloads
  - [x] output/failure contracts remain compatible for parent invocations
  - [x] active facts used by the new version must still typecheck or produce a
    clear activation diagnostic
  - [x] old effects keep resolved targets even if the new version removes the
    corresponding agent declaration
- [x] Define what rule proposals may do: workflows may produce patch proposal
  artifacts through `tell`, `coerce`, `askHuman`, `call`, or `invoke`, but
  activation is performed by `whip revise` or an authorized control-plane API.
- [x] Update [execution-contract.md](execution-contract.md) with revision
  transaction boundaries and cancel-request semantics.
- [x] Update [runtime-store.md](runtime-store.md) with logical revision tables
  and version attribution fields.
- [x] Update [observability.md](observability.md) with revision status, trace,
  and evidence requirements.
- [x] Audit Stage 0 against language/composition terminology, core specs, and
  cancellation semantics; record Stage 0 decisions in this tracker.

Acceptance:

- [x] Specs distinguish compile-time `apply`, runtime `invoke`, and
  control-plane `revise`.
- [x] Specs explicitly say rules cannot activate revisions directly.
- [x] Specs describe both immediate cancellation and running cancel requests.

Stage 0 audit notes:

- [control-plane.md](control-plane.md) now defines the revision object
  vocabulary, compatibility checks, cancellation policies, `whip revise` shape,
  and the `apply` / `invoke` / `revise` boundary.
- [execution-contract.md](execution-contract.md) now defines revision activation
  as a separate atomic control-plane transaction and requires active-epoch
  validation for rule commits.
- [runtime-store.md](runtime-store.md) now defines logical
  `instance_revisions`, `workflow_invocations`, and
  `effect_cancellation_requests` records plus version/epoch attribution fields.
- [observability.md](observability.md) now requires revision and cancellation
  request data in status, trace, evidence, and export surfaces.
- Stage 0 resolved the representation decisions listed below; implementation
  must still validate them through Stage 1 models before runtime code lands.

## Stage 1: Formal Model Spine

Goal: prove the revision/cancellation lifecycle before changing the runtime.

### Maude

- [x] Add revision terms to `models/maude/kernel.maude`:
  - [x] active revision/version for an instance
  - [x] rule/effect version attribution
  - [x] revision activation command
  - [x] cancellation request marker
- [x] Add `models/maude/tests/workflow-revision.maude`.
- [x] Add positive Maude searches:
  - [x] activating a revision advances the active epoch
  - [x] a new-version scoped rule can fire after activation
  - [x] old queued effects keep attribution and may cancel under policy
  - [x] running effects can move to cancel-requested and later terminal
- [x] Add negative Maude searches:
  - [x] terminal instances cannot revise
  - [x] old-version scoped rules cannot fire after activation
  - [x] revision activation does not mutate old effect attribution
  - [x] running effects do not become cancelled solely because cancellation was
    requested
  - [x] invocation effects cannot complete from parent revision alone
- [x] Update `scripts/check-formal-models.sh` expected solution counts.

### TLA+/Apalache

- [x] Extend `models/tla/ControlPlaneLifecycle.tla` with:
  - [x] `activeVersion`
  - [x] `revisionEpoch`
  - [x] `effectVersion`
  - [x] `cancelRequested`
  - [x] revision activation events
- [x] Add actions:
  - [x] `ActivateRevision`
  - [x] `RequestCancelEffect`
  - [x] `AcknowledgeCancelRun`
  - [x] `IgnoreLateCancelAfterTerminal`
- [x] Add safety invariants:
  - [x] revision epochs are monotonic
  - [x] a terminal instance has no revision activation
  - [x] claim/start use effects valid for their original version
  - [x] no effect has more than one terminal outcome across revision/recovery
  - [x] active revision is preserved by recovery
  - [x] cancel request is not equivalent to terminal cancellation
- [x] Add bounded Apalache checks for keep, queued-cancel, and request-running
  policies.
- [x] Audit Stage 1 against the new semantics, failed-search coverage, TLA+
  invariants, and CI scripts; record model gaps before runtime coding starts.

Acceptance:

- [x] `scripts/check-formal-models.sh` covers workflow revision.
- [x] TLA+ safety checks pass with revision enabled.
- [x] At least one intentionally unsafe revision behavior is caught by each
  model family.

Stage 1 audit notes:

- [models/maude/kernel.maude](../models/maude/kernel.maude) now models active
  revision epochs, version-scoped rule firing, versioned effect attribution,
  explicit revision activation, and cancel-request markers.
- [models/maude/tests/workflow-revision.maude](../models/maude/tests/workflow-revision.maude)
  covers 4 positive searches and 5 negative searches, including stale-rule
  rejection and cancel-request-not-terminal behavior.
- [models/tla/ControlPlaneLifecycle.tla](../models/tla/ControlPlaneLifecycle.tla)
  now tracks active version, revision epoch, per-effect version, cancellation
  requests, revision events, terminal instance states, and lease/run consistency.
- TLA+ caught one unsafe revision behavior during modeling: a requested-cancel
  running effect could expire its lease back to queued and become claimable
  again. `Claimable` now rejects cancel-requested effects, and the invariant was
  narrowed to assert that a cancel request is not a fake terminal cancellation.
- The bounded TLA+ model covers `keep`, queued cancellation, and
  request-running behavior as separate transition modes in one model rather than
  as separate policy-specific constants.
- `scripts/check-formal-models.sh` now runs the revision Maude test and the
  TLA+ lifecycle check.

## Stage 2: Store Schema And Kernel Contracts

Goal: persist revisions and version attribution without losing replayability.

- [x] Add `instance_revisions`:
  - [x] `revision_id`
  - [x] `instance_id`
  - [x] `epoch`
  - [x] `from_version_id`
  - [x] `to_version_id`
  - [x] `activated_by_event_id`
  - [x] `activation_policy_json`
  - [x] `cancellation_policy`
  - [x] `status`
  - [x] timestamps
- [x] Add attribution fields where needed:
  - [x] rule commit event payload includes `program_version_id` and
    `revision_epoch`
  - [x] effects record `program_version_id` and `revision_epoch`
  - [x] workflow invocations record parent/child version and parent epoch
  - [x] evidence/diagnostics can link to a revision id or epoch
- [x] Add cancel-request persistence:
  - [x] `effect_cancellation_requests` table
  - [x] optional `cancel_requested` status view without making request state a
    terminal effect status
  - [x] request reason, requester, revision id, causation event, idempotency key
  - [x] terminal outcome still uses the normal effect terminal lifecycle
- [ ] Add kernel/store operations:
  - [x] create revision candidate/dry-run report
  - [x] activate revision atomically
  - [x] list active revision for instance
  - [x] list effects impacted by a cancellation policy
  - [x] request cancellation for running effects idempotently
  - [x] terminal-cancel queued/blocked/claimable effects idempotently
- [x] Ensure projection rebuild replays revision events and effect attribution.
- [x] Add store tests for duplicate activation, idempotent cancellation,
  terminal instance rejection, and recovery replay.
- [ ] Audit Stage 2 against schema replay, transaction boundaries, idempotency,
  and migration needs; record any store compatibility gaps.

Acceptance:

- [x] A fresh store can activate and inspect multiple revisions for one
  instance.
- [x] Replay reconstructs active revision and effect cancellation requests.
- [x] Existing tests using one `instances.version_id` still pass or have an
  explicit migration path.

Stage 2 partial audit notes:

- [crates/whipplescript-store/migrations/0001_runtime_store.sql](../crates/whipplescript-store/migrations/0001_runtime_store.sql)
  now creates `instance_revisions`, `effect_cancellation_requests`,
  `instances.revision_epoch`, workflow invocation revision attribution, and
  effect version/epoch attribution columns.
- [crates/whipplescript-store/src/lib.rs](../crates/whipplescript-store/src/lib.rs)
  now activates revisions atomically, updates the active instance version/epoch,
  stores revision rows, attributes rule-created effects to the active revision,
  terminal-cancels queued/blocked old effects for revision policies, and records
  running-effect cancellation requests without changing the effect terminal
  status.
- Claimability now excludes effects with open cancellation requests, including
  effects that return to `queued` after lease expiry.
- Workflow invocation rows now preserve the parent effect's original program
  version and revision epoch plus the child instance's starting version and
  epoch, even when either side later revises independently.
- Projection rebuild now replays revision activation events, revision-triggered
  terminal cancellations, rule-commit effect attribution, and cancellation
  request events so the active revision cache and request rows can be rebuilt
  from the event log.
- Stage 2 still leaves the full candidate/dry-run report open because it
  depends on the Stage 3 compatibility analysis.

## Stage 3: Compiler And Compatibility Analysis

Goal: decide safely whether a candidate source bundle can replace the active
version for a running instance.

- [x] Add a revision compatibility analysis pass that compares active and
  candidate program versions.
- [x] Validate selected root workflow compatibility.
- [x] Validate workflow input/output/failure contracts.
- [x] Validate active fact compatibility against candidate schemas where
  possible.
- [x] Validate agent/profile/capability changes:
  - [x] new effects use candidate declarations and policies
  - [x] old effects continue with stored resolved targets and capabilities
  - [x] removed agents are reported when they affect active or queued work
- [x] Validate source bundle metadata:
  - [x] include closure hash
  - [x] root workflow
  - [x] pattern application provenance
  - [x] generated declaration hashes
- [x] Produce structured diagnostics for incompatible revisions.
- [ ] Add generated Maude checks that assert:
  - [ ] rules commit only when their version equals active revision
  - [ ] old effects keep old attribution
  - [ ] `after ... completes` still handles cancellation branches after
    revision
- [ ] Audit Stage 3 against compatibility diagnostics, generated checks, and
  current language-reference examples; record cases deferred to future explicit
  fact migration.

Stage 3 partial audit notes:

- Compiled program versions now store a structured `analysis_summary` with the
  root workflow, workflow contracts, include closure hashes, pattern application
  provenance, generated declaration names, and schema summaries.
- The store now exposes a non-mutating compatibility report comparing an
  instance's active version with a candidate version. It rejects root workflow
  changes, input contract additions/changes/removals, output/failure removals,
  and output/failure type changes with structured diagnostic codes.
- Active fact compatibility now validates live unconsumed facts against
  candidate schema summaries when the fact schema can be identified. Optional
  additive class fields are accepted; removed schemas, required-field additions,
  type changes, and removed enum variants produce structured compatibility
  diagnostics.
- Program analysis summaries now include deterministic hashes for declarations
  generated by pattern applications, so revision reports can distinguish a
  generated declaration rename from a same-name body/type change.
- Policy checks now use the effect's attributed program version when available.
  Kept old-version effects can still start against their original agent
  declarations after a revision removes or changes the candidate agent, while
  new post-revision effects use the candidate version's declarations.
- Revision reports now include non-blocking agent impact for removed candidate
  agents that still have non-terminal effects, so operators can see old work
  that will continue under stored old-version attribution.
- Generated Maude checks remain open Stage 3 work.

Acceptance:

- [x] Compatible additive revisions pass dry-run.
- [ ] Breaking contract/schema revisions fail with source-span diagnostics.
- [ ] Generated checks cover at least one revision-aware compiled fixture.

## Stage 4: Control Plane And CLI

Goal: expose revision as an explicit operator action with a safe dry-run path.

- [x] Add CLI syntax:
  - [x] `whip revise <instance> <workflow.whip> --root <name> --dry-run`
  - [x] `whip revise <instance> <workflow.whip> --root <name>`
  - [x] `whip revise <instance> <workflow.whip> --root <name> --cancel keep`
  - [x] `whip revise <instance> <workflow.whip> --root <name> --cancel queued`
  - [x] `whip revise <instance> <workflow.whip> --root <name> --cancel running`
- [x] Make `keep` the default cancellation policy.
- [ ] Make dry-run report:
  - [x] active version/epoch
  - [x] candidate version hash
  - [x] compatibility status
  - [x] impacted effects by status and version
  - [x] cancellation actions that would be taken
  - [ ] diagnostics/evidence that would be created
- [x] Make activation report:
  - [x] revision id/epoch
  - [x] old/new version ids
  - [x] cancellation summary
  - [x] next recommended command
- [ ] Require explicit confirmation flags for future destructive policies if
  additional policy levels are introduced.
- [x] Update `whip status --json` with active revision and revision history.
- [x] Update `whip trace` with revision activation and cancellation-request
  records.
- [ ] Audit Stage 4 against CLI UX, JSON stability, status readability, and
  operator failure modes; record missing surfaces.

Stage 4 partial audit notes:

- `whip revise` now compiles the candidate source bundle, runs compatibility
  analysis against the active instance, reports cancellation impact, and keeps
  `--dry-run` non-mutating.
- Activation reuses the same compatibility path, persists the candidate program
  version only after compatibility passes, and calls the store's atomic revision
  activation operation.
- JSON output includes compatibility diagnostics, active version/epoch,
  candidate hashes, terminal-cancel/request-cancel effect lists, and activation
  details.
- `whip trace --check --json` now reconstructs abstract revision activation and
  cancellation-request records from the event log, including revision ids,
  version ids, epoch transitions, cancellation policy, and requested effects.

Acceptance:

- [x] `whip revise --dry-run` never mutates the store.
- [x] `whip revise` mutates the store through one atomic activation path.
- [x] Status makes revision and cancellation state visible without opening
  SQLite.

## Stage 5: Runtime Stepper And Worker Semantics

Goal: make stepping and worker execution honor active revision and cancellation
state.

- [x] Update `whip step` to validate the supplied program against the active
  program version for the instance instead of trusting an arbitrary path for
  production stepping.
- [x] Keep a development override only if it is explicitly named and cannot
  silently revise a running instance.
- [x] Ensure deterministic stepping uses only rules from the active revision.
- [x] Ensure idempotency keys include program version/revision epoch where
  required by the execution contract.
- [x] Ensure existing old-version effects remain claimable/runnable unless
  cancellation policy blocks them.
- [x] Implement queued/blocked/claimable terminal cancellation for revision
  policies.
- [ ] Implement running cancel-request behavior:
  - [ ] agent harness observes cancellation request before launch where possible
  - [ ] long-running providers can be asked to stop where supported
  - [ ] unsupported providers leave visible diagnostics and keep recoverable
    lease state
  - [x] late provider completion after cancel-request records the real terminal
    outcome
- [ ] Update child workflow invocation handling so parent and child revisions
  are explicit and cancellation projection remains single-shot.
- [ ] Audit Stage 5 against rule attribution, worker cancellation, lease
  recovery, and child invocation behavior; record edge cases for e2e.

Acceptance:

- [x] Old rules cannot create new effects after revision.
- [x] Old effects can finish after revision with old attribution.
- [x] Running cancel requests do not fabricate terminal cancellation.

Stage 5 partial audit notes:

- `whip step` now compiles the operator-supplied program path and rejects it
  unless its source and IR hashes match the instance's active program version.
  This prevents stale pre-revision source from committing old rules after a
  revision activation.
- Rule commit idempotency keys now include the active program version and
  revision epoch, so equivalent rule/context/lowering output does not collide
  across revision epochs.
- `whip dev` and child-invocation stepping remain explicit development paths
  that pass an in-memory IR; their revision behavior still needs the broader
  Stage 5 worker/child audit.
- A keep-policy revision leaves existing old-version effects claimable; fixture
  worker completion preserves the old effect's original program version and
  revision epoch attribution.
- Old-version effects also keep their original agent/profile/capability policy
  context after a keep-policy revision, while new effects use the active
  candidate declaration set.
- A request-running revision now has CLI coverage showing that running effects
  stay `running`, expose `cancel_requested` on effects/runs, and produce a
  conforming trace cancellation-request record without a fabricated terminal
  cancellation.
- The same request-running coverage now completes the requested effect with a
  late real provider success and verifies that the cancellation request resolves
  to `terminal` while the effect records `completed`.
- A queued-cancel revision now has CLI coverage showing that queued old-version
  effects become terminal `cancelled` effects and produce a conforming trace
  cancellation record.

## Stage 6: Observability, Evidence, And Diagnostics

Goal: make revision understandable from the normal inspection surfaces.

- [x] Add revision events to `whip log`.
- [x] Add active revision and history to `whip status`.
- [x] Add per-effect version/epoch to `whip effects --json`.
- [x] Add cancellation request state to `whip effects` and `whip runs`.
- [ ] Add evidence links:
  - [ ] revision event -> old/new program versions
  - [ ] revision event -> compatibility diagnostics
  - [ ] revision event -> cancelled/requested effects
  - [ ] cancellation request -> provider/run evidence
- [ ] Add diagnostics for:
  - [ ] incompatible root workflow
  - [ ] incompatible input/output/failure contract
  - [ ] active fact schema mismatch
  - [ ] unsupported running cancellation provider
  - [ ] stale program path or missing source bundle during revision
- [x] Update trace conformance to understand revision and cancel-request
  records.
- [ ] Audit Stage 6 against operator debugging workflows, trace JSON, evidence
  links, and diagnostic source spans; record remaining explainability gaps.

Acceptance:

- [x] An operator can answer "which version created this effect?" from CLI
  output.
- [x] An operator can answer "what did revision cancel or request to cancel?"
  from CLI output.
- [x] Trace conformance rejects impossible revision/cancellation sequences.

## Stage 7: Tests And E2E

Goal: cover the feature through deterministic tests before real-provider
validation.

- [ ] Add parser/compiler fixtures for revision-compatible and
  revision-incompatible source bundles.
- [ ] Add store/kernel unit tests:
  - [ ] activation creates revision row and event
  - [ ] terminal instances reject activation
  - [ ] old effect attribution is stable
  - [ ] queued cancellation is idempotent
  - [ ] running cancellation request is idempotent
  - [ ] recovery preserves active revision
- [ ] Add CLI tests:
  - [x] dry-run compatible
  - [x] dry-run incompatible
  - [x] activation with `--cancel keep`
  - [x] activation with `--cancel queued`
  - [x] activation with `--cancel running`
  - [x] status/trace JSON after revision
- [ ] Add e2e workflows:
  - [ ] v1 starts an agent turn, v2 changes future dispatch
  - [ ] v1 queued effect is cancelled during revision
  - [ ] v1 running effect receives cancel request and later completes/fails
  - [ ] parent invokes child, parent revises while child is running
  - [ ] child revises independently and parent observes terminal output
- [ ] Add generated Maude fixture from a revision-aware compiled example.
- [ ] Audit Stage 7 against formal model coverage, unit coverage, CLI coverage,
  and deterministic e2e; record flaky or provider-dependent gaps.

Acceptance:

- [ ] `cargo test --workspace` covers revision happy and failure paths.
- [ ] `scripts/check-formal-models.sh` covers revision.
- [ ] Deterministic fixture-provider e2e covers keep, queued-cancel, and
  running-cancel-request policies.

## Stage 8: Docs, Examples, And validation

Goal: teach the feature without encouraging source-level self-modifying rules.

- [ ] Update [docs/language-reference.md](../docs/language-reference.md) to say
  revision is a control-plane operation and patch proposals are ordinary
  workflow outputs/artifacts.
- [ ] Update [quickstart.md](quickstart.md) with a small `whip revise
  --dry-run` example after the basic run/step flow.
- [ ] Update [operator-guide.md](operator-guide.md) with revision, rollback-by-
  new-revision, and cancellation policy guidance.
- [ ] Update [troubleshooting.md](troubleshooting.md) with common revision
  diagnostics.
- [ ] Add examples:
  - [ ] repair planner workflow that proposes a patch artifact
  - [ ] revised workflow v1/v2 pair
  - [ ] parent/child invocation revision example
  - [ ] running cancellation request example with fixture provider
- [ ] Add a validation workflow that invokes a patch-proposal child workflow, asks
  human approval, and leaves instructions for running `whip revise`.
- [ ] Audit Stage 8 against docs, examples, companion skill guidance, and
  user-facing terminology; record outdated references.

Acceptance:

- [ ] Docs do not imply ordinary rules can activate revisions.
- [ ] Examples show both keeping and cancelling old work.
- [ ] Companion guidance tells agents to propose patches, not self-modify live
  instances.

## Stage 9: Final Release Audit

Goal: decide whether workflow revision is ready to ship or should remain
experimental.

- [ ] Audit Stage 0 findings and close or classify all spec gaps.
- [ ] Audit Stage 1 findings and close or classify all formal model gaps.
- [ ] Audit Stage 2 findings and close or classify all store/kernel gaps.
- [ ] Audit Stage 3 findings and close or classify all compatibility-analysis
  gaps.
- [ ] Audit Stage 4 findings and close or classify all CLI/control-plane gaps.
- [ ] Audit Stage 5 findings and close or classify all runtime/worker gaps.
- [ ] Audit Stage 6 findings and close or classify all observability gaps.
- [ ] Audit Stage 7 findings and close or classify all test/e2e gaps.
- [ ] Audit Stage 8 findings and close or classify all docs/example gaps.
- [ ] Run required checks:
  - [ ] `cargo test --workspace`
  - [ ] `scripts/check-formal-models.sh`
  - [ ] revision-specific deterministic e2e
  - [ ] trace conformance on revised instances
- [ ] Decide release status:
  - [ ] stable
  - [ ] experimental behind feature flag
  - [ ] deferred
- [ ] Record final audit summary in [final-audit.md](final-audit.md) or a
  linked release note.

Acceptance:

- [ ] Every stage has an explicit audit result.
- [ ] All release-blocking gaps are closed.
- [ ] Remaining non-blocking gaps have owners, rationale, and follow-up tasks.

## Stage 0 Decisions

- [x] `instances.version_id` remains a denormalized active-version cache; the
  canonical append-only history lives in `instance_revisions`.
- [x] Running cancellation uses a separate logical
  `effect_cancellation_requests` table. Status views may display
  `cancel_requested`, but request state is not a terminal effect status.
- [x] The CLI uses `--cancel running`; the semantic policy name is
  `request running`.
- [x] Root workflow retargeting is not allowed in v0. A future retarget feature
  must be an explicit control-plane operation, not implicit revision behavior.
- [x] Active facts read by candidate rules or schemas are strict compatibility
  inputs: incompatible facts block activation with diagnostics.
- [x] Rollback is another revision to a prior program version, not a special
  rollback command in v0.

## Next Implementation Slice

1. Add Stage 1 Maude/TLA+ revision checks and wire them into
   `scripts/check-formal-models.sh`.
2. Implement Stage 2 store schema and kernel operations behind tests before
   adding the CLI.
3. Add Stage 3 compiler compatibility analysis before exposing `whip revise`.
