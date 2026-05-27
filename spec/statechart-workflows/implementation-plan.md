# Statechart Workflow Implementation Plan

Status: working plan

This plan starts from the current Rust statechart runtime work, not from a blank
repository. The implementation direction is now:

```text
native .armature statechart DSL
small orchestration expression kernel
Rust parser, validator, interpreter, modelgen, and CLI
SQLite durable queue/log/state/coerce/agent-invocation storage
BAML generated stdio by default for real coerce execution
native local harness for agent execution
adapter manifests only for explicitly external non-native effects
TLA+/Maude generated checks, with Veil later
```

The north star is a workflow scripting language for agent orchestration that
feels natural to coding agents and humans without granting arbitrary
programming-language authority.

## Current Baseline

The active implementation already has these pieces:

- native `.armature` parser/lowering scaffold using `logos` and `rowan`
- WorkflowIR structs, schema validation, and source diagnostics
- support for `machine`, `initial`, `data`, `event`, `agent`, `capability`,
  `enum`, `class`, `coerce`, nested `state`, `on`, `entry`, `always`, `case`,
  `let`, `assign`, `start`, `send`, `askHuman`, `raise`, `stay`, `goto`, and
  invariants over the implemented subset
- agent targets include `thread`, `codingAgent`, and adapter-backed agents;
  validation rejects starting thread-only agents
- SQLite workflow store with event queue, state, transition logs, effect logs,
  durable coerce call records, recovery for processing events, and schema
  version metadata
- interpreter support for synthetic events, parent-state event fallback,
  entry/always loop protection, fake adapter dispatch, fake coerce executor
  outputs, transitional BAML HTTP coerce execution, fake capability value outputs, and
  adapter-backed capability value calls
- adapter manifest validation and runtime policy enforcement for adapter-backed
  effects
- generated TLA+ and Maude model targets for the current control-state and
  active-invocation abstraction
- CLI commands for validate, emit, run, status, overview, events, log, build,
  check, emit-model, emit-config, prove, validate-adapter, and validate-policy
- `prove` validates contracts and runs the current generated verification
  bundle, TLA+ plus Maude
- deterministic e2e coverage around the CLI/runtime boundary using fake outputs
- CLI `run --baml-url` support for calling an already-running external BAML
  HTTP endpoint as an override
- `run --baml-url` policy enforcement for `baml.coerce`, BAML network access,
  and exact URL allowlists
- current managed BAML implementation starts `baml-cli serve` and records
  `runtime_mode: managed_http`; this is now considered a transitional
  implementation because it requires local TCP listener authority that coding
  agent sandboxes may deny
- target managed BAML mode is generated stdio: write generated BAML source,
  generate/load a BAML client runner, pass named JSON over stdin/stdout, capture
  artifacts, and record `runtime_mode: generated_stdio`
- generated stdio supports `--baml-auth api-key` and explicit
  `--baml-auth codex-oauth`; Codex OAuth reads existing Codex credentials and
  injects only runner-scoped `ARMATURE_CODEX_OAUTH_*` environment variables
- runtime `coerce` persistence/replay through durable coerce records
- runtime BAML backend records include generated BAML source hashes and stable
  interpreter step locators such as `handler.0`, `entry.0.0`, and
  `always.guard`
- expression validation/runtime support for the v1 primitive helpers in
  [expression-primitives.md](expression-primitives.md), including list, map,
  text, time, coerce, and capability value calls
- status projection for workflow data, summarized workflow data, latest coerce
  calls, current coerce failure, historical latest coerce failures,
  current effect failures, current blockers, and policy blockers from durable
  storage
- opt-in real BAML HTTP e2e coverage gated by `ARMATURE_RUN_BAML_E2E=1` and
  `ARMATURE_BAML_URL`
- first scoped JSON plan file adapter slice through `run --plan-file`, covering
  plan snapshot reads and task status updates for ready-for-quality, done, and
  blocked; plan-only workflows get a built-in JSON plan manifest automatically
- first human-review bridge through `run --review-file`, covering durable JSON
  review obligation creation for `askHuman`, plus typed response event intake
  through `emit --review-file`
- native local agent/thread bridge through the SQLite agent ledger and harness,
  covering durable `start` invocations, provider claims, stdout/stderr
  artifacts, completion records, typed `finished` event enqueue, and harness
  status; the old JSON agent-file bridge is hidden fixture scaffolding only

Known major gaps:

- expression validation implements optional-presence proof for direct nil
  comparisons, conjunctions, shared non-null facts across disjunctions,
  De Morgan-style negated disjunctions, double negation, and ordered
  case-pattern refinement; it intentionally does not yet attempt full SAT-style
  boolean reasoning
- generated formal models do not yet include workflow data or expression
  invariants
- native durable agent invocation storage and harness execution are implemented
  through the SQLite ledger; any remaining JSON agent bridge should be treated
  as fixture/debug scaffolding rather than product surface
- provider command execution supports harness profile policy so workflow
  authors choose semantic profiles such as `research` and `repo-writer` while
  operators control concrete commands, filesystem/network posture, environment
  allowlists, timeout, and enforcement mode
- Codex and Claude Code provider presets map selected profile authority into
  native CLI flags where available; Pi remains best-effort until its stable
  sandbox flags are documented and tested
- the provider-adapter target is now specified as a language/runtime boundary:
  `codingAgent` declares an abstract role, harness profile policy binds that
  role to a provider, and provider adapters implement concrete Codex, Claude
  Code, Pi, or command launches
- fuller plan/state adapters remain future work
- BAML runtime status projection exists as a derived coerce-ledger projection;
  dedicated runtime records remain future work

## Immediate Track: Native Agent Ledger And Harness

Goal: replace the JSON agent-file bridge with a first-class durable harness
foundation. Backwards compatibility is not an objective. The implementation
should optimize for the right architecture before other features depend on the
old bridge.

Former shape, now quarantined as hidden fixture scaffolding:

```text
workflow runtime -> adapter dispatcher -> agents.json
harness/script -> armature emit --fixture-agent-file -> workflow_events
```

Target shape:

```text
workflow runtime -> SQLite agent_invocations
harness runner -> SQLite agent_completions + workflow_events
workflow runtime -> processes queued completion events
```

