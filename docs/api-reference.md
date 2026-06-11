# CLI & API reference

The implemented surfaces: CLI commands and their JSON output, a compact
language index, runtime status values and event types, JSON inspection
shapes, and Rust crate APIs. Semantics and examples live in the
[language reference](language-reference.md) and
[runtime & operations](runtime-operations.md).

## Global CLI options

All CLI commands use the same global shape:

```sh
whip [--store path] [--json] [--input JSON] <command> [args]
```

| Option | Meaning |
| --- | --- |
| `--store path` | SQLite store path. Defaults to `.whipplescript/store.sqlite`, or `WHIPPLESCRIPT_STORE` when set. Use `:memory:` for in-memory tests. |
| `--json` | Emit machine-readable JSON where the command supports it. |
| `--input JSON` | Start input for `run` and `dev`. The payload must be keyed by declared workflow input names. |

The current command set is:

```text
check, compile, run, revise, step, worker, dev, accept, instances, status, log,
facts, effects, runs, artifacts, inbox, items, evidence, diagnostics, trace,
pause, resume, cancel, retry, recover, doctor
```

Run `whip <command> --help` or `whip help <command>` to print the usage line
for any command.

### Environment variables

| Variable | Meaning |
| --- | --- |
| `WHIPPLESCRIPT_STORE` | Default store path when `--store` is omitted. |
| `WHIPPLESCRIPT_ITEMS_STORE` | Path for the builtin work-queue tracker (defaults to `.whipplescript/items.sqlite`). |
| `WHIPPLESCRIPT_EXEC_ALLOW` | Dev-profile raw `exec "<command>"` allow-list: colon-separated glob prefixes such as `scripts/*:bin/ci-*`. Commands that do not match fail without running. |
| `WHIPPLESCRIPT_EXEC_PROFILE` | `dev` (default) or `hosted`. Hosted rejects raw exec strings and requires script capabilities. |
| `WHIPPLESCRIPT_SCRIPT_MANIFEST` | JSON manifest path for hosted script capabilities. Equivalent to `--script-manifest`. |
| `WHIPPLESCRIPT_RUN_ID` | Run identity stamped onto items filed by an agent through `whip items add`. |
| `WHIPPLESCRIPT_PROVIDER_CONFIGS` | Colon-separated provider binding config paths for the worker (`WHIPPLESCRIPT_NATIVE_PROVIDER_CONFIGS` is a legacy alias). |

## CLI commands

### `doctor`

```sh
whip doctor
whip --json doctor
whip --json doctor --providers
whip --json doctor --provider-config examples/provider-configs/native/native.example.json
```

Opens or creates the configured store, reports schema version, and checks
optional tools:

```text
maude
java
apalache-mc or apalache
baml-cli or baml
codex
claude
pi
loft
```

With `--provider-config`, JSON output includes `provider_config_checks`. Each
check contains the config path and redacted validation `results`.
With `--providers`, JSON output includes `provider_health_checks`, a
deterministic non-live posture for Codex, Claude, and Pi. It reports CLI
availability, credential-reference posture, and deeper checks that require
explicit real-provider validation without printing credential values.

### `check`

```sh
whip check [--model-search] [--root Workflow] \
  [--exec-profile dev|hosted] [--script-manifest <path>] \
  <workflow.whip>...
whip --json check [--model-search] [--root Workflow] \
  [--exec-profile dev|hosted] [--script-manifest <path>] \
  <workflow.whip>...
```

