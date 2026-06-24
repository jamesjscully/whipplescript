# JSON Reference

This page is the public machine-readable contract surface for the CLI. Field
sets may grow within the same schema version; consumers should ignore unknown
fields. Required fields listed here are safe to depend on for the current
`main` documentation set and released `0.1.x` CLI unless a schema version
changes.

For command usage, see [CLI reference](api-reference.md).

## Stability Model

| Surface | Schema/version | Stability |
| --- | --- | --- |
| `check --json` | `whipplescript.check_report.v0` | Draft public report. Required fields are stable; additional fields may appear. |
| `compile --json` | `whipplescript.compile_report.v0` | Draft public report. Required fields are stable; additional fields may appear. |
| `dev --json` | `whipplescript.dev_report.v0` | Draft public report. Intended for acceptance tests and local tooling. |
| `dev --stream ndjson` | `whipplescript.dev_stream.v0` | Draft event envelope. Event names may grow. |
| `accept --json` | `whipplescript.acceptance_report.v0` | Test-only report. Store-isolation expectations are part of the contract. |
| `trace --json --check` | `whipplescript.local_trace.v0` | Draft trace/conformance report. |
| Package manifest | `whipplescript.package_manifest.v0` | First-class package/library/provider manifest. |
| Platform construct catalog | `whipplescript.platform_construct_catalog.v0` | Compiler-owned package construct/lowering vocabulary emitted by `whip package catalog`. |
| Package check | `whipplescript.package_check.v0` | Result of `whip package check --json`. |
| Package contract | `whipplescript.package_contract.v0` | Digest-bearing normalized package/registry artifact used by check, compile, and verified artifact reports. |
| Package lock | `whipplescript.package_lock.v0` | Pins accepted package manifests by exact SHA-256. |
| Artifact manifest | `whipplescript.artifact_manifest.v1` | Provider artifact metadata contract. |
| Inspection commands | command-shaped JSON | Stable enough for operators; no schema id yet. |
| Coordination inspection | command-shaped JSON | Stable enough for operators; no schema id yet. |

