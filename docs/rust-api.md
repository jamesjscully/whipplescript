# Rust API Reference

The Rust crates are internal-stability APIs for the workspace. They are useful
for integration tests, local tooling, and contributors, but they are not a
published semver contract. Users automating WhippleScript should prefer the CLI
and the JSON contracts in [JSON reference](json-reference.md).

One exception is the revision-pinned native host surface used by GaugeDesk and
other embedding hosts. It is intentionally pre-1.0 and must be pinned to an
exact commit, but its types are public so hosts do not reimplement governance,
IFC, or turn lifecycle semantics.

## `whipplescript` host library

`host_protocol` defines the placement-neutral `whipplescript.host.v1` wire
types. `OpenInstanceCommand` opens one durable WhippleScript instance for a
host chat. `StartTurnCommand`, `LabeledRuntimeEvent`, and `TurnReceipt` carry
the same verified `PolicyEpochRef`; commands contain resource/provider refs,
not resource bodies or credentials.

`host_runtime::GovernedHostRuntime` is the native persistent facade:

| Item | Meaning |
| --- | --- |
| `open(store_path, epoch, signed_envelope)` | Open/reopen SQLite and bind the runtime to the exact verified policy epoch. |
| `open_with_verifier(store_path, epoch, signed_envelope, verifier)` | Open an embedded runtime under an externally signed envelope; requires the host's pinned `GovernanceAttestationVerifier` and never consults process-global admin state. |
| `open_instance(command, packages)` | Resolve a pinned package and issue a durable WhippleScript instance ref. |
| `run_turn(...)` | Run the owned brokered loop with the native HTTP driver, persistent transcript, evidence projection, and terminal receipt. |
| `run_turn_with_driver(...)` | Drive the same sans-I/O machine with a host-supplied transport (tests and remote placements). |
| `TurnExecution::output` / `LabeledTurnOutput` | WhippleScript-folded assistant/tool projection carrying the turn's IFC join label; hosts never inspect the runtime store or recreate transcript folding. |
| `PackageResolver` | Resolve immutable WhippleScript package bytes/IR and its package-declared tool schemas. |
| `SecretResolver` | Resolve provider credentials ephemerally, after policy admission. |
| `ResourceResolver` | Resolve image bytes and execute package-declared tools against only the resource refs admitted for the turn. |
| `NativeWorkspaceResolver` / `native_workspace_tool_specs` | WhippleScript-owned native file capability: confined, symlink-refusing read/write/edit/grep/find/list operations with host-supplied read-only subtrees. |

The facade fails closed unless the signed envelope governs every resource,
provider binding, and placement handle. `ResolvedPackage::compile` retains the
pinned program IR, and instance/turn admission runs WhippleScript's IFC checker
over that IR under the verified envelope before any secret is resolved. The
facade binds instances to package content hashes and policy identity, rejects
cross-binding reuse, and persists only references/evidence—not resolved
provider secrets.

Embedding authorities create the exact bytes to sign with
`gov::external_signing_bytes`, attach the result with
`SignedEnvelope::from_external_signature`, and verify through
`GovernanceAttestationVerifier`. The external key id is carried on
`PolicyEpochRef`, so command/event/receipt anti-mixup binds the cryptographic
trust root as well as the envelope hash, epoch, and signer. The legacy
hash-attested `whip gov` path remains root/admin gated and cannot verify an
external artifact without its pinned verifier.

## `whipplescript-core`

| Item | Meaning |
| --- | --- |
| `version()` | Workspace package version string. |
| `IMPLEMENTATION_STAGE` | Current implementation-stage label printed by the CLI. |

## `whipplescript-parser`

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
HarnessDecl
QueueDecl
FlowDecl
AgentDecl
EnumDecl
EventDecl
LeaseDecl
LedgerDecl
CounterDecl
ClassDecl
TableDecl
CoerceDecl
AssertDecl
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

## `whipplescript-store`

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
register_package
register_package_manifest
load_package_manifests_from_dir
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

## `whipplescript-kernel`

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
run_coerce
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
| `CoerceClient` / `FakeCoerceClient` | coerce provider abstraction. |

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
