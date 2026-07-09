# Runtime Store

Status: draft

The runtime store is the durable substrate for the control plane. The first
implementation should use SQLite. The conceptual model should not depend on
SQLite-specific behavior beyond transactional durability and useful indexes.

## Logical Tables

```text
programs
program_versions
instances
instance_revisions
events
facts
effects
effect_dependencies
workflow_invocations
effect_cancellation_requests
runs
artifacts
evidence
capability_bindings
profiles
skills
package registrations
inbox_items
leases
diagnostics
```

Implementation note: package registrations are stored as package/provider
registration records. Legacy manifest inputs may still contain a `plugin_id`
field while compatibility fixtures are being retired, but the logical and
physical store model is package-based.

All durable objects that can be surfaced to users should carry enough
correlation to reconstruct a local trace:

```text
instance_id
program_id?
program_version_id?
revision_id?
revision_epoch?
causation_id?
correlation_id?
idempotency_key?
source_span?
```

## Programs

`programs` stores user-facing names. `program_versions` stores immutable
compiled artifacts.

Required fields:

```text
program_id
name
version_id
source_hash
ir_hash
compiler_version
created_at
declared_capabilities
declared_profiles
declared_skills
declared_schemas
analysis_summary
generated_artifacts
artifact_root
```

## Instances

Required fields:

```text
instance_id
program_id
version_id                 # active program-version cache
active_revision_epoch
status
created_at
started_at
updated_at
completed_at?
input_json
last_event_id?
last_error?
```

`instances.version_id` is a denormalized cache of the active program version.
The canonical append-only revision history lives in `instance_revisions`.

## Instance Revisions

Every instance starts at revision epoch `0`, which points at the program version
used at start. A later workflow revision appends another row with the next
monotonic epoch and updates the instance active-version cache.

Required fields:

```text
revision_id
instance_id
epoch
from_version_id?
to_version_id
activated_by_event_id?
activation_policy_json
cancellation_policy          # keep | cancel_queued | request_running
status                       # active | superseded | rejected
requester?
reason?
created_at
activated_at?
diagnostic_ids?
evidence_ids?
```

The `(instance_id, epoch)` pair is unique. Replaying revision activation events
must reconstruct the same active epoch, active version, diagnostics, evidence,
and cancellation requests.

## Events

Events are append-only:

```text
event_id
instance_id
sequence
type
payload_json
time
source
causation_id?
correlation_id?
idempotency_key?
diagnostic_ids?
evidence_ids?
```

The `(instance_id, sequence)` order is the canonical per-instance event order.
`idempotency_key` is unique within an instance for externally replayable or
recovery-replayable events. The store must reject duplicate terminal provider
events and duplicate assertion result events with the same key.

Provider and assertion event payloads are part of the stable store contract.
Provider terminals use one canonical event:

```text
effect.terminal
```

Terminal provider failures must include or link a durable diagnostic record with
`effect_id`, optional `run_id`, `code`, `message`, `idempotency_key`,
`correlation_id`, `evidence_ids`, optional `artifact_ids`, and optional
`source_span`. The `effect.terminal` payload also carries provider failure
metadata such as `provider`, `phase`, `error_kind`, `recoverable`, and optional
`retry_after`. Attempt counters are run/effect policy metadata, not required
diagnostic fields in the current store schema.

Assertion result events use:

```text
assertion.passed
assertion.failed
assertion.errored
```

Assertion payloads must include `assertion_id`, `assertion_text`, `result`,
`program_version_id`, optional `rule_name`, `source_span`, `read_set`, optional
`actual_json`, optional `expected_json`, optional `error_code`, optional
`message`, `diagnostic_ids`, `evidence_ids`, `correlation_id`, and
`idempotency_key`.

Revision events use:

```text
workflow.revision_activated
effect.cancellation_requested
```

`workflow.revision_activated` payloads must include `revision_id`,
`instance_id`, `from_version_id`, `to_version_id`, `from_epoch`, `to_epoch`,
`activation_policy`, `cancellation_policy`, `terminal_cancel_effects`, and
`request_cancel_effects`.
`effect.cancellation_requested` payloads must include `request_id`, `effect_id`,
optional `revision_id`, `reason`, and `requested_by`.

