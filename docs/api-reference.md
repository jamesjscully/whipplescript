# WhippleScript API Reference

Status: draft

This reference catalogs the currently implemented surfaces: the `.whip`
language, CLI commands, runtime events/statuses, JSON inspection shapes, and
Rust crate APIs. It is intentionally factual; design rationale belongs in
`spec/`.

## Global CLI Options

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
check, compile, run, step, worker, dev, instances, status, log, facts, effects,
runs, inbox, evidence, diagnostics, trace, pause, resume, cancel, retry, doctor
```

## CLI Commands

### `doctor`

```sh
whip doctor
whip --json doctor
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
loft
```

### `check`

```sh
whip check [--model-search] [--root Workflow] <workflow.whip>...
```

Parses, resolves includes, type-checks, lowers to IR, and prints the IR snapshot.
With `--model-search`, also runs generated Maude checks when available.

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
  "path": "examples/minimal-noop.whip",
  "workflow": "MinimalNoop",
  "source_hash": "...",
  "ir_hash": "...",
  "snapshot": "..."
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
  [--program <workflow.whip>] \
  [--root Workflow] \
  [--once] \
  [--fail | --timeout | --cancel] \
  [--max-child-iterations N]
```

Claims currently claimable effects and completes them through the selected
provider. The default provider is the deterministic fixture provider. `--fail`,
`--timeout`, and `--cancel` force fixture terminal outcomes for failure-path
tests.

Supported fixture effect kinds:

```text
agent.tell
baml.coerce
loft.claim
human.ask
capability.call
event.emit
workflow.invoke
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
  [--until idle] \
  [--max-iterations N] \
  [--fail | --timeout | --cancel]
```

Convenience local validation loop. It starts a new instance, alternates `step`
and `worker`, stops when idle or when `--max-iterations` is reached, then
evaluates source assertions.

JSON output includes the instance id, workflow name, per-iteration step reports,
worker reports, and assertion reports.

### Inspection Commands

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

### Inbox Commands

```sh
whip inbox
whip inbox show <item>
whip inbox answer <item> --choice <value> [--by NAME]
whip inbox answer <item> --text <value> [--by NAME]
```

Inbox commands inspect and answer human review requests created by `human.ask`
effects.

### Lifecycle Commands

```sh
whip pause <instance>
whip resume <instance>
whip cancel <instance>
whip retry <instance> <effect>
```

| Command | Meaning |
| --- | --- |
| `pause` | Transition a running instance to paused. |
| `resume` | Transition a paused instance back to running. |
| `cancel` | Transition a running/paused/blocked instance to terminal cancelled. |
| `retry` | Move an eligible failed or timed-out effect back to queued. |

Terminal instances are absorbing: completed, failed, and cancelled instances do
not accept further public lifecycle transitions or rule commits.

## Language Reference Index

For examples and semantics, see [Language Reference](language-reference.md).
This section is a compact index of source constructs.

### Top-Level Constructs

| Construct | Surface | Meaning |
| --- | --- | --- |
| Workflow | `workflow Name { ... }` or `workflow Name` | Deployable runtime boundary. |
| Contract | `input name Type`, `output name Type`, `failure name Type` | Typed workflow input/output/failure contract. |
| Include | `include "path.whip"` | Source bundle composition. |
| Plugin import | `use memory` | Import plugin by name. |
| Class | `class Name { field Type }` | Typed fact and payload schema. |
| Enum | `enum Name { A B }` | Finite string domain. |
| Agent | `agent name { profile "..."; capacity N; skills [...] }` | Logical provider target and policy metadata. |
| Coerce | `coerce fn(args...) -> Type { prompt """...""" }` | Declared BAML-backed effect. |
| Pattern | `pattern Name<T> { ... }` | Compile-time reusable fragment. |
| Apply | `apply Name<Type> as Alias { ... }` | Pattern specialization. |
| Assertion | `assert expression` | Deterministic projection check in `dev`. |

### Rule Constructs

| Construct | Surface | Meaning |
| --- | --- | --- |
| Rule | `rule name ... => { ... }` | Atomic deterministic rewrite. |
| Fact match | `when Class as binding` | Bind an unconsumed fact. |
| Guarded match | `when Class as binding where expr` | Bind fact only when pure guard is true. |
| Started event | `when started` | Match the initial `external.started` event. |
| Availability | `when worker is available` | Match logical agent capacity/policy availability. |

### Rule Body Operations

| Operation | Effect/commit output |
| --- | --- |
| `record Class { ... }` | New fact. |
| `record Class from binding { ... }` | New fact with copied fields. |
| `consume binding` / `done binding` | Mark matched fact consumed. |
| `done binding -> record ...` | Consume and create replacement fact atomically. |
| `tell agent ... as turn` | `agent.tell` effect. |
| `coerce fn(...) as result` | `baml.coerce` effect. |
| `claim issue with loft as claim` | `loft.claim` effect. |
| `askHuman ...` | `human.ask` effect. |
| `call capability for value as result` | `capability.call` effect. |
| `emit event.name as event` | `event.emit` effect. |
| `invoke Workflow { ... } as child` | `workflow.invoke` effect. |
| `after effect succeeds/fails/completes` | Dependency branch scoped by terminal status. |
| `case expr { Pattern => { ... } }` | Deterministic finite-domain branch. |
| `complete output { ... }` | `workflow.completed` event and terminal completed state. |
| `fail failure { ... }` | `workflow.failed` event and terminal failed state. |

## Status Values

### Instance Status

```text
created
running
paused
blocked
completed
failed
cancelled
```

`completed`, `failed`, and `cancelled` are terminal.

### Effect Status

```text
queued
blocked_by_dependency
blocked_by_capacity
blocked_by_policy
claimed
running
completed
failed
timed_out
cancelled
```

### Run Status

```text
running
completed
failed
timed_out
cancelled
lease_expired
```

### Lease Status

```text
active
released
expired
```

## Event Types

Common event types:

| Event | Meaning |
| --- | --- |
| `external.started` | Instance start input event. |
| `rule.committed` | Rule atomically committed facts/effects/dependencies/terminal action. |
| `effect.run_started` | Provider run started for an effect. |
| `effect.terminal` | Effect completed, failed, timed out, or cancelled. |
| `effect.blocked` | Effect blocked before provider start. |
| `effect.retry_requested` | Effect returned to queued for retry. |
| `lease.expired` | Active run lease expired. |
| `instance.transitioned` | Pause/resume/cancel transition. |
| `workflow.completed` | Workflow produced declared output and became completed. |
| `workflow.failed` | Workflow produced declared failure and became failed. |
| `agent.turn.completed` | Agent turn completion projection. |
| `agent.turn.failed` | Agent turn failure projection. |
| `agent.turn.timed_out` | Agent turn timeout projection. |
| `agent.turn.cancelled` | Agent turn cancellation projection. |
| `human.answered` | Human answered an inbox item. |

## JSON Inspection Shapes

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
  "name": "WorkItem",
  "key": "item-1",
  "value": {},
  "source_event_id": "evt_...",
  "source_rule": "seed"
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

`status --json` returns instance metadata, aggregate counts, recent events, and
optional `workflow_invocations.parent` / `workflow_invocations.children` links.

### Trace

`trace --json --check` returns:

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

## Rust Crate APIs

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

Trace API:

| Item | Meaning |
| --- | --- |
| `TraceEvent` | Abstract lifecycle event. |
| `TraceRecord` | Sequenced abstract event. |
| `check_trace(records)` | Validate trace conformance. |

## Formal And Release Checks

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