The event-sourced part is the persisted ledger. The harness may still poll the
database for queued work in the MVP; polling is a wakeup strategy, not a data
model.

### Design Commitments

- `start` and `send` for declared local agents are native runtime operations,
  not adapter-manifest effects.
- Agent invocations are durable rows in the workflow SQLite database.
- The harness claims invocations from SQLite with transactional leases.
- Provider stdout/stderr and metadata are stored as durable run artifacts.
- Provider completion writes both an agent completion record and a typed
  workflow event, usually `finished`.
- `armature emit` remains useful for manual external events, but the harness
  must not shell out to `armature emit` for normal completion.
- `--agent-file` is removed from normal UX and documentation. If any JSON bridge
  code survives temporarily, it must be named and documented as a fixture/debug
  helper with a concrete use case.

### 0.1 Respec The Runtime Boundary

Update the specs before code changes:

- [runtime-semantics.md](runtime-semantics.md): define native `start` and `send`
  persistence semantics.
- [effects.md](effects.md): classify local agent invocation separately from
  adapter-backed effects.
- [storage.md](storage.md): add native agent ledger tables and schema version
  expectations.
- [component-contracts.md](component-contracts.md): add the harness contracts and
  remove file-backed local agent records from the main contract path.
- [operations.md](operations.md): replace `emit --agent-file` recovery guidance
  with ledger/harness commands.
- [product-surface.md](product-surface.md): define the intended CLI UX for
  `harness once`, `harness run`, and `harness status`.

Exit criteria:

- no spec presents `--agent-file` as the product path
- the exact tables and status transitions are specified
- completion event payload shape is explicit
- native harness behavior is distinguishable from generic adapter behavior

### 0.2 Add Agent Ledger Storage

Work in `crates/armature-engine` storage:

- bump the SQLite schema version
- add tables:
  - `agent_invocations`
  - `agent_messages`
  - `agent_completions`
  - `harness_events`
- add indexes for queued work, active work, claims, leases, provider run ids,
  and recent harness events
- add store methods:
  - `insert_agent_invocation(record)`
  - `insert_agent_message(record)`
  - `queued_agent_invocations(workflow_id, limit)`
  - `claim_agent_invocation(invocation_id, worker_id, lease_until)`
  - `mark_agent_invocation_started(...)`
  - `mark_agent_invocation_exited(...)`
  - `record_agent_completion(...)`
  - `append_harness_event(...)`
  - `recover_expired_agent_leases(now)`

Exit criteria:

- storage tests cover schema migration, insert, claim, duplicate claim
  prevention, lease recovery, completion recording, and recent event queries
- existing workflow stores fail closed on unsupported future schema versions

### 0.3 Make `start` And `send` Native

Work in `crates/armature-engine`:

- route `start` for `codingAgent`/local provider agents to the native ledger
  instead of the manifest dispatcher
- route `send` to durable `agent_messages`
- append effect log records that point at the native invocation/message ids
- make event processing transactionally persist state, logs, and invocation
  records together
- project `active_invocations` from native invocation/completion rows instead
  of JSON agent-file effects
- keep adapter-backed agents possible only when a workflow explicitly targets an
  adapter-backed capability

Exit criteria:

- running a workflow with `start worker` creates a queued `agent_invocations`
  row without any adapter manifest or JSON file
- bounded `maxActive` checks use native invocation state
- deterministic runtime tests cover duplicate replay/idempotency

### 0.4 Add Harness Commands

Work in `crates/armature-cli` and, if the code gets large, a new harness module
or crate:

```text
armature harness once <workflow.armature> --store <path> --config <path>
armature harness run <workflow.armature> --store <path> --config <path>
armature harness status <workflow.armature> --store <path>
```

`once`:

- claims at most one queued invocation
- runs the configured provider
- records stdout/stderr and exit metadata
- records completion
- enqueues the typed completion event
- exits with a useful code

`run`:

- loops over `once`
- recovers expired leases
- optionally drains queued workflow events between harness ticks in a later
  slice
- handles shutdown without corrupting claims

`status`:

- shows queued, claimed/running, completed, failed, and recently retried agent
  work
- joins this with workflow state, active invocations, current blockers, and
  recent failures

Exit criteria:

- fake-provider e2e covers a complete start -> harness completion -> finished
  event -> workflow transition loop
- status is useful enough to debug an idle workflow without opening SQLite

### 0.5 Provider Runner MVP

Start with a generic command-template runner, then add presets for Codex, Claude
Code, and Pi.

Provider config:

```json
{
  "agents": {
    "worker": {
      "provider": "command",
      "command": ["sh", "-c", "printf '%s\n' \"$ARMATURE_PROMPT\""],
      "cwd": ".",
      "timeoutSeconds": 1800
    }
  }
}
```

Runtime environment for providers:

```text
ARMATURE_WORKFLOW_ID
ARMATURE_INVOCATION_ID
ARMATURE_AGENT
ARMATURE_PROMPT
ARMATURE_INPUT_JSON
ARMATURE_RUN_DIR
```

Presets compile to the same internal command-runner contract. `command`
requires an explicit command array; `codex`, `claude-code`, and `pi` supply
default command templates and also accept command overrides plus extra args.
`timeoutSeconds` is enforced by the harness and records `provider_timed_out`
plus a typed `finished` event with `status: "timed_out"`. Desire-path logging
should record provider command errors so we can pave better presets from actual
use.

Exit criteria:

- command provider works in CI without external credentials
- Codex/Claude/Pi presets are config-driven wrappers over the same runner
- provider timeouts are enforced and visible in harness status
- `harness run --drive-workflow` can process completion events back through the
  workflow until idle
- stdout/stderr paths are visible in status and completion records

### 0.6 Completion Event Semantics

The harness writes a completion record and enqueues a typed workflow event in
one transaction.

Default completion event:

```json
{
  "id": "inv_...",
  "name": "worker",
  "status": "succeeded",
  "summary": "...",
  "exitCode": 0
}
```

Rules:

- `id` is the Armature invocation id
- `name` is the declared Armature agent name
- `status` is `succeeded`, `failed`, `cancelled`, or `timed_out`
- `summary` is a short harness/provider summary, not raw logs
- raw stdout/stderr remain artifacts
- the event payload must validate against the workflow's declared event schema
  before enqueue
- if validation fails, record a harness failure and do not enqueue a malformed
  workflow event

Exit criteria:

- malformed completion payloads fail visibly in harness status
- successful completion can retire active invocation accounting
- duplicate completion attempts are idempotent

### 0.7 Remove Or Quarantine `--agent-file`

Once native harness e2e passes:

- remove `--agent-file` from CLI help, README, skill docs, and operation docs
- delete JSON agent-file e2e tests or rewrite them against native ledger
- delete JSON agent adapter code if no longer needed
- if a file-backed fixture remains, rename it so it cannot be mistaken for the
  product interface

Exit criteria:

- the product path has one durable agent interface: SQLite ledger plus harness
- no current docs instruct users or agents to manage `agents.json`
- tests do not require the old bridge for normal agent orchestration

## Next Track: Harness Profile Policy

Goal: make native provider execution governable without putting provider
commands or sandbox trivia into ordinary workflow source.

Target shape:

```text
.armature source -> agent profile intent
harness profile policy -> concrete provider + authority posture
harness runner -> enforced or best-effort provider execution + audit event
```

Design commitments:

- `profile "name"` on an agent is semantic intent, not a provider name.
- Local default may remain permissive for fast experiments.
- Safer built-in mode separates `research`, `repo-reader`, `repo-writer`, and
  `human-review` profiles.
- Custom mode requires every profile to include an agent-facing description and
  explicit provider, timeout, environment, filesystem, network, tool, and
  enforcement settings.
- The harness must record requested profile, resolved profile, provider,
  requested authority, enforced authority, and unsupported best-effort gaps.
- Raw `command` providers are allowed by default only in permissive mode; in
  separated/custom mode they must be explicitly named by policy.
- The Armature skill should teach coding agents to read profile descriptions
  before assigning profiles.

Implementation status:

1. Parser/lowering support for `profile "name"` inside agent option blocks:
   implemented.
2. `profile: Option<String>` on WorkflowIR agents and tests: implemented.
3. Harness profile policy structs and validation: implemented for:
   - mode: `permissive | separated | custom`
   - default profile resolution
   - built-in separated profile set
   - profile names/descriptions
   - provider, command, args, cwd, timeoutSeconds
   - filesystem/network/env/tool/enforcement settings
4. `--profile-policy` on `validate`, `harness once`, `harness run`, and
   `harness status`: implemented.
5. Queued invocation profile resolution before provider launch: implemented.
6. Provider authority mapping:
   - `codex`: maps filesystem to `--sandbox` and allowed network to `--search`
   - `claude-code`/temporary `claude` alias: maps filesystem to `--permission-mode` and tools to
     `--allowedTools`
   - `command`: requires explicit approval and reports external-process
     authority
   - `pi`: reports best-effort until stable sandbox flags are mapped
7. Harness events/status JSON include profile resolution and enforcement
   evidence.
8. E2e/unit coverage:
   - permissive default still runs a simple command fixture
   - separated mode denies unknown profile
   - separated mode denies raw command unless explicitly approved
   - repo-writer profile launches with network-denied posture when provider
     supports it or records best-effort evidence when it does not
   - status shows profile resolution and enforcement gaps

Exit criteria:

- users can write `.armature` files with semantic agent profiles: done
- operators can govern provider launch authority without editing workflow
  source: done for harness launch policy, provider-dependent for deep sandboxing
- coding agents receive clear diagnostics when they choose the wrong profile:
  done for validation and launch-time profile resolution failures
- no docs recommend raw provider names as workflow intent: done

### 0.8 Desire-Path Apparatus

The native harness records lightweight local observation:

- record provider command, normalized args, exit code, stderr snippet, and
  failed workflow command guesses in `harness_events`
- classify obvious cases as:
  - `provider_command_failed`
  - `workflow_validation_failed`
  - `unknown_agent`
  - `completion_schema_mismatch`
  - `idle_without_work`
  - `lease_expired`
- expose recent observations through `harness status --json`

This is not a telemetry system. It is a local product-development apparatus for
watching agents use Armature and turning repeated mistakes into better language
and CLI ergonomics.

Exit criteria:

- a failed fake provider run leaves enough evidence to improve the UX
- status can distinguish workflow bugs from provider/harness bugs

## Next Track: Real Provider Adapters

Goal: turn `codingAgent` from a confusing pseudo-constructor into a clear
language-level role declaration backed by concrete provider adapters.

Target shape:

```text
.armature source declares agent role + profile intent
harness profile policy resolves profile to provider + authority
provider adapter builds and runs the concrete launch
SQLite harness ledger records evidence and completion
workflow processes typed finished events
```

Design commitments:

- `codingAgent` is abstract. It never means Codex, Claude Code, Pi, or shell by
  itself.
- workflow source should prefer `agent worker = codingAgent { ... }`; the
  existing `codingAgent()` syntax may remain as a compatibility alias during
  migration.
- provider names belong in harness policy, not ordinary workflow control flow.
- `command` provider is a deterministic fixture or externally sandboxed escape
  hatch, not proof that real coding agents are wired correctly.
- `--profile-policy` should be sufficient for governed provider launches once
  adapters are complete; `--config` should be a local override/test fixture.
- every launch must record requested authority, enforced authority, warnings,
  command evidence, stdout/stderr paths, and completion event ids.

### 1.1 Respec And Rename The Language Surface

Spec changes:

- [authoring-format.md](authoring-format.md): make `codingAgent` an abstract
  role declaration and document `codingAgent()` as compatibility syntax only.
- [grammar.md](grammar.md): allow constructor-like agent declarations without
  parentheses.
- [source-to-ir.md](source-to-ir.md): preserve agent kind, profile, maxActive,
  and source spans independent of provider binding.
- [provider-adapters.md](provider-adapters.md): keep the concrete provider
  contract and launch evidence shapes.
- [harness-profiles.md](harness-profiles.md): specify that profiles are semantic
  intent and provider bindings are deployment/runtime policy.

Implementation:

- update the parser to accept:

  ```armature
  agent worker = codingAgent {
    profile "repo-writer"
    maxActive 1
  }
  ```

- keep `codingAgent()` accepted for now
- update parser suggestions to prefer the no-parentheses form
- update examples, templates, skill docs, and dogfood fixtures to use the
  no-parentheses form

Exit criteria:

- old and new syntax both parse
- canonical diagnostics/examples use `codingAgent { ... }`
- docs no longer imply `codingAgent()` is a concrete runner or mock constructor

### 1.2 Make Harness Policy A Complete Provider Binding

Current harness execution still leans on `--config` for concrete command
details. The product path should be:

```sh
armature harness run workflow.armature \
  --store workflow.sqlite \
  --profile-policy .armature/harness-policy.json \
  --drive-workflow
```

Implementation:

- allow `harness once/run/status` to omit `--config` when `--profile-policy`
  supplies complete provider information or a provider adapter has a default
  launch template
- keep `--config` as an override for deterministic fixtures
- define config/profile merge rules:
  - profile policy supplies provider, authority, timeout, cwd, env, tools
  - provider adapter supplies default command template when profile does not
    override command
  - local config may override command/cwd/timeout only when policy permits that
    override mode
  - if config and policy both specify provider, they must match
- expose profile-policy-only execution in `armature init` scaffolding

Exit criteria:

- a Codex profile can run with no `harness.json`
- deterministic command fixtures still work with explicit `--config`
- profile/provider mismatch errors include the agent, profile, configured
  provider, expected provider, and fix

### 1.3 Extract Provider Adapter Trait

Refactor the current inline command-building logic into provider adapters.

Internal target:

```rust
trait ProviderAdapter {
    fn name(&self) -> &'static str;
    fn capabilities(&self) -> ProviderCapabilities;
    fn build_launch(&self, invocation: &AgentInvocation, profile: &ResolvedProfile)
        -> Result<LaunchPlan, AdapterError>;
    fn run(&self, launch: LaunchPlan) -> Result<ProviderRunResult, AdapterError>;
}
```

Concrete structs:

```text
CommandAdapter
CodexAdapter
ClaudeCodeAdapter
PiAdapter
```

Implementation notes:

- keep synchronous process execution for MVP
- keep timeout enforcement in the harness
- keep stdout/stderr capture in the common runner
- provider-specific code should only build launch plans and enforcement
  evidence
- all adapters produce the same completion payload shape

Exit criteria:

- unit tests cover launch plan generation for command, Codex, Claude Code, and
  Pi
- harness e2e still passes through the adapter registry
- provider warnings are stable and visible in `harness status --json`

### 1.4 Codex Adapter

Default launch:

```text
codex <authority flags> exec {{prompt}}
```

Implement:

- filesystem mapping:
  - `read_only` and `none` -> `--sandbox read-only`
  - `workspace_write` -> `--sandbox workspace-write`
  - `provider_default` -> no sandbox flag
- network mapping:
  - `allowed` or `provider_default` -> `--search`
  - `denied` -> omit `--search` and record limited-denial evidence
- env allowlist through common runner
- cwd and timeout through common runner
- custom command override evidence:
  - `providerCliFlagsApplied: false`
  - warning that custom command owns equivalent flags

Exit criteria:

- command plan tests for read-only research and workspace-write repo-writer
- opt-in dogfood fixture launches a real Codex run when `codex` is installed
- skipped gracefully with clear diagnostics when `codex` is unavailable

### 1.5 Claude Code Adapter

Provider name target: `claude-code`

Temporary compatibility: accept `claude` as an alias in policy/config.

Default launch:

```text
claude <authority flags> -p {{prompt}}
```

Implement:

- filesystem mapping:
  - `read_only` and `none` -> `--permission-mode plan`
  - `workspace_write` -> `--permission-mode acceptEdits`
  - `provider_default` -> no permission-mode flag
- allowedTools mapping -> `--allowedTools <comma-list>`
- network denied -> warning until a stable native flag is validated
- custom command override evidence as with Codex

Exit criteria:

- command plan tests for plan-mode and acceptEdits-mode
- opt-in dogfood fixture launches a real Claude Code run when `claude` is
  installed
- docs consistently say Claude Code, not just Claude, for the provider concept

### 1.6 Pi Adapter

Default launch:

```text
pi run {{prompt}}
```

Implement:

- provider adapter exists and produces launch evidence
- enforcement evidence is best-effort by default
- no claim of filesystem/network sandbox enforcement until stable Pi flags are
  documented and tested

Exit criteria:

- command plan test covers warning/evidence shape
- opt-in dogfood fixture can launch Pi when available
- docs are explicit that Pi is not enterprise-safe until enforcement support is
  validated

### 1.7 Real Provider Dogfood

Maintain dogfood fixtures in two categories:

```text
deterministic command-provider fixtures
real provider fixtures
```

Deterministic:

- Ralph Wiggum fixture remains command-provider and CI-friendly
- README labels it as deterministic and not a real coding-agent launch

Real provider:

- add `dogfood/codex-repo-writer`
- add `dogfood/claude-code-repo-writer`
- add `dogfood/pi-repo-writer` once Pi flags are validated enough for dogfood
- each fixture should:
  - use `codingAgent { profile "repo-writer" }`
  - run through `--profile-policy`
  - create or edit a tiny project artifact
  - record harness status and provider evidence
  - be opt-in, not normal CI

Exit criteria:

- at least one real provider dogfood run has been executed locally and its UX
  notes are captured
- deterministic fixture remains runnable without external credentials

### 1.8 Fix Sequential `maxActive` Ergonomics

Dogfooding exposed a sharp edge: if a `finished` transition immediately starts
the next `maxActive 1` invocation, active invocation retirement may not be
visible until after the completion event is processed, so the next `start` can
fail.

Candidate fixes:

1. Runtime semantic change: when processing a `finished` event, retire the
   matching invocation before evaluating `start` capacity checks in that same
   transition.
2. Language pattern: require users to raise an internal event before the next
   `start`.
3. Sugar: add a dedicated sequencing helper later, such as `after finished`
   semantics, if the pattern repeats.

Preferred next step:

- implement a runtime semantic test for the desired behavior before changing
  code