## Facts

Facts are the current materialized state:

```text
fact_id
instance_id
program_version_id?
revision_epoch?
name
key
value_json
source_event_id?
source_rule?
source_effect_id?
source_run_id?
schema_id
provenance_class
external_system?
external_id?
correlation_id?
created_at
updated_at
```

`key` is a stable identity for set-like facts. Facts that need multiplicity must
include a unique key in their value. Accidental duplicate facts should not be a
semantic feature.

Fact provenance is defined in [fact-provenance.md](fact-provenance.md).

## Effects

Effects are durable outbox records:

```text
effect_id
instance_id
program_version_id
revision_epoch
kind
target
resolved_target_json?
resolved_provider_binding_id?
input_json
status
created_by_rule
created_by_event_id?
source_span?
correlation_id
idempotency_key
required_capabilities
profile?
created_at
updated_at
```

Effect status:

```text
queued
blocked_by_dependency
running
completed
failed
timed_out
cancelled
blocked_by_capability
blocked_by_profile
blocked_by_capacity
```

The store may compute `claimable` from `queued` effects whose dependencies,
policy checks, retry windows, and capacity constraints are satisfied.

Effects created before a workflow revision keep their original
`program_version_id`, `revision_epoch`, resolved target, provider binding,
profile, and capability attribution. New effects created after revision use the
active program version and revision epoch at rule-commit time.

## Effect Cancellation Requests

Running cancellation is represented as a separate durable request rather than a
terminal effect status. Status views may display `cancel_requested`, but the
effect's terminal status remains one of the normal terminal outcomes.

Required fields:

```text
cancellation_request_id
effect_id
instance_id
revision_id
revision_epoch
requested_by_event_id
requester?
reason?
idempotency_key
status                       # requested | terminal
created_at
terminal_event_id?
diagnostic_ids?
evidence_ids?
```

`idempotency_key` is unique for the effect/revision pair. Replaying recovery
must not create duplicate cancellation requests or convert a request into a
cancelled `effect.terminal` event without provider, harness, timeout, or
recovery evidence.

## Effect Dependencies

Effect dependencies are durable edges inside one rule-produced effect graph:

```text
dependency_id
instance_id
upstream_effect_id
downstream_effect_id
predicate
created_by_rule
created_at
```

Allowed predicates:

```text
succeeds
fails
completes
```

Source order never creates dependency edges. Edges exist only when the source
program expresses dependency, such as an `after` block.

## Workflow Invocations

Workflow invocations are durable parent-to-child links created by
`workflow.invoke` effects.

Required fields:

```text
invocation_id
parent_instance_id
parent_effect_id
parent_program_version_id
parent_revision_epoch
child_instance_id?
child_program_version_id?
child_revision_epoch?
target_workflow
input_json
status
terminal_event_id?
source_span?
correlation_id?
idempotency_key?
created_at
updated_at
```

Parent and child instances may revise independently. The invocation row
preserves the parent effect's original program version and revision epoch, and
links to the child instance's starting version/epoch when the child is created.
Revision does not rewrite invocation links or project child terminal state more
than once.

## Runs

A run is one provider attempt to execute an effect:

```text
run_id
effect_id
instance_id
program_version_id
revision_epoch
provider
worker_id
status
started_at
completed_at?
exit_code?
summary?
metadata_json
attempt
idempotency_key
correlation_id
failure_stage?
error_code?
retryable?
diagnostic_ids?
evidence_ids?
```

Multiple runs may exist for one effect when retry policy permits it.
Each retry creates a new run with an incremented `attempt` while preserving the
effect id, effect idempotency key, and correlation id. A run is recoverable
until it has a terminal event linked by evidence.

## Artifacts

Artifacts hold provider outputs. Evidence records hold causal relationships.
The first implementation may store both in SQLite plus files on disk, but the
model should export cleanly to tracing systems later.