JSON Schemas for the versioned report envelopes live in
[`spec/report-schemas/`](https://github.com/jamesjscully/whipplescript/tree/main/spec/report-schemas). Validate them with
`scripts/check-report-schemas.sh`.

## Required Fields

### Check Report

Successful `whip --json check <workflow.whip>` entries require:

```json
{
  "schema": "whipplescript.check_report.v0",
  "path": "examples/minimal-noop.whip",
  "status": "ok",
  "workflow": "MinimalNoop",
  "source_hash": "...",
  "ir_hash": "...",
  "snapshot": "...",
  "source_metadata": {
    "tags": [],
    "descriptions": [],
    "targets": {}
  },
  "contract_registry": {
    "schema": "whipplescript.contract_registry.v0",
    "libraries": [],
    "declaration_forms": [],
    "effect_contracts": [],
    "diagnostics": []
  },
  "package_contract": {
    "schema": "whipplescript.package_contract.v0",
    "package_contract_digest": "...",
    "package_lock_digest": "...",
    "contract_registry": {}
  },
  "construct_graph": {
    "schema": "whipplescript.construct_graph.v0",
    "package_contract_digest": "...",
    "nodes": [],
    "edges": [],
    "derived_facts": [],
    "diagnostics": []
  }
}
```

Error entries require `schema`, `path`, `status: "error"`, and an `error`
object. Error kinds include `io`, `diagnostics`, `package_lock`, and
`construct_graph`. Diagnostics include a message and source span when
available.

`package_contract` is the normalized package/registry artifact. It carries the
locked manifest summaries, platform construct catalog, contract registry, and a
`package_contract_digest` over that body. `construct_graph` is the normalized
static composition artifact for the checked program and cites the same
`package_contract_digest`, so report verification can reject graphs checked
against a stale package contract. In the current executable slice, locked
package capability calls emit effect-operation nodes, package effect-contract
nodes, and resolved capability edges. `timer.wait` nodes advertise
`schedule_template` output. Core rule-template nodes also advertise rule-owned
`fact_record` templates when the rule body can record facts. Empty
graphs are valid for programs that do not use package-backed constructs yet.
Accepted graphs include
`derived_facts` owned by `construct_graph_validator` for the structural graph
predicates checked by the current validator.

### Compile Report

`whip --json compile <workflow.whip>` requires:

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
  },
  "contract_registry": {
    "schema": "whipplescript.contract_registry.v0",
    "libraries": [],
    "declaration_forms": [],
    "effect_contracts": [],
    "diagnostics": []
  },
  "package_contract": {
    "schema": "whipplescript.package_contract.v0",
    "package_contract_digest": "...",
    "package_lock_digest": "...",
    "contract_registry": {}
  }
}
```

`contract_registry` is the compiler's normalized view of library/effect
contracts used by the program. Standard built-in surfaces are reported as
`std.*` libraries with version `v0`. Package imports resolved through a package
lock appear with the pinned package version; an unlocked source import appears
with version `unlocked` until the lock supplies the package contract.
`declaration_forms` describe library-owned source forms registered by locked packages.
`metadata_only` forms are tooling metadata.
The accepted executable target today is `capability_call`, which lowers a
fixed core-owned form to the named `target_capability`; for example, the memory
package can authorize `recall from <pool> for <query> as <binding>` as a
memory recall capability call. `effect_contracts` describe the source forms,
required capabilities, provider families, output schema, and runtime validation
boundary for each effect kind. For locked package capability calls,
`runtime_boundary` is enforced by the worker before success facts are derived.
Package declaration forms use platform-owned lowering classes. New lowering
targets or non-`capability.call` package effects require a platform extension
class before they can appear in a manifest.

### Package Lock

`whip package lock --output whip.lock <manifest.json>...` writes:

```json
{
  "schema": "whipplescript.package_lock.v0",
  "packages": [
    {
      "package_id": "package-memory",
      "name": "memory",
      "version": "0.1.0",
      "manifest_path": "/abs/path/examples/packages/memory.json",
      "manifest_sha256": "..."
    }
  ]
}
```

`check`, `compile`, `run`, `dev`, and `worker` reject a lock entry when the
manifest name, version, package id, or SHA-256 no longer matches. A lock may not
contain duplicate package ids or package names; source imports resolve through
package name, so each locked package name is unique.

### Trace Report

`whip --json trace <instance> --check` requires:

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

## Inspection Shapes

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

Required: `event_id`, `instance_id`, `sequence`, `event_type`, `payload`,
`occurred_at`, `source`.

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

Required: `fact_id`, `name`, `value`, `provenance_class`. Facts derived from
`table` declarations include `provenance_class: "table"` and a row
`source_span`.

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

Required: `effect_id`, `kind`, `status`, `input`.

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

Required: `run_id`, `effect_id`, `provider`, `worker_id`, `status`,
`started_at`.

### Diagnostic

Runtime diagnostics returned by `diagnostics --json` require:

```json
{
  "diagnostic_id": "diag_...",
  "instance_id": "inst_...",
  "severity": "error",
  "code": "provider.failure",
  "message": "provider run failed",
  "source_span": null,
  "event_id": "evt_...",
  "effect_id": "effect-...",
  "run_id": "run-..."
}
```

Optional fields link to program, version, assertion, evidence, artifact,
causation, and correlation records when available.

## Coordination Inspection

`leases --json` returns:

```json
[
  {
    "resource": "deploy_slot",
    "key": "prod",
    "holder": "inst_...",
    "acquired_at": "...",
    "expires_at": "..."
  }
]
```

`ledger --json` returns:

```json
[
  {
    "ledger": "decisions",
    "partition": "incident",
    "seq": 1,
    "entry": {},
    "appended_by": "inst_...",
    "appended_at": "..."
  }
]
```

`counters --json` returns:

```json
[
  {
    "counter": "budget",
    "key": "customer-1",
    "consumed": 42,
    "period": "2026-06-11"
  }
]
```

These commands read `WHIPPLESCRIPT_COORDINATION_STORE` or
`.whipplescript/coordination.sqlite`.

## Status Values

Instance status:

```text
running
paused
completed
failed
cancelled
```

Effect status:

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

Run status:

```text
running
completed
failed
timed_out
cancelled
lease_expired
```

Lease status:

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
| `signal.emit.completed` | In-workflow typed signal injection completed. |

## Provider And Artifact Shapes

Provider binding config, provider validation result, native lifecycle
observation, provider terminal metadata, artifact manifests, and artifact
capture failures are JSON contracts for operators and provider authors. They
are documented with the provider model in [Providers & packages](providers.md)
and with schema details in `spec/reporting.md`, `spec/observability.md`, and
`spec/report-schemas/`.