- decide whether the formal active-invocation model should match option 1
- if option 1 is coherent, implement it so ordinary sequential loops do not
  need boilerplate internal events

Exit criteria:

- Ralph-style sequential workflow can be written without the internal `next`
  event, or docs clearly justify why that event is required
- maxActive invariants still hold under replay and duplicate completion events

### 1.9 Documentation And Skill Updates

Update:

- `skills/armature-statechart/SKILL.md`
- `examples/templates/simple-agent-supervisor.armature`
- dogfood READMEs
- CLI help for `harness run`
- `armature init` scaffold comments

Messaging:

- `codingAgent` is an abstract role
- `profile` is semantic intent
- provider adapter is selected by harness policy
- command provider is deterministic/local, not a real coding agent unless the
  command itself launches one

Exit criteria:

- an agent reading only the skill file can correctly distinguish command
  fixtures from real provider adapters
- examples show both deterministic and real-provider paths
- desire-path records are local, explicit, and easy to delete with the store

## Phase 0: Remodeling Checkpoint

Goal: decide whether the formal models and IR abstraction need a small remodel
before changing runtime semantics for real `coerce`.

This phase should be quick and focused. It should not reopen browser
portability, generated TypeScript execution, HJSON, or arbitrary scripting.

Questions to answer:

- Does the current generated TLA+/Maude abstraction still match the selected
  language after the expression primitive boundary was narrowed?
- Should `coerce` appear in the model as an abstract synchronous value event,
  or is the current per-function nondeterministic output map sufficient?
- Does the model need to distinguish successful coerce reuse from fresh coerce
  execution, or can idempotency remain a runtime/storage invariant?
- Which expression primitives must be represented formally now, and which can
  stay as static/runtime validation obligations?
- Do we need a hand-written update to the existing spec implementation model
  before coding, or are generated-model notes plus runtime tests enough?

Recommended work:

1. Review `models/statechart-workflows/SpecImplementation.tla` and
   `SpecImplementation.maude` against the selected semantics.
2. Add comments or small state variables only if they clarify the selected
   `coerce` abstraction.
3. Keep BAML backend internals, provider behavior, prompts, raw responses, and network
   failures out of the formal model.
4. Model `coerce` as nondeterministic schema-valid output. Runtime handles
   idempotency and durable replay.
5. Explicitly classify every invariant in the implementation plan as one of:
   static validation, runtime enforcement, generated model, hand-written model,
   property test, adapter contract test, or future work.

Exit criteria:

- the plan states whether formal artifacts need immediate changes
- no runtime code proceeds under ambiguous `coerce` semantics
- any model changes still pass the existing formal check script
- if no model changes are needed, the rationale is recorded in this file or a
  short model README note

## Phase 1: Normalize Code Boundaries

Goal: align existing code structure with the selected architecture before
adding new behavior.

### 1.1 Workflow Crate

Work in `crates/armature-workflow`:

- keep `class`, `enum`, and `coerce` declarations as the source of truth
- keep generated BAML source as a derived artifact
- add schema variants or reserved-type diagnostics for media types only when
  the parser, validator, runtime, policy, and BAML executor all support
  the representation
- tighten static expression validation toward
  [expression-primitives.md](expression-primitives.md)
- reject undeclared calls, unsupported helpers, general-purpose operations,
  and optional-field access that is not guarded or pattern-proven
- keep source diagnostics in source vocabulary: say `data`, not internal
  `context`, unless discussing IR JSON

Exit criteria:

- unsupported expression primitives fail validation with source spans
- supported helpers have stable names and schemas
- BAML boundary types are validated before build/runtime

### 1.2 Engine Boundary

Work in `crates/armature-engine`:

- introduce `CoerceExecutor` as a runtime dependency, separate from
  `EffectDispatcher`
- move fake coerce behavior behind `FakeCoerceExecutor`
- remove direct interpreter dependence on `fake_coerce_outputs` as the
  production execution path
- keep deterministic fake outputs available for unit and e2e tests
- introduce DTOs for `CoerceRequest`, `CoerceOutcome`, `CoerceBackend`,
  `CoerceStatus`, and `CoerceErrorCategory`
- make coerce failures distinct from `UnsupportedExpression`

Proposed trait shape:

```rust
pub trait CoerceExecutor {
    fn coerce(&mut self, request: CoerceRequest) -> Result<CoerceOutcome, CoerceError>;
}
```

The engine owns argument evaluation, schema validation, idempotency lookup, and
storage. The executor owns only the backend call.

Exit criteria:

- fake and real coerce can share one request/outcome boundary
- interpreter code can evaluate `coerce` without knowing whether the backend is
  fake, generated stdio, external HTTP, or brokered
- failures carry categories useful for status and retry policy

### 1.3 Adapter Crate

Work in `crates/armature-adapters`:

- remove or demote the placeholder `BamlAdapter` trait if it conflicts with
  the engine-facing `CoerceExecutor`: implemented by removing the placeholder
  trait; real `coerce` uses the engine `CoerceExecutor`
- keep adapter manifests for external effects such as `askHuman`, capability
  operations, and explicitly adapter-backed `start`/`send`
- do not model `coerce` as a normal adapter-manifest effect in v1
- if dependency direction becomes awkward, move shared DTOs into a small
  contracts module or crate instead of letting adapters depend on interpreter
  internals

Exit criteria:

- adapter-backed effects and executor-backed coerce have separate, clear
  boundaries
- no adapter manifest is required just to call BAML-backed `coerce`

## Phase 2: Durable Coerce Storage

Goal: make `coerce` replay-safe and inspectable before calling any real model.

Work in `crates/armature-engine` storage:

- add a `coerce_calls` table matching [storage.md](storage.md)
- add migration/version handling for the new table
- add indexes for latest calls per workflow/function and successful
  idempotency-key lookup
- add store methods:
  - `find_successful_coerce_call(workflow_id, idempotency_key)`
  - `append_coerce_call_attempt(record)`
  - `latest_coerce_calls(workflow_id, limit)`
  - `latest_coerce_failures(workflow_id, limit)`
- include coerce records in log/status projections without mixing them into
  asynchronous effect dispatch logs

Idempotency key shape:

```text
workflow_id/workflow_version/event_id/transition_attempt/step_path/function_name
```

Current runtime step locators use the interpreter path through handler, entry,
always, guard, and invariant evaluation. A future source-map layer may replace
these with source-span-backed paths, but replay safety does not depend on that.

Exit criteria:

- successful coerce outputs can be reused by idempotency key
- failed coerce attempts are append-only and visible
- schema migrations preserve existing stores or fail closed on unsupported
  versions
- tests cover insert, lookup, duplicate success behavior, latest calls, and
  latest failures

## Phase 3: BAML Execution Backends

Goal: implement real `coerce` through a sandbox-friendly generated stdio
backend by default, while keeping external BAML HTTP and brokered BAML as
explicit modes.

Work in engine and CLI:

- keep `BamlHttpCoerceExecutor` for explicit `--baml-url`
- add `BamlGeneratedStdioCoerceExecutor` for default managed execution
- keep CLI override:

```text
--baml-url http://127.0.0.1:2024
```

- when no fake coerce output and no `--baml-url` is supplied, workflows with
  `coerce` should use generated stdio by default
- external HTTP mode calls:

```text
POST /call/<function_name>
```

with named JSON arguments derived from the `coerce` declaration parameter names
- generated stdio mode sends the same named JSON arguments through a framed
  stdin/stdout protocol to a generated BAML client runner
- support request timeouts
- support generated runner startup timeout and protocol validation
- classify backend errors:
  - `baml_cli_not_found`
  - `baml_generation_failed`
  - `baml_runner_start_failed`
  - `baml_runner_protocol_error`
  - `baml_broker_unavailable`
  - `baml_http_unavailable`
  - `baml_http_error`
  - `baml_timeout`
  - `baml_parse_failure`
  - `baml_schema_validation_failure`
  - `baml_policy_denied`
  - `internal_error`
- persist raw response metadata according to policy; allow redaction of raw
  provider output while retaining parsed output and failure details

Implementation sequence:

Implemented so far:

1. Add HTTP client dependency in the smallest crate that needs it.
2. Add executor construction in CLI when `--baml-url` is provided.
3. Compute and record generated BAML artifact hash.
4. Validate arguments before calling HTTP.
5. Validate parsed output after HTTP.
6. Reuse successful output before HTTP.
7. Append failure records on all backend and schema errors.

Backend migration sequence:

1. Add lazy mode resolution at `coerce` evaluation time: fake outputs,
   external URL, brokered mode, no coerce call, generated stdio default.
2. Keep existing `managed_http` code only as an explicit debug/compatibility
   backend while generated stdio is implemented.
3. Add policy diagnostics for generated stdio, external HTTP, and brokered
   modes.
4. Add `GeneratedBamlRunner` helper:
   - write generated `baml_src`: implemented
   - write TypeScript `generators.baml`: implemented
   - run `baml-cli generate`: implemented for the managed runner path
   - generate or locate a tiny runner entrypoint: implemented
   - send one named JSON request over stdin/stdout: implemented
   - capture stdout/stderr artifacts: failure details are captured in errors;
     dedicated artifact files remain future hardening
   - validate protocol responses: implemented
5. Use generated stdio in `run` and `harness run --drive-workflow` when real
   coerce is needed and no explicit backend override is supplied.
6. Record runtime metadata in coerce backend records and status projection.
7. Add brokered storage/protocol after generated stdio is working.

Testing:

- unit test request construction and output validation
- unit test generated runner protocol success/failure without TCP listeners
- deterministic e2e should continue using `FakeCoerceExecutor`
- add generated-runner tests with a fake runner executable
- keep local HTTP tests limited to explicit HTTP mode
- keep opt-in real BAML integration test gated by:

```text
ARMATURE_RUN_BAML_E2E=1
BAML CLI or external URL
provider credentials or compatible local provider
```

Exit criteria:

- Armature-managed BAML can satisfy a real workflow `coerce` without requiring
  a separately started sidecar or a local listening socket
- an external BAML server can satisfy a real workflow `coerce` when `--baml-url`
  is supplied
- replay of the same committed transition does not call BAML again
- coerce failure is visible through failed events, durable diagnostics, and
  current coerce failure status while the event is unresolved; latest coerce
  failures remain historical and v0 does not create a hidden blocked state
- fake e2e tests still pass without network/provider credentials

## Phase 4: Status, Overview, And Debuggability

Goal: make real and fake workflows legible when they wait, fail, or choose a
branch.

Work in engine and CLI:

- include workflow data summary in `status` and `overview`
- include latest coerce calls, current coerce failure, and historical failures
- show current state, active invocations, pending events, latest transition,
  latest effects, current effect failures, current blockers, historical recent
  failures, policy blockers, and latest coerce decisions together
- avoid printing raw BAML responses when policy redacts them; general workflow
  data redaction is a later policy slice
- avoid adapter calls during status projection; status reads durable records
  only

Suggested status JSON additions:

```json
{
  "data_summary": {},
  "latest_coerce_calls": [],
  "latest_coerce_failures": [],
  "policy_blockers": []
}
```

Exit criteria:

- `armature status --json` exposes coerce and data summary fields
- `armature overview` renders those fields compactly for humans
- policy-denied effect dispatches are projected as first-class policy blockers
- no status command performs hidden live adapter or BAML calls
- e2e tests assert status includes current state, queued events, active
  invocations, data summaries, and coerce decisions/failures where applicable

## Phase 5: Expression Kernel Completion

Goal: finish the useful 99% orchestration primitives without drifting into a
general-purpose language.

Implement and validate the v1 primitive set:

- literals: string, block string, int, float, duration, bool, nil, object, list
- paths and field access over `data`, event bindings, and `let` bindings
- equality and ordering over compatible scalar schemas
- boolean logic
- membership
- case patterns over enum/literal/glob/wildcard
- object/list construction
- path-only string interpolation
- list helpers:
  - `list.length`
  - `list.isEmpty`
  - `list.contains`
  - `list.append`
  - `list.remove`
  - `list.first`
  - receiver sugar for `xs.append(value)` and `xs.remove(value)`
- map helpers:
  - `map.get`
  - `map.set`
  - `map.remove`
  - `map.containsKey`