```text
artifact_id
run_id
kind
path
content_hash?
mime_type?
summary?
redaction_state?
created_at
```

Examples:

```text
stdout
stderr
provider_metadata
coerce_request
coerce_response
failure_transcript
trace_json
patch
report
```

## Evidence

Evidence links the durable history:

```text
evidence_id
instance_id
kind
subject_type
subject_id
causation_id?
correlation_id?
summary?
metadata_json
created_at
```

Examples:

```text
rule_fired
fact_recorded
effect_queued
effect_dependency_created
capability_decision
skill_attached
artifact_written
human_answered
policy_blocked
provider_failure
assertion_result
workflow_revision
cancellation_request
diagnostic_link
source_span_link
```

Evidence is not workflow truth. It is an observability layer over truth in the
event log, fact projection, effect outbox, runs, artifacts, and external
kernels.

Active fact projection rows include a nullable `consumed_at` timestamp. A
`rule.committed` payload records both produced facts and `consumed_facts`; replay
must insert produced facts and then mark the referenced active facts consumed in
event order. Default fact projection reads filter out consumed rows, while audit
and evidence views may still show them as historical facts.

Evidence for diagnostics should link:

```text
diagnostic_id
event_id?
fact_id?
effect_id?
run_id?
artifact_id?
assertion_id?
source_span?
evidence_ids?
```

This makes a failure durable even when the readable transcript is stored as an
artifact or redacted. Source spans must identify source path or bundle member,
byte/line/column range, and the syntactic construct that caused the diagnostic.

## Diagnostics

Diagnostics are durable, queryable records for compile, stepper, assertion,
provider, harness, policy, and projection failures:

```text
diagnostic_id
instance_id?
program_id?
program_version_id?
severity             # info | warning | error
code
message
source_span?
subject_type?
subject_id?
event_id?
effect_id?
run_id?
assertion_id?
evidence_ids?
artifact_ids?
created_at
```

Assertion failures and assertion evaluation errors must produce diagnostics
linked to their assertion event and source span. Provider startup/auth/tool/
transport/timeout failures must produce diagnostics linked to the run/effect,
failure event, evidence, and any failure transcript artifact.

## Transactions

A rule commit is atomic:

```text
consume/update facts
produce facts
append derived events if any
enqueue effects
persist effect dependency edges
record diagnostics
advance instance cursor
```

Provider execution is never part of the rule commit. Providers interact through
the effect queue and append completion events.

Provider terminal updates are atomic at the store boundary:

```text
append provider/effect terminal or retry event
update run status and error metadata
update effect status and retry metadata
record diagnostics
record evidence links
record artifacts or artifact references
derive completion/failure facts when applicable
```

If any part of this transaction fails, the run remains recoverable and the
worker must not report completion out of band.

Workflow revision activation is atomic at the store boundary:

```text
append revision activation event
insert instance revision row
update instance active-version cache
record compatibility diagnostics
record evidence links
terminal-cancel queued/blocked/claimable old-version effects when requested
insert cancellation requests for running old-version effects when requested
```

If any part of revision activation fails, no active-version change,
terminal-cancel, or cancellation request from that activation may persist.
`whip revise --dry-run` uses the same compatibility and impact analysis but
must not write events, rows, diagnostics, evidence, or effect state.

## Recovery

On startup, the control plane recovers:

- running effects whose leases expired
- instances with unprocessed events
- provider runs that exited without recorded completion
- diagnostics for failed rule commits
- active revision caches from revision activation events
- effect cancellation requests and terminal cancellation idempotency

Recovery must not duplicate effects with the same idempotency key.
It also must not duplicate provider terminal events, assertion result events,
diagnostics, evidence links, artifacts, or derived facts when their
idempotency/correlation keys already exist. Retrying a provider effect must
reuse the original effect idempotency key for external de-duplication and create
a distinct run idempotency key for the new attempt.

Recovery must preserve revision event order. It must not let an old-version rule
commit create new effects after a later revision epoch is active. It must also
preserve old-version effect attribution when those effects complete after a
revision.