Parses, resolves includes, type-checks, lowers to IR, enforces the
[liveness checks](language-reference.md#liveness-checks), and prints the IR
snapshot. With `--model-search`, also runs generated Maude checks when
available.

With `--exec-profile hosted`, raw `exec "..."` is a check error and named
`exec <capability> with <record>` forms must resolve in the supplied script
manifest.

JSON output is an array with one report per input path. Successful entries
include source hashes, the IR snapshot, and `source_metadata`:

```json
[
  {
    "schema": "whipplescript.check_report.v0",
    "path": "examples/provider-language-e2e.whip",
    "status": "ok",
    "workflow": "ProviderLanguageE2E",
    "source_hash": "...",
    "ir_hash": "...",
    "snapshot": "...",
    "source_metadata": {
      "tags": [
        {"name": "fixture", "target_kind": "workflow", "target": "ProviderLanguageE2E"}
      ],
      "descriptions": [
        {
          "value": "Static provider x language task rows",
          "target_kind": "table",
          "target": "language_tasks"
        }
      ],
      "targets": {
        "workflow:ProviderLanguageE2E": {
          "target_kind": "workflow",
          "target": "ProviderLanguageE2E",
          "tags": ["fixture", "acceptance"],
          "description": "Fixture-backed provider x language acceptance workflow"
        }
      }
    }
  }
]
```

Diagnostic entries use `"status": "error"` and include structured source spans.

Exit behavior:

| Exit | Meaning |
| --- | --- |
| `0` | All inputs compile and optional model searches pass. |
| `1` | Diagnostics or generated checks failed. |
| `2` | CLI usage error. |

### `compile`

```sh
whip compile <workflow.whip> [--root Workflow]
whip --json compile <workflow.whip> [--root Workflow]
```

Prints the compiled IR snapshot. JSON output includes:

```json
{
  "schema": "whipplescript.compile_report.v0",
  "path": "examples/minimal-noop.whip",
  "workflow": "MinimalNoop",
  "source_hash": "...",
  "ir_hash": "...",
  "snapshot": "...",
  "source_metadata": {
    "tags": [],
    "descriptions": [],
    "targets": {}
  }
}
```

### `run`

```sh
whip [--store path] [--input JSON] run <workflow.whip> [--root Workflow]
```

Compiles the source bundle, creates a program version if needed, creates an
instance, appends `external.started`, and seeds declared workflow input facts.
It does not run ready rules or providers.

JSON output:

```json
{
  "instance_id": "inst_...",
  "program_id": "prg_...",
  "version_id": "ver_...",
  "workflow": "WorkflowName",
  "store": ".whipplescript/store.sqlite"
}
```

### `step`

```sh
whip [--store path] step <instance> --program <workflow.whip> [--root Workflow]
```

Runs deterministic rule evaluation for one instance until no further rule commit
is possible. It may create facts, consume facts, enqueue effects, add dependency
edges, and execute workflow terminal actions. It never executes providers.

Human output:

```text
step <instance> committed_rules=N facts=N consumed=N effects=N
```

JSON output includes:

```json
{
  "instance_id": "inst_...",
  "committed_rules": 1,
  "facts_created": 1,
  "facts_consumed": 0,
  "effects_created": 2,
  "guards": [],
  "branches": []
}
```

### `worker`

```sh
whip [--store path] worker <instance> \
  [--provider fixture] \
  [--provider-config <path>] \
  [--exec-profile dev|hosted] \
  [--script-manifest <path>] \
  [--program <workflow.whip>] \
  [--root Workflow] \
  [--once] \
  [--fail | --timeout | --cancel] \
  [--max-child-iterations N]
```

Starts currently claimable effects and completes them through the selected
provider. The default provider is the deterministic fixture provider.
`--provider-config <path>` can be repeated to bind source harness ids to
concrete provider configs; worker also reads colon-separated
`WHIPPLESCRIPT_PROVIDER_CONFIGS` and the legacy
`WHIPPLESCRIPT_NATIVE_PROVIDER_CONFIGS`. `--fail`, `--timeout`, and `--cancel`
force fixture terminal outcomes for failure-path tests.

Hosted script execution uses `--exec-profile hosted --script-manifest <path>`
or `WHIPPLESCRIPT_EXEC_PROFILE=hosted` plus
`WHIPPLESCRIPT_SCRIPT_MANIFEST=<path>`. The worker registers `script.<name>`
capabilities for the instance program, verifies SHA-256 before spawn, and
runs argv-direct with JSON stdin.

Supported fixture effect kinds:

```text
agent.tell
baml.coerce
human.ask
capability.call
workflow.invoke
queue.file
queue.claim
queue.release
queue.finish
timer.wait
exec.command
```

JSON output includes:

```json
{
  "instance_id": "inst_...",
  "provider": "fixture",
  "ran_effects": 1,
  "terminal_events": ["evt_..."]
}
```

### `dev`

```sh
whip [--store path] [--input JSON] dev <workflow.whip> \
  [--root Workflow] \
  [--provider fixture] \
  [--provider-config <path>] \
  [--exec-profile dev|hosted] \
  [--script-manifest <path>] \
  [--until idle] \
  [--max-iterations N] \
  [--include-tag TAG] \
  [--exclude-tag TAG] \
  [--stream ndjson] \
  [--fail | --timeout | --cancel]
```

Convenience local validation loop. It starts a new instance, alternates `step`
and `worker`, stops when idle or when `--max-iterations` is reached, then
evaluates source assertions. `--provider-config <path>` can be repeated and is
passed to the embedded worker loop. `--include-tag <tag>` and `--exclude-tag
<tag>` can be repeated to select which source assertions are evaluated and
reported; they do not skip rules, effects, providers, or table seeding.
Exclusion takes precedence when both filters match an assertion.

`--exec-profile hosted --script-manifest <path>` applies the hosted script
capability checks before the instance starts and passes the same manifest to
the embedded worker.

`--stream ndjson` emits compact line-delimited JSON progress envelopes with
schema `whipplescript.dev_stream.v0`. Current events are `dev.started`,
`dev.events`, `dev.step`, `dev.worker`, `dev.idle`, `dev.assertions`, and
`dev.report`. `dev.events` carries batches of newly persisted raw runtime events
using the same object shape as `log --json`. `dev.assertions` carries the compact
executable-spec assertion summary; the final `dev.report` line embeds the same
`whipplescript.dev_report.v0` object as `dev --json`.

JSON output includes the instance id, workflow name, `source_metadata`,
per-iteration step reports, worker reports, durable diagnostics for the dev
instance, compact `provider_runs`, `provider_artifacts`, and
`provider_evidence` summaries, an `executable_spec` assertion summary grouped
by source tag, assertion filter counts, and assertion reports.
`provider_artifacts` groups metadata by artifact kind and MIME type and
includes compact artifact item links without exposing artifact paths or
content. `provider_evidence` groups evidence metadata by kind and subject type
and includes compact evidence item links without exposing evidence metadata
payloads.
Assertion reports include `target_id`, source tags, and any assertion
description, plus `event_id` links to the durable assertion event,
`diagnostic_ids` links for failed or errored assertions, and
deterministic fact/effect `reads` so acceptance reports can group checks by
source metadata and the projections they validate. Each read includes
`match_count` and concrete fact/effect match ids where available. Effect
matches include `prompt_content_type` when the effect input preserved an
annotated multiline prompt. Acceptance report assertion-read summaries also
include compact trace/evidence item counts for grouped effect matches.

### `accept`

```sh
whip [--store path] [--json] accept <fixture.json>
```

Runs a test-only acceptance fixture through the same local `dev` control-plane
path and validates the final report. Fixtures use schema
`whipplescript.acceptance_fixture.v0`; reports use
`whipplescript.acceptance_report.v0`, include `observed.summary` totals plus
grouped final fact/effect count summaries, compact observed provider-run,
artifact-link, evidence-link, control-action, source-metadata, assertion-read,
diagnostic, trace, inbox, and executable-spec summaries, and the full
`whipplescript.dev_report.v0` under `dev_report`. Relative `workflow` and
`provider_config_paths` entries resolve from the fixture file directory.
Fixtures can assert diagnostics by code, executable-spec summaries, tagged and
untagged executable-spec groups, deterministic assertion reads and match
metadata such as prompt content type plus trace/evidence link counts, fixture
action counts, final fact/effect totals, grouped final fact/effect counts,
source metadata targets, provider run counts, metadata-only artifact counts,
metadata-only evidence counts, and human inbox item counts. Observed
assertion-read match groups include compact `trace_sequences` and `evidence_ids`
links for drilldown. The observed trace summary reports event totals,
reconstructed abstract trace event groups, compact abstract trace items, and
conformance; fixtures can assert trace summary fields, groups, and stable item
selectors through `expect.trace`. `expect.assertion_reads` entries must include
at least one selector: `source`, `kind`, `head`, or `guard`.
The v0 command accepts one fixture path; external suite runners should isolate
stores per fixture.

Fixtures may also provide `setup.facts` entries with a declared class `name`,
optional stable `key`, and JSON `value`. These setup facts are validated against
the workflow class schema and derived into the started instance before setup
actions and before the normal dev loop. `setup.inbox` can create pre-existing
human review items with a prompt, status/severity, choices, and related link
arrays before the normal dev loop. `setup.effects` and `setup.artifacts` are
rejected in v0; effects and artifacts must be produced through ordinary rules,
workers, and providers. Fixture `actions` can apply real `pause`, `resume`, or
`cancel` control-plane transitions before the dev loop. Fixture and expectation
fields are shape-checked before the workflow starts so wrong-typed expectations
are rejected rather than treated as absent.

### `revise`

```sh
whip [--store path] revise <instance> <workflow.whip> \
  [--root Workflow] \
  [--dry-run] \
  [--cancel keep|queued|running]
```

Checks whether a candidate source bundle can become the active program version
for a non-terminal running instance. With `--dry-run`, it reports compatibility
without changing the store. Without `--dry-run`, it records a revision
activation event and future `step` calls use the new active program version.

Cancellation policy controls old-version effects:

| Policy | Meaning |
| --- | --- |
| `keep` | Keep old-version effects claimable/runnable. |
| `queued` | Terminal-cancel queued, blocked, and claimable old-version effects. |
| `running` | Cancel queued old-version effects and request cancellation for running old-version work. |

Running cancellation requests are not terminal results. Providers still record
the eventual completion, failure, timeout, or cancellation acknowledgement.

JSON dry-run output includes the candidate version, compatibility diagnostics,
agent impact, cancellation impact, and no activation event. Activation output
includes the activated version, revision epoch, cancellation policy, diagnostics,
and evidence links.

### Inspection commands

| Command | Meaning |
| --- | --- |
| `instances` | List all instances in the configured store. |
| `status <instance>` | Show instance status, counts, recent events, and workflow invocation links in JSON. |
| `log <instance>` | Show append-only event log. |
| `facts <instance>` | Show current unconsumed facts. |
| `effects <instance>` | Show effects, status, target, profile, and block reason. |
| `runs <instance>` | Show provider run attempts. |
| `evidence <instance>` | Show evidence records and evidence links. |
| `diagnostics <instance>` | Show durable diagnostics. |
| `trace <instance> [--check]` | Show trace bundle; with `--check`, reconstruct abstract trace and run conformance checks. |

All inspection commands support `--json`.
Facts seeded from `table` declarations appear in JSON with
`"provenance_class": "table"` and a `source_span` whose `construct` is
`"table_row"`.

### Inbox commands

```sh
whip inbox [<instance>]
whip inbox show <item>
whip inbox answer <item> --choice <value> [--by NAME]
whip inbox answer <item> --text <value> [--by NAME]
```

Inbox commands inspect and answer human review requests created by `human.ask`
effects. Bare `whip inbox` lists pending items; `whip inbox <instance>`
filters pending items to one instance.

### Items commands

```sh
whip items add --queue <Q> --title <T> [--body <B>] [--label <L>]
whip items list [--queue <Q>] [--status <S>]
whip items show <id>
```

Items commands operate the builtin work-queue tracker (see
[work queues](language-reference.md#work-queues)). The builtin tracker is
workspace-scoped, stores items in `.whipplescript/items.sqlite` (override with
`WHIPPLESCRIPT_ITEMS_STORE`), and issues sequential ids `WS-1`, `WS-2`, and so
on. `--status` filters on the item status categories `open`, `in_progress`,
`done`, and `cancelled`. When an agent files an item mid-turn through
`whip items add`, the new item carries run-identity provenance taken from the
`WHIPPLESCRIPT_RUN_ID` environment variable.

### Lifecycle commands

```sh
whip pause <instance>
whip resume <instance>
whip cancel <instance>
whip retry <instance> <effect>
whip recover <instance>
```

| Command | Meaning |
| --- | --- |
| `pause` | Transition a running instance to paused. |
| `resume` | Transition a paused instance back to running. |
| `cancel` | Transition a running or paused instance to terminal cancelled. |
| `retry` | Move an eligible failed or timed-out effect back to queued. |
| `recover <instance>` | Reconcile interrupted native provider runs from persisted provider evidence. |

Terminal instances are absorbing: completed, failed, and cancelled instances do
not accept further public lifecycle transitions or rule commits.

## Language reference index

For examples and semantics, see [Language Reference](language-reference.md).
This section is a compact index of source constructs.

### Top-level constructs

| Construct | Surface | Meaning |
| --- | --- | --- |
| Workflow | `workflow Name { ... }` or `workflow Name` | Deployable runtime boundary. |
| Contract | `input name Type`, `output name Type`, `failure name Type` | Typed workflow input/output/failure contract. |
| Include | `include "path.whip"` | Source bundle composition. |
| Plugin import | `use memory` | Import plugin by name. |
| Class | `class Name { field Type }` | Typed fact and payload schema. |
| Enum | `enum Name { A B }` | Finite string domain. |
| Agent | `agent name { profile "..."; capacity N; skills [...] }` | Logical provider target and policy metadata. |
| Coerce | `coerce fn(args...) -> Type { prompt """markdown ... """ }` | Declared BAML-backed effect. |
| Flow | `flow name when ... { step; step; ... }` | A rule whose body is a multi-step sequence; lowers to `flow.<name>.seg<N>` rules. |
| Queue | `queue name { tracker builtin }` | Declared vendor-neutral work-item backlog. |
| Pattern | `pattern Name<T> { ... }` | Compile-time reusable fragment. |
| Apply | `apply Name<Type> as Alias { ... }` | Pattern specialization. |
| Assertion | `assert expression` | Deterministic projection check in `dev`. |

### Rule constructs

| Construct | Surface | Meaning |
| --- | --- | --- |
| Rule | `rule name ... => { ... }` | Atomic deterministic rewrite. |
| Fact match | `when Class as binding` | Bind an unconsumed fact. |
| Guarded match | `when Class as binding where expr` | Bind fact only when pure guard is true. |
| Started event | `when started` | Match the initial `external.started` event. |
| Readiness | `when Class as item` or `when { ... }` | Match facts and other deterministic rule conditions. |
| Availability | `worker is available` inside a `when` clause/group | Match logical agent capacity/policy availability. |
| Human answer | `when human answered <label> as x` | Match a `human.answer.received` fact created when an inbox item is answered. The binding payload exposes `choice`, `text`, `answered_by`, `prompt`, `inbox_item_id`, and `effect_id`. |
| Agent turn | `when <agent> completed turn ... [as x]` | Match an `agent.turn.completed` fact. A declared agent name filters to that agent's turns; the generic word `worker` matches any agent. |
| Queue readiness | `when <queue> has ready item as x` | Match an item that is ready to be claimed in a work queue. |
| General event | `when fact <dotted.name> as x [where ...]` | General readiness form; the English phrases above are sugar over it. |

### Rule body operations

| Operation | Effect/commit output |
| --- | --- |
| `record Class { ... }` | New fact. |
| `record Class from binding { ... }` | New fact with copied fields. |
| `done binding` | Mark matched fact consumed. `consume binding` is a deprecated alias; the checker now emits a warning for it. |
| `done binding -> record ...` | Consume and create replacement fact atomically. |
| `tell agent ... [timeout <dur>] as turn` | `agent.tell` effect. |
| `coerce fn(...) as result` | `baml.coerce` effect. |
| `decide "..." -> { ... } as result` | Inline typed `baml.coerce` effect. |
| `exec "<command>" as result` | Dev-profile `exec.command` effect (gated by `WHIPPLESCRIPT_EXEC_ALLOW`; exposes `exit_code`, `stdout`). |
| `exec <capability> with <record> -> Type as result` | Hosted `exec.command` effect requiring `script.<capability>`, typed JSON stdin, SHA-256 manifest verification, and typed stdout ingestion. |
| `file item into <queue> { ... }` | `queue.file` effect. |
| `claim <item> [as x]` | `queue.claim` effect (already-claimed is a branchable failure). |
| `release <item>` | `queue.release` effect. |
| `finish <item> [{ summary ... }]` | `queue.finish` effect. |
| `timer <duration> as x` | `timer.wait` effect completed when due. |
| `cancel <binding>` | Terminal-cancel a pending effect; request cancellation of a running one. |
| `askHuman ... [choices [...]] ...` | `human.ask` effect. |
| `call capability for value as result` | `capability.call` effect. |
| `invoke Workflow { ... } as child` | `workflow.invoke` effect. |
| `after effect succeeds/fails/completes` | Dependency branch scoped by terminal status. |
| `case expr { Pattern => { ... } }` | Deterministic finite-domain branch. |
| `complete output { ... }` | `workflow.completed` event and terminal completed state. |
| `fail failure { ... }` | `workflow.failed` event and terminal failed state. |

## Status values

### Instance status

```text
running
paused
completed
failed
cancelled
```

`completed`, `failed`, and `cancelled` are terminal.

### Effect status

```text
queued
blocked_by_dependency
blocked_by_capacity
blocked_by_capability
blocked_by_profile
running
completed
failed
timed_out
cancelled
```

### Run status

```text
running
completed
failed
timed_out
cancelled
lease_expired
```

### Lease status

```text
active
released
expired
```

## Event types

Common event types:

| Event | Meaning |
| --- | --- |
| `external.started` | Instance start input event. |
| `rule.committed` | Rule atomically committed facts/effects/dependencies/terminal action. |
| `effect.run_started` | Provider run started for an effect. |
| `effect.terminal` | Effect completed, failed, timed out, or cancelled. |
| `effect.blocked` | Effect blocked before provider start. |
| `effect.cancellation_requested` | Running effect received a durable cancellation request. |
| `effect.retried` | Effect returned to queued for retry. |
| `lease.expired` | Active run lease expired. |
| `lease.renewed` | Active run lease was renewed. |
| `instance.transitioned` | Pause/resume/cancel transition. |
| `workflow.completed` | Workflow produced declared output and became completed. |
| `workflow.failed` | Workflow produced declared failure and became failed. |
| `workflow.revision_activated` | Instance active program version changed. |
| `workflow.revision_rejected` | Non-dry-run revision failed compatibility checks. |
| `fact.derived` | Runtime projection derived a durable fact from an event/effect. |
| `assertion.passed` | Explicit assertion evaluation returned true. |
| `assertion.failed` | Explicit assertion evaluation returned false and produced a diagnostic. |
| `assertion.errored` | Explicit assertion evaluation could not produce a boolean result and produced a diagnostic. |
| `agent.turn.completed` | Agent turn completion projection. |
| `agent.turn.failed` | Agent turn failure projection. |
| `agent.turn.timed_out` | Agent turn timeout projection. |
| `agent.turn.cancelled` | Agent turn cancellation projection. |
| `agent.turn.started` | Native provider turn start observation. |
| `agent.turn.streamed` | Native provider stream observation. |
| `agent.turn.tool_requested` | Native provider tool/approval observation. |
| `agent.turn.artifact_captured` | Native provider artifact/diff observation. |
| `artifact.capture.failed` | Provider artifact capture failed before or during terminal completion. |
| `human.ask.created` | Human review request was created. |
| `human.answer.received` | Human answered an inbox item. |

## JSON inspection shapes

Field sets may grow. Consumers should ignore unknown fields.

### Event

```json
{
  "event_id": "evt_...",
  "instance_id": "inst_...",
  "sequence": 1,
  "event_type": "rule.committed",
  "payload": {},
  "occurred_at": "...",
  "source": "kernel",
  "causation_id": null,
  "correlation_id": null
}
```

### Fact

```json
{
  "fact_id": "fact_...",
  "program_version_id": "ver_...",
  "revision_epoch": 0,
  "name": "WorkItem",
  "key": "item-1",
  "value": {},
  "provenance_class": "rule",
  "source_span": null
}
```

### Effect

```json
{
  "effect_id": "effect-1",
  "kind": "agent.tell",
  "target": "worker",
  "status": "queued",
  "profile": "repo-writer",
  "policy_block_reason": null,
  "input": {}
}
```

### Run

```json
{
  "run_id": "run-...",
  "effect_id": "effect-1",
  "provider": "fixture",
  "worker_id": "whip-worker",
  "status": "completed",
  "started_at": "...",
  "completed_at": "..."
}
```

### Status

`whip --json status <instance>` returns instance metadata, aggregate counts,
recent events, and optional `workflow_invocations.parent` /
`workflow_invocations.children` links.

### Trace

`whip --json trace <instance> --check` returns:

```json
{
  "schema": "whipplescript.local_trace.v0",
  "instance_id": "inst_...",
  "events": [],
  "facts": [],
  "effects": [],
  "runs": [],
  "evidence": [],
  "evidence_links": [],
  "abstract_trace": [],
  "conformance": {"ok": true}
}
```

The trace report uses schema `whipplescript.local_trace.v0`; the draft JSON
Schema is validated by `scripts/check-report-schemas.sh`.

### Provider binding config

Provider binding config JSON is consumed by `whip doctor --provider-config`,
`whip worker --provider-config`, `whip dev --provider-config`, and
`scripts/check-native-provider-configs.sh`.

```json
{
  "provider_id": "codex-main",
  "provider_kind": "codex",
  "surface": "codex_app_server",
  "credentials_ref": "env:OPENAI_API_KEY",
  "profile_ids": ["repo-reader", "repo-writer"],
  "default_model": "gpt-5.4-mini",
  "workspace_policy": "read_only",
  "timeout_ms": 60000,
  "cancellation_depth": "native_stop",
  "artifact_policy": "required",
  "health_checks": ["codex_cli", "app_server_schema"]
}
```

Enums:

| Field | Values |
| --- | --- |
| `provider_kind` | `codex`, `claude`, `pi`, `fixture`, `command`, `baml`, `loft` |
| `surface` | `codex_app_server`, `claude_agent_sdk`, `pi_sdk`, `pi_rpc`, `fixture`, `command`, `baml_http`, `loft_cli` |
| `cancellation_depth` | `none`, `cooperative_request`, `native_stop`, `hard_process_stop`, `remote_session_cancel` |

Unknown config fields are preserved as `extra` for validation/reporting but
must not contain secret values.

Workers can discover config files through colon-separated
`WHIPPLESCRIPT_PROVIDER_CONFIGS`; `WHIPPLESCRIPT_NATIVE_PROVIDER_CONFIGS` is
accepted as a legacy alias. Source harness ids bind to config `provider_id`
values, and matching configs populate native request fields such as
`default_model`, `workspace_policy`, `cancellation_depth`, `artifact_policy`,
`credentials_ref`, `timeout_ms`, `profile_ids`, and health-check metadata.

### Provider capability JSON

The kernel exposes built-in capability descriptions for native and fixture
providers:

```json
{
  "provider_kind": "pi",
  "surface": "pi_rpc",
  "protocol_version": "pi-cli-rpc",
  "session_identity_fields": ["session_id", "parent_id"],
  "stream_event_kinds": ["agent_start", "turn_start", "message_start", "message_end", "turn_end", "agent_end"],
  "tool_policy": "pi_tools_extensions_skills",
  "cancellation_depths": ["none", "cooperative_request", "native_stop"],
  "artifact_manifest": true,
  "health_checks": ["pi_cli", "rpc_mode", "provider_model", "extensions"],
  "auth_requirements": ["pi_provider_api_key_or_auth_storage"]
}
```

Capability JSON is descriptive. Runtime policy must still validate a concrete
binding before a provider turn starts.

### Provider validation result

Provider validation results are redacted:

```json
{
  "provider": "codex-main",
  "surface": "codex_app_server",
  "status": "pass",
  "phase": "provider.surface.valid",
  "code": "surface_supported",
  "message": "provider kind and adapter surface are supported",
  "retryable": false,
  "missing_config_keys": []
}
```

`status` is `pass`, `fail`, or `skip`. Failures use `phase` values such as
`provider.config.invalid`, `provider.surface.unsupported`, or
`provider.config.missing`.

### Native lifecycle observation

Native provider observations normalize into `agent.turn.*` events and same-name
facts. Event payloads use this shape:

```json
{
  "effect_id": "tell",
  "run_id": "run-tell",
  "agent": "worker",
  "provider": "codex",
  "profile": "repo-writer",
  "status": "tool_requested",
  "provider_event_type": "item/tool/call",
  "provider_session_id": "thread-1",
  "provider_turn_id": "turn-1",
  "terminal": false,
  "provider_payload_shape": {"type": "object", "keys": 6},
  "evidence_id": "ev_..."
}
```

Canonical statuses:

```text
started
streamed
tool_requested
artifact_captured
completed
failed
timed_out
cancelled
```

The linked evidence kind is `agent.turn.native_event`. It stores provider event
type, session/turn ids, terminal flag, status, and redacted payload shape, not
raw provider payload text.

### Provider terminal metadata

`effect.terminal` payloads for provider runs include provider metadata under
`payload.metadata`. Agent terminal metadata includes redacted stdout, stderr,
transcript, usage shape, failure shape, provider correlation id, and terminal
payload hash:

```json
{
  "stdout": {"redacted": true, "bytes": 0, "chars": 0},
  "stderr": {"redacted": true, "bytes": 0, "chars": 0},
  "transcript": {"redacted": true, "bytes": 128, "chars": 128},
  "usage": {},
  "failure": null,
  "provider_correlation_id": "key_...",
  "terminal_payload_hash": "f8311b4ed0a2c641"
}
```

Recovery-generated terminal metadata wraps the persisted provider evidence:

```json
{
  "recovery": "provider_evidence_terminal",
  "evidence_id": "ev_...",
  "provider_metadata": {},
  "provider_correlation_id": "key_...",
  "terminal_payload_hash": "..."
}
```

The terminal idempotency key is derived from instance id, run id, provider
correlation id, terminal payload hash, and the terminal marker. The store still
rejects any second terminal completion for the same running run, even if the
second attempt has a distinct idempotency key.

Artifact metadata can be inspected without reading artifact contents:

```sh
whip --json artifacts <run-id>
```

The JSON response contains `run_id` and an `artifacts` array with artifact id,
kind, redacted path/ref, redacted content hash, MIME type, and creation time.
The command does not read or emit raw artifact files.
`whip --json runs <instance>` and `whip --json trace <instance>` include
`artifact_count` per run so operators can discover runs with artifact metadata
before calling `whip artifacts`.

### Artifact manifest

Provider evidence may include an artifact manifest:

```json
{
  "schema_version": "whipplescript.artifact_manifest.v1",
  "entry_count": 1,
  "entries": [
    {
      "artifact_id": "art_...",
      "kind": "transcript",
      "uri": {
        "type": "ref",
        "value": "provider://codex/runs/run-tell/transcript_ref"
      },
      "content_hash": {
        "algorithm": "provider",
        "value": "..."
      },
      "mime_type": "text/plain",
      "size_bytes": null,
      "redaction_status": "unredacted_metadata_only",
      "retention_policy": "provider_default",
      "required": false,
      "source_provider_event": null
    }
  ]
}
```

Allowed `uri.type` values are `path` and `ref`. Allowed `redaction_status`
values are `redacted`, `unredacted_metadata_only`, and `reference_only`.
Allowed `retention_policy` values are `ephemeral`, `provider_default`,
`retain`, and `delete_after_run`.

### Artifact capture failure

Artifact capture failures append `artifact.capture.failed`:

```json
{
  "event_type": "artifact.capture.failed",
  "provider": "codex",
  "adapter": "codex_app_server",
  "run_id": "run-tell",
  "artifact_ref": {
    "type": "ref",
    "value": "provider://codex/runs/run-tell/diff"
  },
  "error_kind": "missing",
  "recoverable": true,
  "message": {"redacted": true, "bytes": 24, "chars": 24},
  "transcript_ref": "provider://codex/runs/run-tell/transcript_ref",
  "stderr_ref": null
}
```

Allowed `error_kind` values are `missing`, `unreadable`, `oversized`,
`hash_mismatch`, and `redaction_failed`. Required artifact capture failure
prevents a provider result from being marked successful.

## Rust crate APIs

The Rust APIs are currently internal-stability APIs for the workspace. They are
useful for integration tests and local tooling, but should not be treated as a
published semver contract yet.

### `whipplescript-core`

| Item | Meaning |
| --- | --- |
| `version()` | Compiler/runtime version string. |
| `IMPLEMENTATION_STAGE` | Current stage label. |

### `whipplescript-parser`

Primary entrypoints:

| Item | Meaning |
| --- | --- |
| `parse_program(source)` | Parse source into AST plus diagnostics. |
| `compile_program(source)` | Parse/type-check/lower source into `IrProgram`. |
| `compile_program_with_root(source, root)` | Compile a source bundle with explicit root selection. |
| `format_program(source)` | Format source while preserving rule/coerce block bodies. |
| `parse_expression(expr)` | Parse a guard/assertion expression. |
| `parse_duration_seconds(value)` | Parse supported duration literal to seconds. |
| `parse_time_epoch_seconds(value)` | Parse supported timestamp literal to epoch seconds. |

Important AST/IR structs include:

```text
Program
WorkflowDecl
WorkflowContractDecl
PatternDecl
ApplyDecl
IncludeDecl
UseDecl
AgentDecl
EnumDecl
ClassDecl
CoerceDecl
RuleDecl
WhenClause
IrProgram
IrWorkflowContract
IrPatternApplication
IrAssertion
IrUse
IrSchema
IrAgent
IrCoerce
IrRule
IrEffectNode
IrEffectDependency
IrTerminalOutput
Expr
```

### `whipplescript-store`

`SqliteStore` owns durable runtime persistence.

Lifecycle and program methods:

| Method | Meaning |
| --- | --- |
| `open(path)` / `open_in_memory()` | Open store and apply migrations. |
| `schema_version()` | Read applied schema version. |
| `create_program_version(...)` | Create or find program version metadata. |
| `create_instance(...)` | Create a running instance. |
| `transition_instance(...)` | Pause/resume/cancel with transition guards. |
| `status(instance_id)` | Aggregate instance status view. |
| `list_instances()` / `get_instance()` | Instance inspection. |

Rule/effect methods:

| Method | Meaning |
| --- | --- |
| `append_event(...)` | Append raw event. |
| `commit_rule(...)` | Atomic rule commit with facts, effects, dependencies, and optional workflow terminal action. |
| `derive_fact(...)` | Derive fact from an event/projection. |
| `claimable_effects(instance_id)` | List effects ready for worker execution. |
| `satisfy_dependencies(instance_id)` | Release dependency-blocked effects whose predicates are satisfied. |
| `start_run(...)` | Start provider run and active lease. |
| `complete_effect(...)` | Mark running run/effect terminal. |
| `complete_effect_with_terminal_diagnostic(...)` | Terminal completion with diagnostic capture. |
| `cancel_effect(...)` | Cancel an effect. |
| `renew_lease(...)` / `expire_leases(...)` | Lease maintenance. |
| `retry_effect(...)` | Retry failed/timed-out effect. |

Inspection methods:

```text
list_events
list_facts
list_facts_including_consumed
list_effects
list_runs
list_evidence
list_evidence_links
list_diagnostics
list_diagnostics_from_events
list_artifacts_for_run
```

Registry and extension methods:

```text
register_plugin
register_plugin_manifest
load_plugin_manifests_from_dir
register_capability_schema
register_effect_provider
register_profile
bind_capability
register_skill
attach_skill
list_skills
list_skill_attachments
record_skill_evidence
```

Human review methods:

```text
create_inbox_item
list_inbox_items
get_inbox_item
answer_inbox_item
```

Workflow invocation methods:

```text
record_workflow_invocation
get_workflow_invocation
list_child_workflow_invocations
get_parent_workflow_invocation
```

### `whipplescript-kernel`

`RuntimeKernel` wraps store operations and emits trace records.

Core methods:

```text
create_program_version
create_program_version_for_program
create_instance
ingest_external_event
derive_fact
evaluate_rules
commit_rule
claimable_effects
satisfy_dependencies
start_run
complete_run
fail_run
timeout_run
cancel_run
cancel_effect
pause_instance
resume_instance
cancel_instance
renew_lease
expire_leases
retry_effect
```

Provider execution methods:

```text
run_agent_turn
record_native_agent_turn_observation
record_artifact_capture_failure
recover_provider_terminal_from_evidence
recover_running_provider_runs
run_baml_coerce
run_loft_effect
run_human_ask
```

Provider traits and helpers:

| Item | Meaning |
| --- | --- |
| `AgentHarness` | Agent provider adapter trait. |
| `CommandAgentHarness` | Command-backed harness for local adapters. |
| `CodexAgentHarness` | Codex adapter wrapper over command launch plan. |
| `ClaudeCodeAgentHarness` | Claude Code adapter wrapper over command launch plan. |
| `PiStyleAgentHarness` | Pi-style adapter wrapper over command launch plan. |
| `MockAgentHarness` | Deterministic test harness. |
| `BamlClient` / `HttpBamlClient` / `FakeBamlClient` | BAML coerce provider abstraction. |
| `LoftClient` / `CommandLoftClient` / `FakeLoftClient` | Loft effect provider abstraction. |

Native provider modules:

| Module | Meaning |
| --- | --- |
| `provider` | Provider capability/config validation and built-in native capabilities. |
| `codex_app_server` | Codex app-server transport and evidence summaries. |
| `claude_agent_sdk` | Claude Agent SDK sidecar client, policy mapping, and evidence summaries. |
| `pi_rpc` | Pi RPC client, policy mapping, and event summaries. |
| `native_lifecycle` | Codex/Claude/Pi event normalization into `agent.turn.*`. |
| `artifact_manifest` | Artifact manifest and capture-failure payload helpers. |

Trace API:

| Item | Meaning |
| --- | --- |
| `TraceEvent` | Abstract lifecycle event. |
| `TraceRecord` | Sequenced abstract event. |
| `check_trace(records)` | Validate trace conformance. |

## Formal and release checks

Common root checks:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
scripts/check-formal-models.sh
scripts/check-tla-models.sh
scripts/check-e2e.sh
scripts/check-release-readiness.sh
```

`scripts/check-formal-models.sh` runs Maude checks and the TLA check wrapper.
`scripts/check-tla-models.sh` runs Apalache type checking and bounded safety.
`scripts/check-e2e.sh` runs deterministic fixture-provider integration tests.
