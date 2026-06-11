# Acceptance Fixtures

Status: draft v0 test-only format

Acceptance fixtures are JSON files that run a workflow through the existing
control-plane `dev` path and validate the final dev report. They are not
WhippleScript runtime syntax and do not add a second execution model.

## Fixture Shape

```json
{
  "schema": "whipplescript.acceptance_fixture.v0",
  "workflow": "provider-language-e2e.whip",
  "provider": "fixture",
  "setup": {
    "facts": [
      {
        "name": "LanguageTask",
        "value": {
          "provider": "codex",
          "language": "French",
          "expectedScript": "Latin",
          "prompt": "Write a short poem.",
          "artifactPath": "target/dogfood/language/codex-french.txt",
          "status": "queued"
        }
      }
    ]
  },
  "actions": [
    {"type": "pause", "reason": "exercise control-plane pause"},
    {"type": "resume"}
  ],
  "include_tags": ["acceptance"],
  "expect": {
    "dev_status": "success",
    "workflow": "ProviderLanguageE2E",
    "status": "passed",
    "source_metadata": {
      "targets": [
        {
          "target_kind": "workflow",
          "target": "ProviderLanguageE2E",
          "tags": ["fixture", "acceptance"],
          "description": "Fixture-backed provider x language acceptance workflow"
        },
        {
          "target_kind": "table",
          "target": "language_tasks",
          "tags": ["fixture"],
          "description": "Static provider x language task rows"
        }
      ]
    },
    "diagnostics": 0,
    "actions": [
      {"type": "pause", "count": 1},
      {"type": "resume", "count": 1}
    ],
    "assertions": {
      "total": 6,
      "passed": 6,
      "failed": 0,
      "error": 0
    },
    "assertion_tags": [
      {"tag": "acceptance", "total": 6, "passed": 6, "failed": 0, "error": 0}
    ],
    "assertion_reads": [
      {
        "source": "effect:kind agent.tell where status == completed",
        "match_count": 6,
        "matches": [
          {
            "name": "agent.tell",
            "status": "completed",
            "prompt_content_type": "markdown",
            "trace_items": 24,
            "evidence_items": 12,
            "trace_sequences": [3, 4, 5],
            "count": 6
          }
        ]
      }
    ],
    "trace": {
      "conformance": {"ok": true},
      "summary": {
        "events": 70,
        "abstract_events": 50
      },
      "groups": [
        {"type": "effect_created", "count": 12},
        {"type": "effect_terminal", "count": 12}
      ],
      "items": [
        {"sequence": 3, "type": "effect_created", "status": "queued"}
      ]
    },
    "inbox": [
      {"status": "pending", "severity": "normal", "count": 1}
    ],
    "summary": {
      "facts": 18,
      "effects": 12
    },
    "facts": [
      {"name": "LanguageE2EResult", "count": 6}
    ],
    "effects": [
      {"kind": "agent.tell", "status": "completed", "count": 6},
      {"kind": "baml.coerce", "status": "completed", "count": 6}
    ],
    "runs": [
      {"provider": "fixture", "status": "completed", "count": 12, "artifact_count": 12}
    ],
    "artifacts": [
      {"kind": "stdout_ref", "mime_type": "text/plain", "count": 6},
      {"kind": "transcript_ref", "mime_type": "text/plain", "count": 6}
    ],
    "evidence": [
      {"kind": "agent.turn.provider", "subject_type": "run", "count": 6},
      {"kind": "baml.coerce.provider", "subject_type": "run", "count": 6},
      {"kind": "skills.injected", "subject_type": "run", "count": 6},
      {"kind": "rule.committed", "subject_type": "rule_commit", "count": 19}
    ]
  }
}
```

Supported runner fields:

- `workflow`: `.whip` source path to run. Relative paths resolve from the
  fixture file directory.
- `root`: optional workflow root for multi-workflow bundles.
- `provider`: dev-loop provider, defaulting to `fixture`.
- `provider_config_paths`: optional provider config files. Relative paths
  resolve from the fixture file directory.
- `input`: optional workflow start input object, keyed by declared workflow
  input binding names and validated by the ordinary workflow start path.
- `setup.facts`: optional fixture setup facts derived into the started
  instance before setup actions and before the dev loop. Each entry has
  `name`, optional `key`, and `value`. The fact `name` must be a declared class,
  and `value` is validated against that class schema. Omitted keys use the same
  deterministic record key helper as ordinary `record` writes.
- `setup.inbox`: optional fixture setup inbox items created before setup
  actions and before the dev loop. Each entry has required `prompt`, optional
  `status` defaulting to `pending`, optional `severity` defaulting to `normal`,
  optional `choices`, optional `freeform_allowed` defaulting to `true`, and
  optional `related_effects` / `related_artifacts` arrays. Seeded inbox items
  are useful for acceptance tests that start with pre-existing human work.
- `actions`: optional control-plane setup actions applied to the started
  workflow instance before the dev loop runs. v0 supports `pause`, `resume`,
  and `cancel`; `pause` and `cancel` accept an optional `reason`.
- `include_tags` / `exclude_tags`: assertion report filters.
- `outcome`: fixture worker outcome, defaulting to `completed`.
- `max_iterations`: local dev loop iteration cap. When present, it must be a
  positive integer.

