# Gherkin Lessons Tracker

Status: v0 complete

This tracker turns the Gherkin/Cucumber review into concrete WhippleScript work.
The goal is to incorporate useful authoring, reporting, and validation lessons
without changing WhippleScript into a Cucumber-style free-text BDD runner.

Reference material:

- Gherkin reference: https://cucumber.io/docs/gherkin/reference/
- Gherkin parser/compiler architecture: https://github.com/cucumber/gherkin
- Cucumber Messages event protocol: https://github.com/cucumber/messages

## Design Position

Borrow:

- concise static examples and table data
- executable specification mindset
- tags for selection and reporting
- non-semantic descriptions for human-readable reports
- content-typed multiline strings
- schema-backed event/report output

Do not borrow:

- `Given` / `When` / `Then` as core `.whip` workflow syntax
- free-text step matching to external definitions
- natural-language phrases that secretly lower to effects
- localized language keywords
- a runtime dependency on Cucumber or Gherkin

WhippleScript remains a typed, deterministic, event-sourced rule system. Any
feature in this tracker must preserve explicit facts, effects, dependencies,
source spans, typed IR, replay, and diagnostics.

## Status Legend

- [x] Implemented and covered by tests.
- [~] Partially specified or implemented, with known gaps.
- [ ] Not implemented.

## Acceptance Pipeline

Each feature should pass these gates before it is considered complete:

1. **Spec**: syntax, static semantics, runtime semantics, IR/event/report shape,
   and compatibility rules are documented.
2. **Implementation**: parser, lowering, runtime/store, CLI, formatter, docs, or
   reporting code implements the specified behavior.
3. **Validation**: focused unit tests, parser diagnostics, golden IR snapshots,
   runtime fixtures, CLI e2e, or generated model checks cover expected behavior.
4. **Review**: examples and docs are reviewed for readability, determinism, and
   no hidden control flow.

## Feature Backlog