- text helpers:
  - `text.trim`
  - `text.contains`
  - `text.startsWith`
  - `text.endsWith`
  - `text.matchesGlob`
- time helpers:
  - `now`
  - `elapsedSince`
  - `time.elapsedSince`
- typed value calls:
  - `coerce`
  - direct calls to coerce declarations
  - capability value calls

Reject:

- loops
- recursion
- user-defined functions
- lambdas
- map/filter/reduce
- sorting
- general arithmetic libraries
- regex
- arbitrary string processing
- inline multimodal manipulation
- implicit type coercions
- imports or host callbacks

Exit criteria:

- every supported primitive has validation tests and runtime tests
- every rejected construct has a targeted diagnostic
- examples use only supported primitives
- formal model generation either models the primitive or explicitly fails with
  an actionable unsupported-construct diagnostic

## Phase 6: Formal Model Updates

Goal: keep verification useful as runtime semantics become more complete.

Work in `crates/armature-modelgen` and `models/statechart-workflows`:

- keep BAML backend internals out of generated and hand-written models
- model coerce as nondeterministic schema-valid output
- model bounded outputs for enums, literals, bools, nulls, unions, and record
  discriminants
- add workflow data abstraction only for fields needed by invariants
- decide whether each expression primitive is:
  - modeled directly
  - statically validated and elided
  - runtime-enforced only
  - unsupported for model generation
- preserve fail-closed behavior for expression invariants that cannot be
  represented
- keep Maude as a possible reference-semantics pressure tool for handler lookup,
  raised events, and effect commit ordering
- keep Veil as a later proof-oriented target after semantics stabilize

Exit criteria:

- generated TLA+/Maude artifacts still pass existing formal checks
- modelgen diagnostics clearly explain unsupported expression/invariant cases
- generated models agree with the hand-written model on control-state,
  active-invocation, and coerce-output abstractions

## Phase 7: Real Adapter Slices

Goal: connect the runtime to useful external systems without weakening the
language boundary.

Implement in this order:

1. **Scoped plan/state adapter**
   - read plan snapshot: implemented for JSON plan files through
     `run --plan-file`
   - count unfinished work: implemented for JSON plan files through
     `plan.unfinishedItems()`
   - read next ready item: implemented for JSON plan files through
     `plan.nextReadyItem()`
   - mark ready for quality: implemented for JSON plan files
   - mark done: implemented for JSON plan files
   - mark blocked: implemented for JSON plan files
   - use lock or compare-and-write for file-backed state: implemented for JSON
     plan writes with a short-lived lock file plus atomic temp-file replacement
   - expose schemas and required capabilities through manifests: implemented
     through adapter manifests, with built-in JSON manifest injection for
     `run --plan-file` when no explicit `plan.*` manifest is loaded

2. **Human review adapter or event bridge**
   - create visible review obligations: implemented for JSON review files
     through `run --review-file`
   - expose schemas and required capabilities through manifests: implemented
     through adapter manifests, with built-in JSON manifest injection when no
     explicit `askHuman` manifest is loaded
   - accept typed human-response events: implemented for
     `emit --review-file` through the built-in `humanReview.responded` schema
   - update local review records from typed responses: implemented for
     `emit --review-file`
   - keep idempotency keys stable: implemented for JSON review obligation
     records

3. **Native agent ledger and harness**
  - start declared local agent work: implemented through native
    `agent_invocations`
  - send messages to declared local targets: implemented through native
    `agent_messages`
  - claim queued work: implemented through harness leases in SQLite
  - run providers: implemented through command-provider MVP plus Codex, Claude
    Code, and Pi preset command construction; provider adapters are not yet
    extracted as first-class structs
  - observe typed completion events: implemented through harness completion
    records and direct `workflow_events` enqueue
  - record stdout/stderr artifacts: implemented through durable run directories
    referenced from invocation rows
  - enforce target compatibility: implemented through declared
    `thread`/`codingAgent`/adapter targets, static rejection for starting
    thread-only agents, and launch-time profile/provider policy checks

4. **Legacy compatibility adapter**
   - not a product objective
   - keep only if a narrow fixture/debug use case is documented
   - must not shape WorkflowIR or source semantics
   - must not reintroduce arbitrary script authority as normal workflow logic

Exit criteria:

- a real workflow can start agent work, receive completion, update plan state,
  and ask for human review
- all adapter effects are manifest-described, policy-checked, idempotent where
  required, and logged durably
- adapter failures produce visible workflow state

## Phase 8: CLI, Build, And Policy Completion

Goal: make the product commands coherent for local and enterprise usage.

CLI work:

- ensure all commands that validate or execute workflows accept
  `--adapter-manifest` and `--policy` consistently: implemented for
  validate, run, status, overview, build, check, prove, emit-model, and
  emit-config; events and log accept them as validation-only inspection
  context; emit accepts adapter manifests for typed event intake and policy
  documents for policy-document validation
- ensure file-backed adapter convenience flags are accepted by runtime and
  inspection commands: implemented for `run`, `emit` where event intake needs
  typed adapter events, and for `validate`, `build`, `check`, `prove`,
  `emit-model`, `emit-config`, `status`, `overview`, `events`, and `log` as
  validation-only or build-metadata context
- keep `events --limit` and `log --limit` bounded: implemented with a 10,000
  record cap before querying SQLite
- support event inspection by queue state: implemented for `events --status`
  using durable status-filtered storage queries
- support administrative retry of failed queue records: implemented for
  `retry-event --event-id`, limited to `failed` and `dead_lettered` events,
  preserving attempt counts and clearing `last_error`
- make generated stdio the default real `coerce` runtime when a workflow
  contains `coerce`, no fake coerce output is supplied, and no explicit backend
  override is supplied
- keep `--baml-url` as an explicit external endpoint override
- keep `--fake-coerce-output` as testing/development-only
  - duplicate fake output names are rejected to avoid silent test fixture
    overrides
  - fake output names containing whitespace or control characters are rejected
