# Workflow Revision Follow-Ups Tracker

Status: active vNext planning tracker; docs/spec-only.

This tracker covers non-blocking follow-ups from
[final-audit.md](final-audit.md) after the v0 workflow revision implementation
was declared stable for the local deterministic runtime and CLI.

The v0 revision contract remains unchanged:

- `whip revise` may activate a new program version for one non-terminal
  instance when the selected root workflow is unchanged and compatibility
  checks pass.
- Existing events, facts, effects, runs, invocations, diagnostics, artifacts,
  and evidence retain their original attribution.
- Active facts are typechecked against the candidate program; schema-breaking
  migration is not automatic.
- Queued old-version work may be terminal-cancelled, while running old-version
  work only receives durable cancellation requests.
- Broader destructive policies are not available without a future explicit
  confirmation surface.

## Current Position

| Area | Status | Boundary |
| --- | --- | --- |
| Root workflow retargeting | [ ] | v0 rejects candidate roots that do not match the active instance root. vNext needs a separate explicit retarget operation with contract mapping, parent-invocation compatibility, and traceable operator intent. |
| Source-declared live fact migration | [ ] | v0 rejects or reports active facts that no longer typecheck. vNext needs source-declared, deterministic migration plans before allowing schema-breaking activation. |
| Provider-specific out-of-band cancellation depth | [ ] | v0 persists running cancellation requests and waits for provider, timeout, or recovery terminal outcomes. vNext needs provider capability declarations and acknowledgements for deeper cancellation support. |
| Broader destructive policy confirmations | [ ] | v0 supports only `keep`, `queued`, and `running`. vNext destructive policies must be opt-in, named, previewed, confirmed, and auditable. |

## v0 And vNext Boundaries

v0 is stable and should not be reinterpreted by this tracker. Implementations of
this tracker must be additive and feature-gated until their acceptance gates are
complete.

v0 must continue to:

- reject ordinary revision when the candidate root workflow differs from the
  active instance root
- reject schema-breaking active fact incompatibilities unless all affected
  facts are handled by an implemented migration plan
- treat running cancellation request as intent, not a terminal outcome
- reject unknown or more destructive cancellation policies

vNext may add:

- `WorkflowRetarget`, a distinct control-plane activation object and CLI/API
  operation for changing the root workflow of a running instance
- source-declared migration plans that transform active facts in an atomic
  activation transaction
- provider capability metadata for cancellation request, acknowledgement,
  cooperative stop, session interrupt, and force-kill depth
- dedicated confirmation flags for destructive policies that drop, consume,
  cancel, or migrate durable state beyond v0 behavior

## Acceptance Gates

- [ ] Existing v0 `whip revise` behavior and tests remain unchanged unless a new
  flag/API operation is used.
- [ ] Root retargeting is exposed as an explicit operation, not as accidental
  `--root` drift on `whip revise`.
- [ ] Retarget dry-run reports old root, new root, input/output/failure contract
  compatibility, parent invocation impact, active fact impact, cancellation
  impact, required migrations, and destructive confirmations.
- [ ] Retarget activation is append-only, atomic, idempotent, and blocked for
  terminal instances.
- [ ] Source-declared migration plans are deterministic, typed, versioned, and
  tied to source spans and program versions.
- [ ] Migration activation records every consumed, transformed, retained,
  tombstoned, and rejected active fact with evidence and diagnostics.
- [ ] No active fact is silently coerced or dropped because a candidate program
  removed or changed a schema.
- [ ] Provider-specific cancellation depth is declared by provider bindings and
  observable per cancellation request.
- [ ] A cancellation acknowledgement never creates duplicate terminal outcomes
  and never overwrites a late real provider completion.
- [ ] Every policy more destructive than v0 `queued`/`running` requires a
  dedicated confirmation flag and dry-run preview of impacted durable objects.
- [ ] Formal models cover each new state transition before runtime
  implementation lands.
- [ ] Status, trace, evidence, diagnostics, and export surfaces can explain
  retarget, migration, provider acknowledgement, and destructive confirmation
  outcomes.
- [ ] Deterministic fixture-provider e2e covers success, rejection, idempotent
  retry, and recovery for each feature slice.

## Stage 0: Spec Alignment

Goal: define the additive contracts without weakening v0 revision semantics.

- [ ] Add conceptual vocabulary:
  - `WorkflowRetarget`
  - `RetargetEpoch` or a retarget subtype of `WorkflowRevision`
  - `FactMigrationPlan`
  - `FactMigrationRun`
  - `ProviderCancellationCapability`
  - `CancellationAcknowledgement`
  - `DestructiveRevisionConfirmation`