| ID | Feature | Spec | Implementation | Validation | Review | Notes |
| --- | --- | --- | --- | --- | --- | --- |
| GHK-001 | Typed static tables | [x] | [x] | [x] | [x] | `table name as Class [...]` compiles to a generated `when started` record rule with row type validation, formatter support, parser tests, stable IR, row source metadata, table fact provenance, fixture-backed e2e coverage, source metadata reports, and acceptance report coverage. |
| GHK-002 | Table migration examples | [x] | [x] | [x] | [x] | `examples/provider-language-e2e.whip` uses `table language_tasks`; its golden IR, fixture-backed e2e test, acceptance fixture, and docs were updated. Broader example migration can continue opportunistically outside this Gherkin lessons tracker. |
| GHK-003 | Tags | [x] | [x] | [x] | [x] | Tags are specified and implemented as non-semantic metadata on workflows, tables, assertions, and rules, with parser/formatter/IR coverage, provider-language example tags, `check --json`/`compile --json` source metadata surfacing, `dev` assertion include/exclude filters, executable-spec tag groups, and acceptance fixture coverage. Rule/effect filtering remains intentionally out of scope. |
| GHK-004 | Non-semantic descriptions | [x] | [x] | [x] | [x] | `description "..."` is supported on workflows, tables, assertions, and rules, preserved as typed IR source metadata, formatted, tested, used in the provider-language and human-review examples, surfaced through `check --json`/`compile --json`, and pinned for workflow/table/assertion/rule targets in acceptance fixtures. |
| GHK-005 | Content-typed multiline strings | [x] | [x] | [x] | [x] | `tell`, `askHuman`, and `coerce` multiline prompts can use known short names like `"""markdown` or MIME-style delimiters like `"""application/json`; CLI lowering preserves `prompt_content_type` in effect input JSON, preserves coerce `prompt_template`, and has focused unit coverage for all three prompt surfaces. Parser diagnostics catch malformed annotation-shaped openers, formatter coverage preserves annotated multiline prompts including JSON human prompts, effect read matches surface prompt content type in dev reports, the provider-language acceptance fixture pins Markdown `agent.tell` and `baml.coerce` assertion-read matches, and `examples/human-review.accept.json` pins `application/json` `human.ask` assertion reads plus pending inbox output. Artifact and evidence reports include compact item links without exposing raw paths, content, or evidence payloads. |
| GHK-006 | Executable-spec reports | [x] | [x] | [x] | [x] | Report surfaces are specified, implemented, schema-validated, and covered by checked examples: `check --json`, `compile --json`, and `dev --json` emit `source_metadata`; `dev --json` includes an `executable_spec` assertion summary grouped by tag, provider run summaries, metadata-only artifact/evidence summaries, compact artifact/evidence item links, durable diagnostics, assertion reports with deterministic reads and concrete fact/effect matches plus prompt content types, target ids/event ids/diagnostic ids/tags/descriptions, and table facts expose row provenance/source spans. Acceptance fixtures can pin executable-spec tagged/untagged group counts, deterministic assertion reads, assertion-read trace/evidence link counts, trace conformance, raw/abstract trace summary totals, trace groups/items, human inbox counts, provider run counts, metadata-only evidence/artifact counts, and final fact/effect totals; acceptance reports expose compact observed executable-spec, source-metadata, assertion-read, provider-run, artifact, evidence, inbox, trace summary/groups/items, and diagnostic summaries. Assertion-read effect match groups include compact `trace_sequences` and `evidence_ids` links for drilldown. The checked provider-language and human-review fixtures cover both multi-provider automation and manual-review report surfaces. |
| GHK-007 | Stable report/event protocol | [x] | [x] | [x] | [x] | `spec/reporting.md`, `spec/report-schemas/*.schema.json`, and `scripts/check-report-schemas.sh` document and validate the current check/compile/dev/trace/accept JSON contract, `dev --stream ndjson` envelopes, schema ids, source metadata, assertion filters, streamed `dev.events` raw runtime event batches, streamed `dev.assertions` summaries, durable diagnostics, assertion reads/matches, assertion-read trace/evidence link counts and compact link arrays, executable-spec summaries, provider run summaries, metadata-only artifact/evidence summaries, compact artifact/evidence item links, compact and assertable acceptance trace summaries/groups/items, assertion links to durable assertion events/diagnostics, table provenance, and the checked provider-language plus human-review acceptance reports. Broader live streaming outside `dev` is deferred to the observability roadmap rather than this Gherkin lessons tracker. |
| GHK-008 | Acceptance fixture format | [x] | [x] | [x] | [x] | Draft JSON fixture format specified in `spec/acceptance-fixtures.md`; `whip accept <fixture.json>` runs one fixture through the existing `dev` parser/compiler/runtime/report path and validates final dev report expectations, source metadata targets, diagnostic counts by code, fixture control-action counts, executable-spec summary/tag/untagged group counts, deterministic assertion reads/matches, trace conformance, raw/abstract trace summary totals, trace groups/items, human inbox counts, provider run counts, metadata-only artifact/evidence counts, and final fact/effect totals plus grouped counts. Fixture workflow and provider-config paths resolve relative to the fixture file, fixture `input` is covered as ordinary validated workflow start input, fixture `setup.facts` can derive typed external setup facts validated against declared class schemas, `setup.inbox` can create pre-existing human review queue items, and fixture `actions` can apply real `pause`/`resume`/`cancel` control-plane transitions before the dev loop. Acceptance reports include stable observed fact/effect totals, grouped count summaries, action audit events, source-metadata summaries, assertion-read summaries, provider run counts, artifact/evidence counts, inbox summaries, trace conformance summaries, diagnostic counts by code, and executable-spec summaries. `examples/provider-language-e2e.accept.json` covers pause/resume actions, `examples/human-review.accept.json` covers human inbox expectations and concrete trace item selectors, and CLI tests cover typed setup facts, setup inbox items, fixture-shape rejection, expectation and nested-expectation shape rejection, assertion-read selector rejection, scalar option validation, unsupported setup collection rejection, invalid setup inbox diagnostics, zero `max_iterations` rejection, provider-config path validation, cancel actions, expected-failure fixtures, schema validation, trace-summary/item mismatch output, and broader mismatch failure output for the current fixture contract. `setup.effects` and `setup.artifacts` are rejected v0 non-goals; multi-fixture suites should isolate stores externally until suite-level id namespacing is specified. |
| GHK-009 | Formatter and diagnostics polish | [x] | [x] | [x] | [x] | New syntax has focused formatter and diagnostic coverage, including content-typed prompt formatter tests, malformed annotation diagnostics, misplaced multiline effect-binding diagnostics, and targeted diagnostics for pasted Gherkin `Feature`/`Rule`/`Background`/`Scenario`/`Examples`/`Given`/`When`/`Then`/`And`/`But` text. |
| GHK-010 | Authoring docs and companion skill | [x] | [x] | [x] | [x] | `docs/language-reference.md`, `docs/tutorial.md`, `docs/examples.md`, `docs/current-state.md`, `docs/api-reference.md`, and `skills/whipplescript-author/SKILL.md` cover matrices, tags/descriptions, content-typed prompts, tag-filtered `dev`, final JSON reports, NDJSON streams with `dev.events` and `dev.assertions`, report diagnostics, compact artifact/evidence links, acceptance fixture `setup.facts`/`setup.inbox`/`actions`, rejected `setup.effects`/`setup.artifacts`, fixture and expectation shape validation, assertion-read drilldowns and selector requirements, trace item selectors, and the checked provider-language plus human-review acceptance fixtures. |

## Guardrails

- Tags and descriptions are metadata. They must not introduce hidden rule
  readiness, hidden effects, provider routing, retries, or authorization.
- Table rows are static data, not loops over runtime collections.
- Table expansion must preserve row-level source spans, stable idempotency keys,
  and fact provenance.
