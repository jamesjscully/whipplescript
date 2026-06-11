# Reporting Contract

Status: draft v0 contract

WhippleScript reports should make source intent, durable runtime state, and
validation results inspectable without introducing a second execution model.
This is the reporting lesson to borrow from Cucumber Messages: stable
machine-readable envelopes, source metadata, and explicit result records rather
than free-text step matching.

## Scope

Current report surfaces:

- `whip --json check <workflow.whip>...`
- `whip --json compile <workflow.whip>`
- `whip --json dev <workflow.whip>`
- `whip dev <workflow.whip> --stream ndjson`
- `whip --json accept <fixture.json>`
- JSON inspection commands such as `facts`, `effects`, `evidence`,
  `diagnostics`, and `trace`

Draft JSON Schema files:

- [`report-schemas/check_report_v0.schema.json`](report-schemas/check_report_v0.schema.json)
- [`report-schemas/compile_report_v0.schema.json`](report-schemas/compile_report_v0.schema.json)
- [`report-schemas/dev_report_v0.schema.json`](report-schemas/dev_report_v0.schema.json)
- [`report-schemas/dev_stream_v0.schema.json`](report-schemas/dev_stream_v0.schema.json)
- [`report-schemas/local_trace_v0.schema.json`](report-schemas/local_trace_v0.schema.json)
- [`report-schemas/acceptance_fixture_v0.schema.json`](report-schemas/acceptance_fixture_v0.schema.json)
- [`report-schemas/acceptance_report_v0.schema.json`](report-schemas/acceptance_report_v0.schema.json)

Run [`../scripts/check-report-schemas.sh`](../scripts/check-report-schemas.sh)
to generate provider-language `check`, `compile`, `dev`, `trace --check`,
`dev --stream ndjson`, and `accept` reports and validate them against these
schemas.

Reports are descriptive. They must not change rule readiness, effect creation,
provider routing, table seeding, retries, authorization, or assertion
semantics.

## Shared Source Metadata

`check`, `compile`, and `dev` include `source_metadata`:

```json
{
  "tags": [
    {
      "name": "acceptance",
      "target_kind": "assertion",
      "target": "a219774c2ee6f69f",
      "source_span": {"start": 1, "end": 12}
    }
  ],
  "descriptions": [
    {
      "value": "Codex completes both assigned language tasks",
      "target_kind": "assertion",
      "target": "a219774c2ee6f69f",
      "source_span": {"start": 13, "end": 72}
    }
  ],
  "targets": {
    "assertion:a219774c2ee6f69f": {
      "target_kind": "assertion",
      "target": "a219774c2ee6f69f",
      "tags": ["acceptance"],
      "description": "Codex completes both assigned language tasks"
    }
  }
}
```

Target keys use `<kind>:<target>`. Current target kinds are `workflow`,
`table`, `rule`, and `assertion`.

## `check --json`

`check --json` emits an array with one report per input path. Successful entries
use:

```json
{
  "schema": "whipplescript.check_report.v0",
  "path": "examples/provider-language-e2e.whip",
  "status": "ok",
  "workflow": "ProviderLanguageE2E",
  "source_hash": "...",
  "ir_hash": "...",
  "snapshot": "...",
  "source_metadata": {}
}
```

Diagnostic entries use `"status": "error"` and include an `error.kind` of
`"io"` or `"diagnostics"`. Parser diagnostics include message, suggestion, and
source span offsets.

## `compile --json`

`compile --json` emits one compiled-program report:

```json
{
  "schema": "whipplescript.compile_report.v0",
  "path": "examples/provider-language-e2e.whip",
  "workflow": "ProviderLanguageE2E",
  "source_hash": "...",
  "ir_hash": "...",
  "snapshot": "...",
  "source_metadata": {}
}
```

## `dev --json`

`dev --json` emits one validation report after the local run loop:

```json
{
  "schema": "whipplescript.dev_report.v0",
  "instance_id": "inst_...",
  "workflow": "ProviderLanguageE2E",
  "source_metadata": {},
  "steps": [],
  "workers": [],
  "diagnostics": [],
  "provider_runs": {
    "summary": {"total": 12, "artifact_count": 12},
    "groups": [
      {"provider": "fixture", "status": "completed", "count": 12, "artifact_count": 12}
    ]
  },
  "provider_artifacts": {
    "summary": {"total": 12},
    "groups": [
      {"kind": "stdout_ref", "mime_type": "text/plain", "count": 6},
      {"kind": "transcript_ref", "mime_type": "text/plain", "count": 6}
    ],
    "items": [
      {
        "artifact_id": "art_...",
        "run_id": "run_...",
        "kind": "transcript_ref",
        "mime_type": "text/plain",
        "content_hash": "sha256:..."
      }
    ]
  },
  "provider_evidence": {
    "summary": {"total": 37},
    "groups": [
      {"kind": "agent.turn.provider", "subject_type": "run", "count": 6},
      {"kind": "baml.coerce.provider", "subject_type": "run", "count": 6},
      {"kind": "skills.injected", "subject_type": "run", "count": 6},
      {"kind": "rule.committed", "subject_type": "rule_commit", "count": 19}
    ],
    "items": [
      {
        "evidence_id": "ev_...",
        "kind": "agent.turn.provider",
        "subject_type": "run",
        "subject_id": "run_...",
        "causation_id": "eff_...",
        "correlation_id": "key_...",
        "summary": "Fixture provider completed agent turn"
      }
    ]
  },
  "assertion_filter": {
    "include_tags": ["acceptance"],
    "exclude_tags": [],
    "total": 6,
    "selected": 6
  },
  "executable_spec": {
    "status": "passed",
    "summary": {"total": 6, "passed": 6, "failed": 0, "error": 0},
    "tags": []
  },
  "assertions": []
}
```

`diagnostics` is the durable diagnostic list for the dev instance at the end of
the run. It uses the same object shape as `diagnostics <instance> --json`, so
provider boundary failures, policy denials, assertion failures, guard errors,
and terminal diagnostics can be inspected from the single dev report.

`provider_runs` summarizes durable provider runs by provider/status, including
artifact counts for each group. `provider_artifacts` summarizes artifact
metadata by kind and MIME type and includes compact artifact item links:
artifact id, run id, kind, MIME type, and content hash. It does not expose
artifact paths or content.
`provider_evidence` summarizes evidence metadata by kind and subject type and
includes compact evidence item links: evidence id, kind, subject type/id,
causation id, correlation id, and summary. It does not expose evidence metadata
payloads. Use the `runs`, `artifacts`, and `evidence` inspection commands when
full individual run ids, lifecycle details, artifact metadata, or evidence
metadata payloads are needed.

Assertion tag filters select which source assertions are evaluated and reported.
They do not skip rules, effects, providers, or table seeding. Exclusion wins
when both include and exclude filters match an assertion.

Assertion reports include `target_id`, `event_id`, `expr`, `reads`, `tags`,
`status`, `passed`, `actual`, `actual_values`, and `expected`. They may
include `description`, `diagnostic_ids`, `failure_reason`, `error`, and
`source_span`.
`event_id` links the report row to the durable `assertion.passed`,
`assertion.failed`, or `assertion.errored` event. Failed and errored
assertions also include `diagnostic_ids` for the diagnostic records produced
from that assertion result.

## `dev --stream ndjson`

`dev --stream ndjson` emits one compact JSON envelope per line while the local
dev loop runs. Each envelope uses:

```json
{
  "schema": "whipplescript.dev_stream.v0",
  "sequence": 0,
  "event": "dev.started",
  "data": {}
}
```

Current event names are `dev.started`, `dev.events`, `dev.step`, `dev.worker`,
`dev.idle`, `dev.assertions`, and `dev.report`. `dev.events` emits batches of
newly persisted raw runtime events using the same event object shape as
`whip log --json`; batches include `after_sequence`, `count`, and `events`.
`dev.assertions` emits the compact executable-spec assertion summary after
assertion events and diagnostics have been persisted. The final `dev.report`
event embeds the same
`whipplescript.dev_report.v0` object emitted by `dev --json` in its `data`
field. Stream mode is descriptive and does not change runtime semantics.

Each assertion read links the assertion to the deterministic projection it
checked:

