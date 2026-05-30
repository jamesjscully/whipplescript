# Observability And Evidence

Status: draft

WhippleScript must be inspectable. Agent orchestration is only useful if users can
understand why work started, what authority it used, what it observed, and why
it stopped.

This document deliberately targets tracing and observability systems. The first
implementation can be SQLite/file-based, but the data model should map cleanly
to OpenTelemetry-like traces later.

## Evidence Store

The evidence store records artifacts and causal links. Artifacts are bytes or
external references. Evidence is the typed explanation of why the artifact
exists and how it relates to the run.

```text
event
rule firing
fact record
effect
effect dependency
provider run
artifact
diagnostic
human answer
capability decision
skill/context injection
assertion result
provider failure
workflow revision
cancellation request
```

Every external effect should produce an evidence trail even when it fails.
Every assertion evaluation should produce an evidence trail even when it passes,
because passing assertions are part of reproducible validation.

Evidence records should be stable enough to support:

```text
local CLI inspection
agent-readable status summaries
hosted UI timelines
trace export
audit review
bug reproduction
```

## Artifact Types

```text
stdout
stderr
provider_metadata
prompt
model_request
model_response
baml_source
baml_output
loft_command
loft_result
thoth_brief
thoth_verify
memory_query
memory_result
patch
revision_report
cancellation_report
test_output
human_answer
failure_transcript
assertion_read_set
trace_json
```

Artifact retention is policy-controlled. Sensitive artifacts may be redacted or
stored by hash/reference only.

## Trace Shape

Suggested span hierarchy:

```text
instance
  revision activation
    compatibility diagnostics
    cancellation impact
    cancellation request
  event processing
    rule firing
      fact record
      effect queued
      dependency edge
        provider run
          provider startup/auth/tool/transport/timeout event
          artifact capture
          completion event
    assertion evaluation
      assertion pass/fail/error event
      diagnostic
```

Required correlation fields:

```text
instance_id
program_version
revision_id
revision_epoch
event_id
rule_name
effect_id
run_id
capability_id
agent
provider
assertion_id
diagnostic_id
artifact_id
idempotency_key
source_span
```

Evidence should be emitted for kernel transitions, not merely provider outputs:

```text
event appended
rule evaluated
rule committed
fact recorded/projected
effect queued
dependency edge created
effect blocked
effect claimed
run started
run completed
provider startup failed
provider auth failed
provider tool failed
provider transport failed
provider timed out
assertion evaluated
assertion passed
assertion failed
assertion errored
revision dry-run evaluated
revision activated
old-version effect terminal-cancelled
old-version effect cancellation requested
terminal event appended
projection advanced
```

Provider failure evidence must identify the failed boundary:

```text
binding
auth
workspace
startup
submit
stream
tool
transport
timeout
artifact
terminal_append
```

Provider failure evidence links the effect, run, failure event, diagnostic,
source span when available, and artifacts such as stderr, provider metadata, or
failure transcript. Timeout evidence must include the timeout boundary, elapsed
duration, configured deadline, and whether retry is allowed.

Assertion evidence links the assertion source span, expression text, projection
read set, result event, diagnostic, and optional compact actual/expected JSON.
`assertion.failed` records a deterministic false result. `assertion.errored`
records evaluator/type/missing-data errors. Neither result may be represented
only as process exit status or stderr.

Revision evidence links the activation event, revision id/epoch, old and new
program versions, compatibility diagnostics, candidate source-bundle identity,
selected root workflow, cancellation policy, impacted effects, and any
terminal-cancel or cancellation-request records created by the activation.
Dry-run evidence, when persisted as an operator artifact, must be clearly marked
non-mutating and must not be confused with an activation event.

Cancellation request evidence links the request event, effect, active run or
lease when present, provider/harness acknowledgement when available, and the
eventual terminal outcome. A cancellation request is evidence of intent to stop
work, not evidence that the effect has reached a terminal cancellation state.

## Export Shape

The first implementation should store evidence locally, but avoid painting
itself away from tracing systems. A later exporter should be able to map:

```text
instance      -> trace
event process -> span
rule firing   -> span/event
effect        -> span
provider run  -> span
artifact      -> span event or linked blob
diagnostic    -> span event
```

OpenTelemetry is a plausible export target, but the local store remains the
source of truth for v0 because it must work offline and inside local agent
sandboxes.

Current implementation exports a local JSON trace with schema
`whipplescript.local_trace.v0` through:

```sh
whip trace <instance> --json
whip evidence <instance> --json
```

The export includes events, facts, effects, runs, evidence records, and typed
evidence links. This is the stable local trace shape that a later OpenTelemetry
or hosted exporter should consume.

The JSON trace must also expose diagnostics and artifacts as first-class
sections:

```text
schema
instance
program_version
revisions
events
facts
effects
workflow_invocations
effect_cancellation_requests
runs
artifacts
diagnostics
evidence
links
```

Event, diagnostic, artifact, and evidence records in the export must preserve
`correlation_id`, `causation_id`, `idempotency_key`, and `source_span` when
present. Provider failure records must expose structured startup/auth/tool/
transport/timeout details rather than flattening them into a message string.
Assertion pass/fail/error records must expose assertion id, source span, read
set, result, diagnostics, and evidence links.
Revision records must expose revision id, epoch, old/new program versions,
activation event, cancellation policy, compatibility diagnostics, cancellation
impact, and linked evidence. Effects and runs must include their
`program_version_id` and `revision_epoch` so an operator can distinguish old
work completing after a revision from work created by the active version.

Future workflow revision follow-ups add distinct observability records rather
than overloading ordinary revision fields. Trace and evidence exports for those
features must expose:

```text
workflow_retargets
fact_migration_runs
cancellation_acknowledgements
destructive_revision_confirmations
```

Retarget records must name old/new roots, old/new program versions, parent
invocation compatibility, input mapping or mapping absence, active fact impact,
and cancellation impact. Migration records must list consumed, produced,
retained, tombstoned, and rejected facts with migration plan ids and source
spans. Cancellation acknowledgements must include provider capability depth,
provider/harness response, timeout state, evidence, and eventual terminal
outcome when known. Destructive confirmations must include policy name,
confirmation flag, dry-run impact hash, operator identity when available,
reason, and evidence links.

Trace conformance for these follow-ups must reject root changes without a
retarget activation, migrated facts without a migration run, destructive
activation without matching confirmation evidence, and duplicate terminal
outcomes after provider cancellation acknowledgement races.

Trace export is read-only and idempotent. Re-running `whip trace <instance>
--json` over an unchanged store must produce the same logical records and stable
ids, aside from allowed formatting/order normalization documented by the CLI.

## Status UX

The core status view should answer:

```text
what programs are deployed?
what instances are running?
what effects are queued/running/failed?
what agents are active?
what human questions are pending?
what Loft issues are claimed?
what recent failures/blockers exist?
what capabilities or policies blocked work?
what evidence changed since last view?
which revision is active?
which version/epoch created this effect or run?
which effects were cancelled or requested to cancel by revision?
```

Target commands:

```sh
whip ps
whip status <instance>
whip trace <instance>
whip evidence <run-or-effect>
whip failures
whip blockers
```

The UX should also provide compact views for agents:

```text
latest run per logical agent
queued/running/failed effects
pending inbox items
recent capability blocks
recent Loft claims
recent evidence summaries
active revision plus recent revision history
cancellation requests with terminal outcome when known
```

`whip status --json <instance>` must include the active revision epoch, active
program version, revision history, and a cancellation summary. `whip trace
<instance> --json` must include revision activation and cancellation-request
records in event order so trace conformance can reject impossible sequences such
as old-version rule commits after activation or fabricated terminal
cancellation for running effects.

## Open Question

We should decide whether the first implementation writes OpenTelemetry spans
directly or keeps a local evidence schema and exports traces later. The local
schema is likely simpler for the first slice, but the fields should be chosen so
export is straightforward.
