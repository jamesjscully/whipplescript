# `std.telemetry`: read-side event and evidence export

Status: spec drafted 2026-06-14 from package coherence review
([`observability.md`](observability.md)).
Stage: spec -> modeling -> implementation + testing -> review.

> **Reserved-class prerequisites:** NONE. `std.telemetry` contributes no workflow
> construct instances (operator/CLI export surface only; lowering class `none`), so
> it has no construct-system prerequisite and is buildable now. The remaining work
> is the runtime export surface (OTLP exporter provider + export-cursor CLI),
> tracked under the runtime provider stages, not Cycle 2.

## Framing

**Telemetry is a read-side export package over core runtime records.**

WhippleScript execution already records durable events, facts, effects, runs,
artifacts, diagnostics, and evidence. `std.telemetry` does not add workflow
syntax and does not run in rule bodies. It exports those records after the fact
to operator observability systems.

The core shape is:

```text
durable event/evidence log -> cursor-tracked exporter -> telemetry backend
```

Execution must not depend on exporter availability.

## Workflow Surface

There is no `.whip` source surface:

```text
no telemetry declaration block
no rule-body export operation
no workflow-level exporter hook
```

Workflow authors produce ordinary records by running workflows. Operators decide
whether, where, and how those records are exported.

## Operator Surface

`std.telemetry` surfaces through operator configuration, environment variables,
provider bindings, and CLI commands:

```text
whip otel-export
whip telemetry status
whip telemetry reset-cursor
```

The exact CLI can evolve, but the package boundary should remain read-side:
exporters read from the durable store and maintain explicit cursor/checkpoint
state.

## Provider Scope

Initial provider target:

```text
std.telemetry.otlp  OTLP/OpenTelemetry exporter
```

The OTLP provider should honor standard OpenTelemetry environment variables such
as:

```text
OTEL_EXPORTER_OTLP_ENDPOINT
OTEL_EXPORTER_OTLP_PROTOCOL
OTEL_SERVICE_NAME
OTEL_RESOURCE_ATTRIBUTES
OTEL_EXPORTER_OTLP_HEADERS
```

Platform-specific observability backends should be reached through the
OpenTelemetry Collector rather than first-party bespoke exporters.

## Core/Package Boundary

Core owns:

```text
durable event log
fact/effect/run records
artifact and evidence records
diagnostic records
source spans
causal ids and idempotency keys
local trace/evidence JSON shape
provider-run lifecycle ids and terminal status
```

`std.telemetry` owns:

```text
exporter provider contracts
OTLP/OpenTelemetry mapping
cursor/checkpoint state
attribute naming policy
redaction and allowlist policy
export status/report rendering
export failure diagnostics
```

## Construct Graph Contract

`std.telemetry` contributes no workflow construct instances. Its package
manifest may register provider metadata and operator capabilities, but it must
not add rule-body effects, source declarations, direct fact writes, scheduler
hooks, or lifecycle states.

```text
family: metadata / operator_provider
provides: ProviderKind<TelemetryExporter>
requires: durable store read access, export config, redaction policy
lowering class: none for workflow source
runtime entrypoint: operator command / sidecar loop
```

## Capabilities

Telemetry authority is operator/runtime authority, not workflow author authority:

```text
telemetry.export
telemetry.configure
telemetry.cursor.write
```

Importing a package in source must not grant telemetry export access. Export
credentials and allowlists live in operator config.

## Content Policy

Default export is structural only:

```text
ids
kinds
statuses
timings
counts
source metadata
provider kinds
effect kinds
```

Fact field values, prompt bodies, model responses, message text, file contents,
memory contents, and artifact bytes require explicit operator allowlists. An
allowlist may name only declared schema fields; unknown fields are configuration
errors.

## Static And Config Checks

- Workflow checking ignores `std.telemetry`; there is no source syntax to accept.
- Export config must validate endpoint/protocol/header references before export.
- Attribute allowlists must reference declared fields.
- Export cursor state must be scoped to the store, exporter provider, endpoint,
  and mapping version.
- Exporter failure must not block workflow execution or mark events exported.

## Non-Goals

- No workflow rule-body export operation.
- No execution hot-path telemetry hooks.
- No content export by default.
- No provider-specific backend zoo in the first pass.
- No replay or recovery double-export.

## Modeling Notes

- **Failure isolation:** exporter failure does not affect workflow execution.
- **Emit-once:** successful export advances a cursor; failed export does not.
- **Replay safety:** replay/recovery never re-emits telemetry through workflow
  execution.
- **Redaction:** content is excluded unless operator config explicitly allows it.

Detailed evidence, trace, and OpenTelemetry mapping rationale lives in
[`observability.md`](observability.md).