- [ ] Decide whether retarget uses a separate CLI verb (`whip retarget`) or an
  explicit `whip revise --retarget` mode. The first implementation should
  prefer a separate operation unless API ergonomics strongly justify otherwise.
- [ ] Specify that ordinary `whip revise --root DifferentRoot` remains rejected
  without the explicit retarget operation.
- [ ] Define source-level migration declaration shape without allowing arbitrary
  provider calls, filesystem access, network access, wall-clock time, or random
  sources during migration.
- [ ] Define provider cancellation capability metadata in provider/profile
  bindings rather than hard-coding provider names into the kernel.
- [ ] Define the naming rule for destructive confirmation flags: each flag
  confirms one class of destructive behavior and cannot be reused as a generic
  `--force`.

Acceptance:

- [ ] Core specs distinguish ordinary revision, retarget, migration, provider
  cancellation acknowledgement, and destructive confirmation.
- [ ] v0 rejection behavior is still the default when new flags or operations
  are absent.

## Stage 1: Formal Model Requirements

Goal: prove the new transitions before adding runtime code.

Maude requirements:

- [ ] Model explicit retarget activation separately from ordinary revision.
- [ ] Add positive searches for retarget with compatible contracts and no active
  fact loss.
- [ ] Add negative searches showing ordinary revision cannot change root.
- [ ] Add fact migration rules for transform, retain, tombstone, and reject
  outcomes.
- [ ] Add searches proving migration cannot drop active facts without a declared
  destructive confirmation.
- [ ] Add provider cancellation acknowledgement and force-stop capability
  states.
- [ ] Add searches proving acknowledgement is not a duplicate terminal outcome.
- [ ] Add destructive policy confirmation tokens and negative searches for
  unconfirmed destructive activation.

TLA+/Apalache requirements:

- [ ] Add `activeRoot`, `retargetEpoch`, `migrationPlan`, `migrationRun`,
  `providerCancelCapability`, and `destructiveConfirmation` variables.
- [ ] Add `DryRunRetarget`, `ActivateRetarget`, `ApplyFactMigration`,
  `AcknowledgeCancellation`, and `ActivateDestructivePolicy` actions.
- [ ] Add invariants:
  - ordinary revision preserves `activeRoot`
  - retarget advances epoch monotonically and records old/new roots
  - every active incompatible fact is migrated, retained, rejected, or
    explicitly tombstoned
  - no migration consumes facts outside its declared read set
  - cancellation acknowledgement preserves single terminal outcome
  - destructive policies require matching confirmation payloads
- [ ] Add bounded checks for retarget success/reject, migration success/reject,
  provider acknowledgement, late completion after acknowledgement attempt, and
  missing destructive confirmation.

Acceptance:

- [ ] `scripts/check-formal-models.sh` covers all new Maude searches.
- [ ] `scripts/check-tla-models.sh` covers bounded vNext follow-up transitions.
- [ ] Each feature has at least one intentionally unsafe fixture caught by the
  model suite.

## Stage 2: Store And Control-Plane Surfaces

Goal: define durable state before implementation.

Logical store additions:

- `workflow_retargets`: retarget id, instance id, old root, new root, old/new
  program versions, old/new epochs, activation event, compatibility report,
  idempotency key, confirmation ids, and timestamps
- `fact_migration_plans`: plan id, source program version, target program
  version, source span, input fact schemas, output fact schemas, declared read
  set, declared write set, and deterministic plan hash
- `fact_migration_runs`: migration run id, activation id, plan id, consumed fact
  ids, produced fact ids, tombstoned fact ids, rejected fact ids, diagnostics,
  evidence ids, and idempotency key
- `provider_cancellation_capabilities`: provider binding, profile, supported
  depth, acknowledgement mode, timeout policy, evidence requirements, and
  native enforcement level
- `cancellation_acknowledgements`: cancellation request id, run id, provider,
  depth attempted, acknowledgement payload, evidence ids, terminal event id when
  one exists, and idempotency key
- `destructive_revision_confirmations`: confirmation id, activation id, policy
  name, confirmation flag, impact hash, operator identity, reason, evidence ids,
  and idempotency key

Control-plane API surfaces:

- `dry_run_retarget(instance, candidate_program, target_root, options)`
- `activate_retarget(instance, candidate_program, target_root, options,
  idempotency_key)`
- `dry_run_fact_migrations(instance, candidate_program, selected_plans)`
- `activate_revision_with_migrations(instance, candidate_program,
  selected_plans, cancellation_policy, confirmations, idempotency_key)`