```json
{
  "kind": "effect",
  "head": "kind agent.tell",
  "guard": "status == completed",
  "source": "effect:kind agent.tell where status == completed",
  "match_count": 6,
  "matches": [
    {
      "id": "eff_...",
      "name": "agent.tell",
      "status": "completed"
    }
  ]
}
```

`kind` is currently `fact` or `effect`. `source` is a stable source-like label
for grouping reports, while `head` and `guard` are structured fields for
machine consumers. `matches` links the read to concrete active facts or effects.
Fact matches include fact id, name, key, provenance class, and source span when
available. Effect matches include effect id, kind, status, and prompt content
type when the effect input declares `prompt_content_type`.

## Acceptance Reports

`whip --json accept <fixture.json>` emits one
`whipplescript.acceptance_report.v0` object:

```json
{
  "schema": "whipplescript.acceptance_report.v0",
  "fixture": "examples/provider-language-e2e.accept.json",
  "workflow": "examples/provider-language-e2e.whip",
  "passed": true,
  "failures": [],
  "observed": {
    "summary": {"facts": 18, "effects": 12},
    "facts": [{"name": "LanguageE2EResult", "count": 6}],
    "effects": [
      {"kind": "agent.tell", "status": "completed", "count": 6}
    ],
    "actions": [
      {"type": "pause", "event_id": "evt_...", "sequence": 2},
      {"type": "resume", "event_id": "evt_...", "sequence": 3}
    ],
    "source_metadata": {
      "summary": {"targets": 3},
      "targets": [
        {
          "key": "workflow:ProviderLanguageE2E",
          "target_kind": "workflow",
          "target": "ProviderLanguageE2E",
          "tags": ["fixture", "acceptance"],
          "description": "Fixture-backed provider x language acceptance workflow"
        }
      ]
    },
    "runs": {
      "summary": {"total": 12, "artifact_count": 12},
      "groups": [
        {"provider": "fixture", "status": "completed", "count": 12, "artifact_count": 12}
      ]
    },
    "artifacts": {
      "summary": {"total": 12},
      "groups": [
        {"kind": "stdout_ref", "mime_type": "text/plain", "count": 6},
        {"kind": "transcript_ref", "mime_type": "text/plain", "count": 6}
      ]
    },
    "evidence": {
      "summary": {"total": 37},
      "groups": [
        {"kind": "agent.turn.provider", "subject_type": "run", "count": 6},
        {"kind": "baml.coerce.provider", "subject_type": "run", "count": 6},
        {"kind": "skills.injected", "subject_type": "run", "count": 6},
        {"kind": "rule.committed", "subject_type": "rule_commit", "count": 19}
      ]
    },
    "inbox": {
      "summary": {"total": 1},
      "groups": [
        {"status": "pending", "severity": "normal", "count": 1}
      ]
    },
    "trace": {
      "summary": {"events": 61, "abstract_events": 24},
      "groups": [
        {"type": "effect_created", "count": 12},
        {"type": "effect_terminal", "count": 12}
      ],
      "items": [
        {
          "sequence": 4,
          "event": {
            "type": "run_started",
            "run_id": "run_...",
            "effect_id": "eff_..."
          }
        }
      ],
      "conformance": {"ok": true}
    },
    "diagnostics_by_code": [],
    "assertion_reads": [
      {
        "kind": "effect",
        "head": "kind agent.tell",
        "source": "effect:kind agent.tell where status == completed",
        "match_count": 6,
        "matches": [
          {
            "name": "agent.tell",
            "status": "completed",
            "prompt_content_type": "markdown",
            "provenance_class": null,
            "trace_items": 24,
            "evidence_items": 12,
            "trace_sequences": [2, 4, 5],
            "evidence_ids": ["ev_..."],
            "count": 6
          }
        ]
      }
    ],
    "executable_spec": {
      "status": "passed",
      "summary": {"total": 6, "passed": 6, "failed": 0, "error": 0},
      "tags": [
        {
          "tag": "acceptance",
          "status": "passed",
          "summary": {"total": 6, "passed": 6, "failed": 0, "error": 0}
        }
      ]
    }
  },
  "dev_report": {"schema": "whipplescript.dev_report.v0"}
}
```