- keep `build` producing:
  - `workflow-ir.json`: implemented
  - `baml_src/workflow.baml`: implemented
  - generated model files: implemented for TLA+, TLA config, and Maude
  - adapter manifest bundle, when supplied: implemented
  - policy document bundle, when supplied: implemented
  - artifact hashes: implemented through `artifact-hashes.json` and
    `build --json` hash output

Policy work:

- keep the initial exact capability policy shape stable
- BAML-specific policy knobs:
  - `allow_baml_network`: implemented for `run --baml-url`
  - `allowed_baml_urls`: implemented as exact URL allowlist for `run --baml-url`
  - `allow_baml_stdio_runner`: target policy field for generated stdio
  - `allow_baml_codex_oauth`: implemented for explicit Codex/ChatGPT OAuth
    credential authority in generated stdio mode
  - `allow_baml_http`: target policy field for explicit HTTP mode
  - `allow_baml_broker`: target policy field for brokered mode
  - `allow_managed_baml_server`: transitional field for current managed HTTP
    implementation; replace during generated stdio migration
  - `allowed_models`: schema field validated and reserved until Armature owns
    provider/model selection
  - `allowed_env_vars`: used to project the managed BAML process environment
    when supplied; model-specific enforcement remains future work
- make raw response redaction policy explicit: implemented through
  `store_baml_raw_responses`; enterprise redacts by default, explicit false
  always redacts, and parsed output remains durable for replay/status
- keep `baml.coerce` as the capability name for structured model output

Exit criteria:

- command behavior is consistent across validate/build/run/check/status
- policy document validation failures are reported as policy failures, not as
  adapter manifest failures
- local mode stays easy
- enterprise mode can deny unknown capabilities and disallowed BAML URLs
- build artifacts are sufficient to reproduce validation/model assumptions

## Phase 9: End-To-End Testing

Goal: prove the whole product path works, not only unit slices.

Required e2e layers:

1. **Deterministic fake e2e**
   - no network
   - no provider keys
   - fake coerce executor
   - fake adapter manifest
   - spec implementation workflow reaches expected states
   - duplicate events are ignored or handled correctly
   - status/overview explain current state

2. **Recovery e2e**
   - simulate processing-event crash
   - confirm startup requeues safely
   - confirm attempt counts are visible
   - confirm no duplicate active invocation projection

3. **Formal command e2e**
   - run `check` with TLA/Maude when tools are installed
   - skip clearly when tools are absent

4. **Real BAML e2e**
   - opt-in only
   - generated `baml_src`
   - generated stdio runner by default
   - external `--baml-url` only for explicit HTTP coverage
   - provider/local-model credentials supplied by environment
   - assert durable `coerce_calls` records and status projection

5. **Harness and adapter e2e**
   - native harness e2e should start work, claim an invocation, run a fake
     provider, process a typed `finished` event, update plan state, and send a
     completion message
   - file-backed plan/review adapter e2e remains useful for plan state and
     human review
   - real external adapter e2e remains future work once an explicitly external
     adapter exists

CI expectations:

```sh
cargo fmt --all --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo build -p armature-cli
scripts/check-docs.sh
scripts/check-e2e.sh
scripts/check-formal-models.sh
```

`scripts/check-docs.sh` is intentionally more than a smoke test. It validates
and builds the documented supervisor template, runs it through the native
harness with the generated harness policy, checks that `overview` reports the
active worker, checks that `overview` reports summarized workflow data in both
active and settled states, processes the queued completion event, and checks
that `overview` reports an idle settled workflow. It also validates the
generated `.armature/harness-policy.json` and exercises the local human-review
bridge by creating an `askHuman` obligation, emitting a typed
`humanReview.responded` event, and checking that the review JSON file records
the response.

Opt-in real BAML tests should not make normal CI require provider keys.

Exit criteria:

- fake e2e covers the normal workflow loop
- recovery e2e covers durable queue behavior
- formal checks are wired into CI or clearly skippable
- `prove` exercises all currently supported generated verification backends
- real BAML e2e is documented and runnable by a developer with credentials
- CLI regression coverage asserts durable workflow data summaries in both
  `status` and `overview`
- documentation smoke coverage asserts summarized workflow data for the
  supervisor template lifecycle

## Phase 10: Product Hardening

Goal: make the system practical for nontechnical and enterprise users.

Deliverables:

- workflow templates: started with
  `examples/templates/simple-agent-supervisor.armature`
  - the documented local lifecycle is covered by `scripts/check-docs.sh`
- companion skill updates: current `skills/armature-statechart` documents
  file-backed adapter shortcuts, typed response/completion event intake, and
  debugging flow
- example workflows
- enterprise capability policy examples: expanded under `examples/policies/`
- documentation for common stuck states: added
  [operations.md](operations.md)
- diagnostics written for coding agents and operators: capability policy
  diagnostics include conservative `Fix:` hints naming exact policy fields
- migration notes from legacy Armature: added [migration.md](migration.md)
- schema/database migration story: added
  [database-migrations.md](database-migrations.md)
- release checklist: added [release-checklist.md](release-checklist.md)

Exit criteria:

- a nontechnical user can inspect why a workflow is waiting: text `overview`
  includes a derived `waiting:` line from durable status
- the documented template path demonstrates both "active worker" and "settled
  idle" status without custom scripts
- a coding agent can repair a workflow from diagnostics without reading runtime
  internals
- capability violations are explained in terms of contracts and targets
- `spec-implementation.armature` works end to end against real adapters in a
  real repo
- the old TypeScript/script-runner mental model is clearly documented as legacy

## Completion Definition

The v1 track is complete when:

- `.armature` source is the primary product surface
- the implemented expression kernel matches
  [expression-primitives.md](expression-primitives.md)
- real `coerce` uses the selected BAML backend with durable replay-safe records
- SQLite state, queue, transitions, effects, and coerce calls are durable and
  recoverable
- status and overview explain current state, workflow data summary, pending
  events, active invocations, latest effects, latest coerce calls, failures, and
  policy blockers
- generated formal models cover the implemented control-state and
  active-invocation semantics and fail closed for unsupported data invariants
- fake e2e, recovery e2e, formal command e2e, and opt-in real BAML e2e
  exist
- at least one real adapter path can start agent work and process completion
  events
- enterprise policy can deny unknown capabilities and disallowed BAML execution
  surfaces