- `request_provider_cancellation(effect, run, requested_depth)`
- `acknowledge_provider_cancellation(cancellation_request, acknowledgement)`
- `activate_destructive_policy(instance, policy, confirmations,
  idempotency_key)`

Transaction requirements:

- [ ] Retarget, migration, and destructive activation re-run compatibility in
  the same transaction that appends activation records.
- [ ] Idempotency keys are bound to the full activation input, including target
  root, selected migration plans, cancellation depth, destructive policy, and
  confirmation payloads.
- [ ] Projection rebuild reconstructs retargets, migration runs, cancellation
  acknowledgements, and destructive confirmations from the event log.
- [ ] Recovery cannot fabricate migration output, cancellation acknowledgement,
  or destructive confirmation that was not durably recorded.

## Stage 3: Explicit Root Workflow Retargeting

Goal: safely change the root workflow of a non-terminal instance when operators
intentionally choose a new runtime boundary.

Requirements:

- [ ] Retarget is blocked for terminal instances.
- [ ] Retarget dry-run names old root, new root, and selected source bundle.
- [ ] Retarget validates candidate input compatibility against the already
  started instance input or requires an explicit deterministic input mapping.
- [ ] Retarget validates output/failure compatibility for parent invocations.
  If the instance is a child and the parent expects the old contract, retarget
  must be rejected unless a declared terminal-contract adapter is present.
- [ ] Retarget preserves old effects, runs, invocations, facts, diagnostics,
  evidence, and artifacts with original root/version attribution.
- [ ] Future rule stepping uses the new root's rules only after activation.
- [ ] Status and trace show retarget separately from same-root revision.

CLI/API shape candidate:

```sh
whip retarget <instance> candidate.whip --from-root Old --to-root New --dry-run
whip retarget <instance> candidate.whip --from-root Old --to-root New \
  --map-input mapping.json --cancel keep --confirm-retarget-root
```

Open decisions:

- [ ] Whether `--from-root` is required or inferred from the instance active
  root with an activation-time guard.
- [ ] Whether input mappings are declared in source, passed as JSON, or both.
- [ ] Whether terminal-contract adapters share the fact migration declaration
  mechanism or get a separate source form.

## Stage 4: Source-Declared Live Fact Migration

Goal: allow schema-breaking revision only when source declares deterministic
migration plans for active facts.

Requirements:

- [ ] Migration plans are source-declared and compiled into program metadata.
- [ ] Migration expressions run in the pure expression kernel or a restricted
  migration kernel; they cannot call providers or mutate external systems.
- [ ] Each plan declares which old fact class/version it consumes and which new
  fact class/version it produces, retains, or tombstones.
- [ ] Migration dry-run reports exact active fact ids affected by each plan.
- [ ] Activation applies selected plans atomically with revision/retarget.
- [ ] Rejected facts block activation unless a dedicated destructive
  confirmation allows tombstoning or quarantine.
- [ ] Produced migrated facts carry provenance to old fact ids, migration plan
  id, source span, activation event, and target program version.
- [ ] Projection rebuild reproduces migrated fact state from durable migration
  events.

Candidate source sketch:

```whipplescript
migration TicketV1 -> TicketV2 named ticket_v2
  when old.status in ["open", "closed"]
=> {
  record TicketV2 {
    title old.title
    state old.status
  }
}
```

Open decisions:

- [ ] Whether migrations are declared in the old source, candidate source, or a
  paired migration bundle. Candidate source is simplest for activation, while a
  paired bundle may be safer for emergency repair.
- [ ] Whether tombstone/quarantine is a first-class migration action or a
  destructive policy outside the migration language.

## Stage 5: Provider-Specific Cancellation Depth

Goal: make cancellation request behavior explicit per provider without making
the kernel provider-specific.

Cancellation depth vocabulary:

```text
none              provider cannot be interrupted out of band
request_only      runtime records intent; provider observes cooperatively
cooperative_stop  provider supports an API/session stop request
interrupt         provider supports interrupting an active stream/session
force_kill        local harness may terminate a process/session
```

Requirements:

- [ ] Provider/profile bindings declare supported cancellation depth and
  evidence requirements.
- [ ] Revision cancellation policy selects requested depth no deeper than the
  binding/profile allows.
- [ ] Worker cancellation loops are separate from ordinary provider completion
  and write acknowledgement evidence.
- [ ] A cancellation acknowledgement can be:
  - accepted but still running
  - stopped and terminal-cancelled
  - rejected/unsupported
  - lost/timed out
  - raced with completion/failure/timeout
- [ ] Terminal outcome remains single-shot and authoritative.
- [ ] Status distinguishes request pending, acknowledgement received, native
  stop attempted, terminal cancelled, and completed despite cancellation.

