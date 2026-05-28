# Observability And Evidence

Status: draft

Whippletree must be inspectable. Agent orchestration is only useful if users can
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
```

Every external effect should produce an evidence trail even when it fails.

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
test_output
human_answer
```

Artifact retention is policy-controlled. Sensitive artifacts may be redacted or
stored by hash/reference only.

## Trace Shape

Suggested span hierarchy:

```text
instance
  event processing
    rule firing
      fact record
      effect queued
      dependency edge
        provider run
          artifact capture
          completion event
```

Required correlation fields:

```text
instance_id
program_version
event_id
rule_name
effect_id
run_id
capability_id
agent
provider
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
terminal event appended
projection advanced
```

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
`whippletree.local_trace.v0` through:

```sh
whip trace <instance> --json
whip evidence <instance> --json
```

The export includes events, facts, effects, runs, evidence records, and typed
evidence links. This is the stable local trace shape that a later OpenTelemetry
or hosted exporter should consume.

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
```

## Open Question

We should decide whether the first implementation writes OpenTelemetry spans
directly or keeps a local evidence schema and exports traces later. The local
schema is likely simpler for the first slice, but the fields should be chosen so
export is straightforward.
