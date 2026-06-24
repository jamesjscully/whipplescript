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
coerce_source
coerce_output
loft_command
loft_result
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
what work items are claimed?
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

## `std.telemetry`: read-side export package

The canonical package contract is now [`std-telemetry.md`](std-telemetry.md).
This section records the underlying evidence/trace rationale and the
OpenTelemetry export decision.

`std.telemetry` is the standard package for telemetry export providers and
operator-facing export commands. It does not add workflow syntax. Workflows
produce ordinary events, facts, effects, provider runs, artifacts, diagnostics,
and evidence; telemetry reads those records after the fact.

Core owns:

```text
durable event log
evidence records
local trace/evidence JSON shape
source spans
causal ids
effect/run/provider lifecycle ids
```

`std.telemetry` owns:

```text
exporter providers
OTLP/OpenTelemetry mapping
cursor/checkpoint state
attribute naming policy
redaction and allowlist policy
export status/report rendering
```

Surface:

```text
operator config
standard OpenTelemetry environment variables
provider bindings
CLI commands such as whip otel-export
```

Non-goal: no `telemetry { ... }` workflow declaration, no rule-body export
operation, and no exporter hook on the execution hot path.

## OpenTelemetry export — the ambassador (DECIDED 2026-06-10)

This resolves the former open question (direct spans vs. local schema + later
export) and is the design record for
[`language-ergonomics-tracker.md`](decision-records/language-ergonomics-tracker.md) C8.
Decision: **keep the local evidence store as the source of truth, and export to
OpenTelemetry from it via a log-tailing sidecar.** The local schema above stays
authoritative (it must work offline and in sandboxes); OTel is a read-side
projection of it.

### One target, fanned out at the Collector

Do not integrate observability platforms individually — that is N bespoke
exporters. Emit **OpenTelemetry (OTLP)** as the single target; the OpenTelemetry
Collector routes to any backend (Datadog, New Relic, Honeycomb, Grafana/Tempo,
Splunk, Dynatrace, Elastic, the clouds' native suites). One exporter; the
ecosystem provides the rest, including platforms that do not exist yet. This is
how "support a wide variety of enterprise platforms" is met.

### The ambassador: a log-tailing exporter, not in-process hooks

Telemetry is emitted by a **cursor-tracked sidecar** (`whip otel-export`) that
tails the durable event log and emits OTLP — the event log is the buffer. It
reuses the `TraceRecord` projection already specified in
[Trace Shape](#trace-shape) and [Export Shape](#export-shape): read events ->
build the trace (already implemented) -> map to OTLP. The new code is the OTLP
mapping, the cursor, and the metric aggregations. This gives the three
enterprise-hard properties by construction:

- **Zero hot-path overhead** — execution emits nothing extra; telemetry is
  derived from events already written.
- **Failure isolation** — a down/slow collector never blocks or breaks
  execution; the log persists and the exporter catches up.
- **Emit-once / replay-safe** — the cursor exports each event exactly once;
  recovery and replay never re-emit, so metrics are not double-counted.

An in-process exporter was rejected: it couples export to execution and makes
replay double-counting a hazard.

### Ergonomics: honor the standard OTel environment

Do not invent a config format. Honor the standard OpenTelemetry environment
variables (`OTEL_EXPORTER_OTLP_ENDPOINT`, `OTEL_EXPORTER_OTLP_PROTOCOL`,
`OTEL_SERVICE_NAME`, `OTEL_RESOURCE_ATTRIBUTES`, and `OTEL_EXPORTER_OTLP_HEADERS`
for auth, carried as credential *references* per the provider config model).
Consequences: **authors write nothing** (observability is never in `.whip`
source), **operators set the vars they already know**, and it is **off by
default with zero overhead** when no OTel env is present.

### The three pillars

- **Traces (ship first).** An instance is a trace; the span hierarchy and
  correlation fields are already specified above. Two distinctive properties:
  spans are named after source constructs (`flow.triage.seg0`, `agent.tell`,
  `coerce reviewWork`), so **traces read like the workflow**; and because the
  log is durable, the exporter can emit **retroactive, long-lived traces**
  (a human ask that waited overnight) that exceed any platform's live window.
  Agent turns and `coerce` calls are aligned with OTel **GenAI semantic
  conventions** (`gen_ai.system`, `gen_ai.request.model`,
  `gen_ai.usage.input_tokens`/`output_tokens`) so they appear natively in
  LLM-observability tools — fleet model usage, cost, and latency in the
  operator's existing AI dashboards.
- **Metrics (OTLP push v1; Prometheus pull fast-follow).** Projections over the
  log: effect counts and latency histograms by kind/status, queue depth, and
  the coordination primitives as first-class metrics — **lease contention,
  counter consumption-vs-cap, ledger append rate** ([`coordination.md`](coordination.md)).
  OTLP-push ships first; a Prometheus `/metrics` pull endpoint is a fast-follow.
- **Logs (later).** Events as OTLP log records correlated to their span;
  `whip log` already covers local inspection.

### Semantic conventions (the clarity contract)

The telemetry schema is a documented, versioned contract so dashboards do not
break: a stable `whipplescript.*` convention (`whipplescript.instance_id`,
`whipplescript.rule`, `whipplescript.effect.kind`/`status`,
`whipplescript.provider`, ...) reusing the correlation fields above, plus
**version-pinned** `gen_ai.*` alignment (tracking a still-stabilizing OTel
spec).

### Content policy: structural by default, operator allowlist for the rest

The firm default protecting PII and cardinality: **export structural telemetry
only — ids, kinds, statuses, timings, counts — never content (prompt bodies,
fact field values).** Projecting fact values would leak sensitive data and
explode cardinality. Richer business-dimension attributes (e.g. `customer_id`)
are **opt-in via an operator-config allowlist** — not a source annotation, so
the operator owns the cardinality/compliance budget and source stays clean. The
allowlist may only name declared schema fields (an unknown field is a config
error); this extends the existing artifact-redaction discipline.

### Modeling notes

- Emit-once: a crash mid-export resumes from the cursor without duplication.
- Replay safety: recovery/replay does not re-emit telemetry; metrics counted
  once.
- Failure isolation: the execution trace is independent of exporter
  availability.
- Redaction: no content is emitted unless explicitly allowlisted; allowlisting
  an unknown field is rejected.