`observed.summary` reports total final active facts and effects. The grouped
`observed.facts` and `observed.effects` arrays summarize final store state by
fact class and effect kind/status. `observed.runs` summarizes provider run
counts and artifact counts by provider/status. `observed.artifacts` summarizes
artifact metadata by kind and MIME type and includes compact artifact item
links without exposing paths or content.
`observed.evidence` summarizes evidence metadata by kind and subject type and
includes the same compact evidence item links as `provider_evidence`, without
exposing evidence metadata payloads. `observed.inbox` summarizes human inbox
items by status and severity. `observed.actions` records fixture control-plane
setup actions that were applied to the started instance. `observed.source_metadata`
summarizes source metadata targets from the embedded dev report.
`observed.assertion_reads` summarizes deterministic assertion reads and concrete
match metadata such as prompt content type. Effect match groups also include
compact `trace_items` and `evidence_items` counts plus `trace_sequences` and
`evidence_ids` links for matched effects. These identifiers link to
`observed.trace.items` and `observed.evidence.items` without embedding raw
payloads in the assertion read summary.
`observed.trace` summarizes raw event count, reconstructed abstract trace event
groups, compact abstract trace items, and trace conformance without embedding
the full raw store event log.
`observed.diagnostics_by_code` and `observed.executable_spec` give compact
summaries of the embedded dev report for fixture failure messages and CI
dashboards. This is acceptance-report metadata only; the embedded `dev_report`
remains the ordinary dev-loop report contract.

Acceptance fixtures validate selected parts of that final report with
`expect.dev_status`, `expect.status`, `expect.source_metadata`,
`expect.diagnostics`, `expect.diagnostics_by_code`, `expect.actions`,
`expect.assertions`, `expect.assertion_tags`, `expect.assertion_untagged`,
`expect.assertion_reads`, `expect.summary`, `expect.facts`, `expect.effects`,
`expect.runs`, `expect.artifacts`, `expect.evidence`, `expect.inbox`, and
`expect.trace`. Fixture `input` is ordinary workflow start input and uses the
same validation path as `run` and `dev`. Fixture `actions` are existing
control-plane transitions applied before the dev loop. Fixture and expectation
shapes are validated before starting a workflow; wrong-typed expectation fields
are rejected instead of being treated as absent. `expect.assertion_reads`
entries must include at least one selector: `source`, `kind`, `head`, or
`guard`.

`accept` is deliberately single-fixture in v0. Test suites should invoke the
command once per fixture with isolated stores until suite-level runtime id
namespacing is specified.

## Table Fact Provenance

Facts seeded from `table` declarations use:

```json
{
  "provenance_class": "table",
  "source_span": {
    "path": "examples/provider-language-e2e.whip",
    "start": 2132,
    "end": 2363,
    "construct": "table_row"
  }
}
```

The span points to the source row. Table provenance is report metadata; the
runtime still commits ordinary durable facts.

## Stability And Open Work

Stable enough for current v0 tests:

- source metadata shape for `check`, `compile`, and `dev`
- report schema/version identifiers for `check`, `compile`, `dev`, `trace`, and
  `accept`
- draft JSON Schema files for `check`, `compile`, `dev`, `trace`,
  `dev --stream`, and acceptance reports/fixtures
- generated provider-language reports validate against those schema files
- NDJSON stream envelopes for `dev --stream ndjson`, including raw `dev.events`
  runtime event batches
- acceptance report pass/fail, mismatch failures, embedded dev report, observed
  fact/effect count summaries, observed provider run/artifact counts, observed
  evidence counts, observed diagnostic counts, and observed executable-spec
  summaries
- acceptance suite isolation policy: v0 accepts one fixture path, with suite
  runners responsible for isolated stores
- dev report diagnostics for provider, policy, assertion, guard, and terminal
  failures
- assertion filter report counts
- `dev` assertion and `executable_spec` summaries
- assertion deterministic read links with concrete fact/effect matches and
  grouped trace/evidence link counts plus compact link arrays
- assertion links to durable assertion events and failure diagnostics
- table fact provenance and row source spans

Still open:

- broader live event streaming beyond `dev` is deferred to observability work
  outside this Gherkin lessons tracker
- richer evidence and artifact links for provider and policy failures can
  continue under provider observability work