- Content-typed strings are metadata around an existing string body. They must
  not invoke parsers, providers, validators, or model decisions implicitly.
- Any semantic judgment still goes through explicit `coerce`, `call`,
  `askHuman`, `tell`, or `invoke` effects.
- The core grammar must keep `when` as rule readiness/fact observation. Do not
  add `Given`, `When`, or `Then` aliases inside `.whip` workflows.
- If an acceptance fixture format is added, it should compile/run workflows
  through the same control-plane surfaces rather than becoming a second runtime.

## Proposed Implementation Order

### Phase 1: Table Syntax

- [x] Finish `table` syntax and semantics in `spec/language.md`.
- [x] Define AST/IR nodes for static matrices, including row source spans.
- [x] Decide whether matrices are top-level declarations, workflow-local
  declarations, or rule-body seed sugar.
- [x] Implement parser and formatter support.
- [x] Type-check rows against declared class schemas, including `AgentRef`,
  literal unions, arrays, maps, and object fields.
- [x] Lower rows to ordinary deterministic `record` writes during rule
  evaluation.
- [x] Add golden IR fixtures and focused invalid parser coverage.
- [x] Migrate at least one validation workflow from repeated `record` blocks to
  a table.

### Phase 2: Tags And Descriptions

- [x] Specify allowed tag syntax, placement, inheritance, and duplicate handling.
- [x] Specify description placement and termination rules.
- [x] Add parser/IR support while preserving source spans.
- [x] Add CLI filters such as include/exclude tags where useful.
- [x] Ensure compile/check reports include tags and descriptions without
  changing execution.
- [x] Add tests proving tags/descriptions lower as metadata only.

### Phase 3: Typed Multiline Strings

- [x] Specify content type grammar for rule-body multiline prompt strings.
- [x] Decide which surfaces preserve content type metadata: `tell`, `coerce`
  prompts, `askHuman`, and assertion effect-read reports preserve it; `call`
  and evidence/artifact surfacing are out of the current prompt-only syntax.
- [x] Reject unsupported or malformed content-type annotations with clear
  diagnostics.
- [x] Add formatter and stable fixture/report coverage.
- [x] Add at least one Markdown prompt example and one JSON-like payload
  validation fixture if the syntax supports it.

### Phase 4: Reports And Event Streams

- [x] Define a stable report schema for source metadata, tables, assertions,
  assertion event links, effects, diagnostics, evidence, and trace links.
- [x] Decide whether the stream is JSON array output, NDJSON, or both.
- [x] Add CLI output mode and compatibility notes.
- [x] Add tests for assertion failure, provider failure, table rows, tags, and
  descriptions in report output.
- [x] Review output with the documentation examples to ensure it reads as an
  executable specification rather than raw runtime noise.

### Phase 5: Optional Acceptance Fixtures

- [x] Decide whether a separate fixture format is needed after matrices and
  reports land.
- [x] If needed, specify a test-only fixture grammar that describes workflow
  inputs, control-plane actions, human inbox observations, expected
  assertions/tag groups, diagnostics, and observed fact/effect counts.
- [x] Keep the fixture runner outside core `.whip` runtime semantics.
- [x] Add e2e tests that prove fixtures exercise the same parser, compiler,
  runtime store, worker, assertion report, and kernel paths as normal workflows.
- [x] Decide suite isolation semantics before accepting multiple fixture paths:
  v0 keeps `whip accept` single-fixture, and suite runners must invoke it once
  per fixture with isolated stores until suite-level id namespacing is
  specified.

## Review Checklist

Use this checklist before closing any feature in this tracker:

- [x] The feature makes examples shorter or reports clearer without hiding
  durable state changes.
- [x] Source spans survive into diagnostics, IR snapshots, traces, and reports.
- [x] Parser diagnostics reject tempting Cucumber-style misuse where relevant.
- [x] Runtime behavior remains deterministic and replayable.
- [x] New syntax has at least one valid fixture and one invalid diagnostic
  fixture.
- [x] User-facing docs explain why the feature exists and what it does not do.
- [x] The companion authoring skill tells agents how to use the feature safely.

## Decided Questions

- `table` declarations seed ordinary facts through a generated `when started`
  rule; they are not runtime loops and do not require an explicit user-authored
  emit rule.
- Tags are allowed on workflows, tables, assertions, and rules. Individual
  table row tags remain out of scope until there is a concrete report or
  filtering need.
- Descriptions are plain opaque strings. Markdown-style prose can be written in
  the string body, but descriptions do not currently carry a separate content
  type.
- `dev --json` remains the main executable-spec report. `dev --stream ndjson`
  adds progress envelopes, while `accept` emits a distinct fixture result report
  that embeds the ordinary dev report.
- A separate JSON acceptance fixture format is useful for test harnesses, but it
  stays outside `.whip` syntax and outside runtime semantics.