## Stage 6: Broader Destructive Policy Confirmations

Goal: add future destructive policies without hiding state loss behind generic
force flags.

Destructive classes that require dedicated confirmation:

- dropping active facts without migration
- tombstoning active facts during migration
- cancelling or dropping blocked effect graphs beyond v0 queued cancellation
- force-killing provider runs
- retargeting a child instance with parent terminal contract adaptation
- applying a migration plan whose dry-run impact no longer matches activation
  impact

Requirements:

- [ ] Every destructive class has a named policy and a named confirmation flag.
- [ ] Dry-run prints and exports an impact hash over all affected durable object
  ids and actions.
- [ ] Activation requires the same impact hash unless the operator asks for a
  new dry-run.
- [ ] Confirmation evidence records operator identity when available, reason,
  policy name, flag name, impact hash, and source of authorization.
- [ ] Generic `--force` is rejected for these policies.

Candidate flag examples:

```sh
--confirm-drop-active-facts <impact-hash>
--confirm-tombstone-facts <impact-hash>
--confirm-force-kill-runs <impact-hash>
--confirm-retarget-parent-contract <impact-hash>
```

## Stage 7: Observability And Operations

Goal: make every follow-up inspectable before broad use.

Status requirements:

- [ ] Active root, active program version, active revision epoch, and retarget
  history are shown together.
- [ ] Active fact migration summary shows pending, applied, rejected, retained,
  tombstoned, and produced counts.
- [ ] Cancellation request summary shows requested depth, provider capability,
  acknowledgement state, and terminal outcome.
- [ ] Destructive confirmation summary shows policy, impact hash, operator, and
  evidence links.

Trace/export requirements:

- [ ] `whip trace --json` includes `workflow_retargets`,
  `fact_migration_runs`, `cancellation_acknowledgements`, and
  `destructive_revision_confirmations` sections.
- [ ] Trace conformance rejects root changes without retarget activation.
- [ ] Trace conformance rejects migrated facts without a migration run.
- [ ] Trace conformance rejects destructive activation without matching
  confirmation evidence.
- [ ] Trace conformance rejects duplicate terminal outcomes after cancellation
  acknowledgement races.

Operations requirements:

- [ ] Runtime operations docs include dry-run-first workflows for retarget and
  migration.
- [ ] Incident bundle guidance includes retarget, migration, cancellation
  acknowledgement, and destructive confirmation exports.
- [ ] Troubleshooting docs explain common rejection diagnostics.

## Stage 8: Tests And E2E

Goal: implement deterministic acceptance before any real-provider expansion.

Required deterministic tests:

- [ ] CLI dry-run for ordinary root mismatch still rejects under `whip revise`.
- [ ] CLI dry-run and activation for explicit retarget succeeds with compatible
  contracts.
- [ ] Retarget idempotency rejects key reuse with different target root or
  mapping.
- [ ] Retarget of child with incompatible parent contract is rejected.
- [ ] Migration dry-run reports incompatible active facts and candidate plans.
- [ ] Migration activation transforms active facts atomically.
- [ ] Migration activation rolls back all changes when one plan rejects.
- [ ] Projection rebuild reconstructs migrated facts and retarget history.
- [ ] Provider cancellation capability `none` leaves request pending with clear
  status.
- [ ] Provider cancellation `cooperative_stop` records acknowledgement and
  terminal outcome exactly once.
- [ ] Late provider completion after cancellation acknowledgement is handled
  according to terminal event idempotency.
- [ ] Destructive policy activation fails without the specific confirmation
  flag.
- [ ] Destructive policy activation fails when the impact hash changed after
  dry-run.
- [ ] Trace conformance catches each unsafe fixture.

Optional real-provider tests:

- [ ] Codex provider cancellation acknowledgement when the supported surface is
  available.
- [ ] Claude provider cancellation acknowledgement when the SDK supports it.
- [ ] Pi provider cancellation acknowledgement through extension/session APIs
  when available.

Real-provider tests must remain opt-in, non-destructive by default, and
environment-gated like existing real-provider checks.

## Proposed First Implementation Slice

1. Add formal and static rejection coverage proving ordinary same-root revision
   still rejects root mismatch unless an explicit retarget command is used.
2. Add the retarget dry-run data model and CLI/API report only, with no
   activation path.
3. Add trace/status placeholders for retarget dry-run artifacts so operators can
   review old root, new root, contract compatibility, active fact impact, and
   cancellation impact before any state mutation exists.

This slice is useful because it preserves v0 semantics while creating the
smallest concrete surface for the riskiest follow-up: changing the root
workflow boundary of a live instance.