Supported expectations:

Expectation fields, including nested summary/count objects, are shape-checked
before the fixture starts. Wrong-typed expectations are rejected instead of
being treated as absent.

- `dev_status`: `success` or `failure`.
- `workflow`: expected report workflow name.
- `status`: expected `executable_spec.status`.
- `source_metadata.targets`: expected source metadata targets by
  `target_kind` and `target`; optional `tags` are required as a subset of the
  reported target tags, and optional `description` is matched exactly.
- `diagnostics`: expected final diagnostic count.
- `diagnostics_by_code`: expected final diagnostic counts by durable diagnostic
  `code`.
- `actions`: expected fixture action counts by action `type`.
- `assertions.total`, `assertions.passed`, `assertions.failed`,
  `assertions.error`: expected executable-spec summary counts.
- `assertion_tags`: expected executable-spec tag groups by `tag`, with optional
  `total`, `passed`, `failed`, and `error` counts.
- `assertion_untagged`: expected executable-spec untagged group counts, with
  optional `total`, `passed`, `failed`, and `error` counts.
- `assertion_reads`: expected deterministic assertion reads, selected by
  `source` or by optional `kind`/`head`/`guard`. Each assertion-read expectation
  must include at least one selector field. `match_count` checks the read total,
  and nested `matches` can count concrete matches by `name`, `status`,
  `prompt_content_type`, `provenance_class`, `trace_items`, or `evidence_items`.
  Matches can also pin exact `trace_sequences` and `evidence_ids` arrays for
  compact drilldown links. Trace/evidence fields are only attached to effect
  matches. Evidence ids may include run-specific material, so most portable
  fixtures should assert `evidence_items` and inspect observed `evidence_ids`
  rather than pinning them.
- `trace`: expected reconstructed trace summary. `conformance.ok` checks trace
  checker status, optional `summary.events` and `summary.abstract_events` check
  raw and reconstructed event totals, `groups` checks abstract trace event
  counts by `type`, and `items` checks concrete abstract trace records by
  optional `sequence`, `type`, `status`, `effect_id`, `run_id`, `predicate`,
  `reason`, or `provider`. Each trace item expectation must include at least
  one selector field.
- `inbox`: expected human inbox item counts by `status` and optional
  `severity`. Inbox items are created by ordinary `human.ask` execution.
- `summary.facts`, `summary.effects`: expected final active fact/effect totals.
- `facts`: expected final fact counts by class `name`.
- `effects`: expected final effect counts by `kind` and optional `status`.
- `runs`: expected provider run counts by `provider` and optional `status`;
  `artifact_count` can also assert the total artifacts recorded for matching
  runs.
- `artifacts`: expected metadata-only artifact counts by `kind` and optional
  `mime_type`.
- `evidence`: expected metadata-only evidence counts by `kind` and optional
  `subject_type`.

## CLI

```sh
whip --store .whipplescript/accept.sqlite \
  --json \
  accept examples/provider-language-e2e.accept.json
```

`examples/human-review.accept.json` is the smaller checked fixture for
human-inbox workflows. It pins the same fixture/dev/report path for
`askHuman`, including the `application/json` prompt content type, pending inbox
summary, provider run, trace summary, and metadata-only evidence.

The command emits `whipplescript.acceptance_report.v0` with pass/fail status,
failure messages, stable observed fact/effect totals and grouped count
summaries, compact observed provider-run summaries, metadata-only artifact and
evidence summaries, fixture control-action audit events, diagnostic summaries,
executable-spec summaries, and the embedded `whipplescript.dev_report.v0`
report.

`whip accept` intentionally accepts one fixture path in v0. Suite runners should
invoke it once per fixture and use a fresh store, or a store path scoped to that
fixture, because deterministic workflows can otherwise collide on stable
runtime ids when repeated in the same store.

## Guardrails

- Fixtures must remain outside `.whip` grammar.
- Fixtures must run through the same parser, compiler, runtime store, worker,
  and assertion report code paths as ordinary `dev` runs.
- Fixtures describe expected report and observed-store outcomes; they must not
  introduce hidden rule readiness, facts, effects, provider routing, or
  assertion semantics.
- Fixture `setup.facts` is only for durable external setup data. It must use
  declared class schemas and ordinary fact derivation before the normal dev
  loop; it must not bypass rule guards, create effects, or evaluate rules
  itself.
- Fixture `setup.inbox` is only for pre-existing human review work queues. It
  must not complete human effects, answer inbox items, create facts, or create
  effects.
- `setup.effects` and `setup.artifacts` are rejected in v0. Effects and
  artifacts should be produced through ordinary rules, workers, and provider
  harnesses so fixtures do not hide runtime behavior.
- Fixture control actions must call existing control-plane transitions only.
  They must not create a fixture-specific state machine.
- Fixtures may expect success or failure. An expected failing executable spec
  should assert `dev_status: "failure"`, `status: "failed"`, diagnostic counts,
  and assertion summary counts explicitly.
- Do not add multi-fixture CLI execution until suite isolation is explicit.
  Reusing one store for repeated deterministic workflows can make fixture runs
  interfere with each other through stable ids.
