# Control Plane

Status: draft

WhippleScript runs many workflow instances concurrently. A `.whip` source file is
not itself a process. It compiles into a versioned program, and each execution is
a durable instance managed by the local or hosted control plane.

## Core Objects

```text
Program              compiled source plus version hash
Instance             one durable running copy of a program
Invocation           durable parent-to-child workflow request
WorkflowRevision     append-only activation record for changing one running instance to a new program version
RevisionEpoch        monotonic active-program epoch for one instance
Event                append-only observation for an instance
Fact                 current materialized truth for an instance
Effect               durable request for external work
EffectEdge           durable dependency between two effects
Run                  provider execution attempt for an effect
CancellationRequest  durable request to stop old-version work that may have crossed a provider boundary
Capability           registered external authority or effect surface
Profile              policy bundle for agent/tool authority
Runtime              daemon/control-plane process
Artifact             durable file/log/output associated with a run
Evidence             causal record linking events, rules, effects, runs, and artifacts
Skill                deterministic context bundle for an agent or turn
Plugin               package-provided effect/fact-schema/resource extension
InboxItem            pending human review request
```

Every object that belongs to a running workflow is namespaced by:

```text
instance_id
```

The same program version may have many concurrent instances.

## Program Lifecycle

```text
source -> parse -> typecheck -> analyze -> verify/check -> compile -> deploy
```

A compiled program records:

- source path or source bundle identity
- source hash
- compiler version
- generated IR hash
- included source bundle members
- pattern applications and their generated declaration provenance
- workflow input/output/failure contract
- imported/invokable workflow contracts
- declared capabilities
- declared profiles
- declared agent skills
- declared fact/event schemas
- analysis results
- optional generated verification artifacts
- optional generated BAML artifacts

Deploying a program does not run it. Starting creates an instance.

## Source Bundle And Root Selection

The compiler treats a source bundle as an include closure plus one selected root
workflow. A file may contain reusable top-level declarations such as schemas,
coerces, patterns, and invokable workflow declarations. A bundle is deployable
only when exactly one root workflow is selected. If more than one workflow
declaration is visible, the deploy/start command must name the root workflow.

Top-level `pattern` declarations are compile-time building blocks. `apply`
elaborates them into ordinary rules/schemas/effects before runtime. The compiled
program records source provenance for generated declarations so traces can point
back to both the application site and the pattern definition.

Top-level `workflow` declarations are runtime contracts. A root workflow starts
an instance directly. An imported workflow can be invoked by another workflow,
which creates a child instance or equivalent durable invocation record.

## Instance Lifecycle

An instance has a durable state:

```text
created
running
paused
blocked
completed
failed
cancelled
```

The control plane may process multiple instances concurrently, but each
instance's rule commits are serialized. External effects may run concurrently
according to policy and capacity.

Pausing an instance means:

- no new effectful rule rewrites are committed
- already claimed provider runs may continue unless cancellation is requested
- incoming events are still recorded
- status explains the pause boundary

Stopping or cancelling an instance appends an event and transitions the
instance into a terminal control-plane state. It does not delete the log.

`complete` and `fail` are workflow terminal actions. They append terminal
workflow events, validate the declared output/failure payload, and transition
the instance to `completed` or `failed`. A workflow can record intermediate
facts forever; it becomes terminal only through `complete`, `fail`, cancellation,
or control-plane policy.

For child invocations, a terminal child event resolves the parent's invocation
effect/result with the declared output or failure payload. Parent workflows do
not inspect child-local facts or rules except through declared contracts,
evidence, events, and artifacts.

## Workflow Revision

Workflow revision is a control-plane operation for adapting a non-terminal
running instance to a new compiled program version. It is distinct from source
language composition:

```text
apply   = compile-time pattern expansion
invoke  = runtime child workflow invocation
revise  = control-plane activation of a new program version for one instance
```

Ordinary workflow rules cannot activate a revision. They may produce patch
proposal artifacts, ask for human approval, call validation plugins, coerce a
typed decision, tell an agent to prepare a change, or invoke a child workflow
that produces a proposed source bundle. The active program changes only through
`whip revise` or an authorized control-plane API.

Revision activation is append-only and inspectable. A successful activation:

- validates a candidate source bundle and selected root workflow
- creates a new `WorkflowRevision` with the next `RevisionEpoch`
- links the previous active program version to the new program version
- appends a revision activation event with diagnostics and evidence
- makes future rule stepping use the new active revision epoch
- preserves attribution for all existing events, facts, effects, runs,
  invocations, evidence, and diagnostics

Revision is allowed only for non-terminal instances. Completed, failed, and
cancelled instances cannot be revised; a future retry/reopen feature must create
a new instance or another explicit control-plane object.

Compatibility checks are part of the revision dry-run and activation path:

- the selected root workflow name must match the active instance root unless a
  future explicit retarget operation is designed
- the candidate input contract must accept the already-started instance input
- output and failure contracts must remain compatible for parent invocations
- active facts read by candidate rules or schemas must typecheck, or activation
  must fail with a clear diagnostic
- old effects keep their resolved targets, provider bindings, profiles,
  capabilities, source spans, and version attribution even if the candidate
  program removes the declaration that originally produced them

Revision activation accepts an explicit cancellation policy for old-version
effects:

```text
keep             do not cancel old-version effects
cancel queued    terminal-cancel queued, blocked, and claimable old-version effects
request running  cancel queued old-version effects and request cancellation for claimed/running effects
```

Queued/blocked/claimable effects can be terminal-cancelled by the control plane
because no provider work is in flight. Claimed or running effects receive a
durable `CancellationRequest`; they become terminal only after provider/harness
confirmation, timeout, or recovery. Revision must not fabricate a terminal
provider result for work that already crossed the provider boundary.

The following behaviors are vNext follow-ups, not part of ordinary v0
revision:

- changing the active root workflow of an instance
- migrating active facts across schema-breaking revisions
- using provider-specific native cancellation depth beyond durable request
  recording
- applying policies more destructive than `keep`, `queued`, or `running`

Those behaviors require explicit control-plane surfaces, dry-run reports,
formal-model coverage, evidence, and dedicated confirmation flags as tracked in
[workflow-revision-followups-tracker.md](workflow-revision-followups-tracker.md).
They must not be introduced as silent extensions of `whip revise --root` or a
generic force flag.

## CLI Shape

Target local CLI:

```sh
whip check workflow.whip
whip deploy workflow.whip --name spec-impl
whip start spec-impl --input input.json
whip dev workflow.whip --input input.json

whip ps
whip status <instance>
whip facts <instance>
whip log <instance>
whip effects <instance>
whip runs <instance>

whip pause <instance>
whip resume <instance>
whip stop <instance>
whip emit <instance> event.type --json payload.json
whip revise <instance> workflow.whip --root Workflow --dry-run
whip revise <instance> workflow.whip --root Workflow --cancel keep
whip revise <instance> workflow.whip --root Workflow --cancel queued
whip revise <instance> workflow.whip --root Workflow --cancel running

whip plugins
whip skills
whip inbox
whip trace <instance>
whip evidence <run-or-effect>
```

`whip status --json <instance>` includes workflow invocation links when the
instance is either a parent or child in a durable invocation. Parent instances
expose `workflow_invocations.children[]`; child instances expose
`workflow_invocations.parent`. Each link includes the parent instance/effect,
child instance, target workflow, invocation input, source span when available,
and creation time.

`whip dev` is a convenience command for local validation. It should compile,
start one instance, run local effect workers, and stream useful status.

`whip run` may create and start an instance without driving it. The control
plane still needs an explicit driving surface for local validation:

```sh
whip step <instance> --program workflow.whip
whip worker --provider codex --once
whip dev workflow.whip --input input.json --provider codex --until idle
```

`whip step` should evaluate ready rules from the compiled IR, commit their fact
and effect rewrites transactionally, and stop before running external effects.
`whip worker` should claim already-materialized effects and execute configured
providers. `whip dev` may compose those loops for an operator-facing
single-command experience.

For production stepping, the compiled IR comes from the instance's active
revision epoch. A development override may exist for local experiments, but it
must be named explicitly and must not silently revise a running instance.

## Driver Semantics

A fully functional local runtime needs three separate loops. Keeping them
separate makes testing and recovery tractable.

```text
starter: create instance and append external input events
stepper: evaluate ready rules and commit fact/effect rewrites
worker: claim effects and run providers
```

`whip step` is deterministic. Given a program version, instance id, and store
state, it should:

1. Load the compiled IR for the instance's program version.
2. Rebuild or read the current fact projection.
3. Derive standard facts from new external events, such as `started`.
4. Evaluate ready rules in a deterministic order.
5. Lower each selected rule body into:
   - `NewFact` records for `record ...` blocks
   - consumed fact ids for `consume binding` / `done binding`
   - `NewEffect` records for `tell`, `coerce`, `claim`, `askHuman`, `call`,
     and `emit`
   - `NewEffectDependency` records for `after` blocks
   - evidence/diagnostic records for policy decisions and lowering details
6. Commit each rule atomically through the kernel.
7. Stop when no additional rules are ready, the configured step limit is hit,
   or the instance becomes paused/cancelled/terminal.

`whip step` must never execute providers. It only creates durable facts and
outbox effects.

Cron and heartbeat jobs are control-plane observations, not hidden source-level
loops. A local daemon, hosted scheduler, or test fixture may append a durable
timer/tick event, and workflow rules should turn that observation into ordinary
facts or effects. This keeps recurring work replayable and inspectable while
leaving scheduling policy outside the rule evaluator.

Rule readiness includes pure guard evaluation. For a rule with `when Class as
binding where <expr>`, the stepper must bind candidate facts first, evaluate the
typed expression against those bindings, and discard candidates whose guard is
false before lowering the rule body. Guard evaluation is part of deterministic
stepping and must not consult providers, BAML, plugins, the filesystem, the
network, wall-clock time, or random sources.

`when` is the only source-level introducer for rule readiness. `with` is not an
alias for readiness; it is reserved for effect/action configuration such as
`claim issue with loft`. The lowering pass must keep this distinction so a rule
cannot make a world-touching provider choice look like a fact observation.
Grouped readiness blocks, written as `when { ... }`, are parse-time sugar for
ordinary `when` clauses. Each non-empty line in the block must become one
readiness clause in IR before rule readiness and guard evaluation run.

`whip worker` is nondeterministic at the provider boundary but durable at the
kernel boundary. It should:

1. Query claimable effects filtered by provider, profile, capability, and
   optional instance id.
2. Claim one or more effects with leases.
3. Create run records.
4. Resolve provider binding, credentials, native enforcement, and workspace
   policy.
5. Invoke the configured provider adapter.
6. Store artifacts/evidence, including failure transcripts where available.
7. Append terminal completion/failure/timeout/cancel events.
8. Derive standard completion facts.

Every worker boundary has to be durable:

```text
provider binding resolution
credential lookup
workspace preparation
adapter/session launch
request submission
provider stream/read
artifact capture
terminal event append
fact derivation
```

Failures before claim should leave a blocked effect with explainable status and
diagnostics. Failures after claim should append a durable event, update the run,
and link evidence/artifacts before the worker reports completion. If the store
cannot append a terminal event, the worker must leave the lease/run recoverable
instead of reporting success out of band.

Provider and harness failures use these event types:

```text
provider.startup_failed
provider.auth_failed
provider.tool_failed
provider.transport_failed
provider.timed_out
effect.failed
effect.timed_out
effect.cancelled
```

Provider failure event payloads must include:

```text
effect_id
run_id?
provider
stage                 # binding | auth | workspace | startup | submit | stream | tool | transport | timeout | artifact | terminal_append
error_code
message
retryable
attempt
max_attempts
next_retry_at?
idempotency_key
correlation_id
diagnostic_ids
evidence_ids
artifact_ids
source_span?
```

`provider.startup_failed` covers adapter/session launch and harness bootstrap
failures. `provider.auth_failed` covers missing, expired, denied, or
mis-scoped credentials. `provider.tool_failed` covers provider-reported tool or
command failures after a request was accepted. `provider.transport_failed`
covers network, IPC, protocol, broken stream, and malformed provider response
failures. `provider.timed_out` covers queue, startup, request, stream, tool, and
overall run deadlines; its payload must name the timeout boundary and elapsed
duration.

Retry decisions are policy decisions, not hidden worker behavior. A retryable
failure keeps or returns the effect to a retry-pending/queued state with the
same effect id and idempotency key, creates a new run for each attempt, and
links every attempt through shared correlation/evidence. A non-retryable or
exhausted failure appends a terminal event exactly once. Replaying recovery must
be idempotent: the same terminal event idempotency key cannot create duplicate
terminal events, duplicate completion facts, or duplicate external side-effect
acknowledgements.

`whip dev` composes `step` and `worker` for one local validation session. It should
stream status, stop at idle/blocked/terminal states, and make every provider
decision visible in the store.

## Rule Lowering Requirements

The control plane needs a production lowering pass from typed IR and source spans
to concrete store writes. This pass must not rely on ad hoc prompt parsing.

For each rule body construct:

```text
record Class { ... }        -> typed fact projection
record Class from binding   -> typed fact projection with field-copy sugar
consume binding             -> consume the matched fact from current projection
done binding                -> alias for consume binding
done binding -> record ...  -> consume plus result record in one commit
tell agent ...              -> agent.tell effect with target/profile/skills
coerce function(...)        -> baml.coerce effect with function and arguments
claim issue with loft       -> loft.claim effect
askHuman ...                -> human.ask effect
call plugin.capability ...  -> capability.call effect
emit event                  -> event.emit effect
invoke Workflow             -> workflow.invoke effect or child invocation record
after effect succeeds       -> dependency edge
after effect succeeds as x  -> dependency edge with terminal-output alias
then effect/done            -> success-chain sugar over after blocks
complete output             -> workflow.completed terminal event
fail failure                -> workflow.failed terminal event
matrix rows                 -> typed fact records
action/template expansion   -> ordinary facts/effects before commit
assert expression           -> read-only assertion result
```

Lowering must resolve interpolation values from the matched facts/effect outputs
that are in scope. If a value cannot be resolved, the rule commit fails before
persisting any partial records.

Every lowered fact/effect must include:

```text
instance id
program version id
rule name
trigger event id or consumed fact keys
source span
normalized input JSON
stable idempotency key
correlation id
```

Consumed facts are part of the same atomic rule commit as produced facts,
effects, dependencies, evidence, and diagnostics. The store records consumed
fact ids in the `rule.committed` event payload and marks those active projection
rows consumed. Rebuilding projections from the event log must replay the fact
insertion events and then apply the recorded consumption transitions in sequence.

Dynamic agent targets, when supported, must lower to a concrete declared agent
before the `agent.tell` effect is created. The lowered effect stores both the
resolved agent and the source expression/provenance used to resolve it. A
missing, ambiguous, or unauthorized dynamic target blocks or fails before any
provider run starts.

Workflow assertions are deterministic checks over projections. They may run as
part of `whip check`, `whip step --assert`, `whip dev`, and e2e scripts. Failed
assertions should be emitted as diagnostics/evidence tied to the assertion's
source span; they must not mutate user facts or enqueue effects.

Assertion evaluation is a durable observation surface. Each evaluated assertion
must append one of:

```text
assertion.passed
assertion.failed
assertion.errored
```

Assertion event payloads must include:

```text
assertion_id
assertion_text
result                # pass | fail | error
program_version_id
rule_name?
source_span
read_set              # fact/effect/event ids or projection descriptors
actual_json?
expected_json?
error_code?
message?
diagnostic_ids
evidence_ids
correlation_id
idempotency_key
```

`assertion.failed` means the expression evaluated deterministically to false.
`assertion.errored` means evaluation could not produce a boolean result because
of missing data, type errors, unsupported operators, or runtime evaluator
errors. Both are non-mutating: no facts, effects, dependencies, or provider runs
may be committed by the assertion. Re-running the same assertion against the
same program version, instance sequence, and read set must produce the same
idempotency key so recovery does not duplicate assertion diagnostics.

Validation acceptance for this layer:

```text
whip dev examples/implementation-plan-phase-review.whip --provider codex --until idle
```

must create one `PhaseReviewRequest` fact for each implementation-plan phase,
enqueue corresponding `agent.tell` effects, run configured Codex turns, and
leave status/evidence sufficient to explain every dispatched or blocked phase.

## Control Plane Responsibilities

The control plane owns mechanical reliability:

- compile and validate programs
- create and recover instances
- serialize rule commits per instance
- append events
- materialize facts
- enqueue effects
- persist effect dependency edges
- lease and dispatch effects to workers
- record provider runs and artifacts
- record evidence linking events, rules, effects, runs, artifacts, skills, and
  policy decisions
- expose status/log/fact/effect views
- expose inbox, trace, evidence, plugin, and skill views
- pause/resume/cancel instances
- recover abandoned leases

It calls the kernel operations defined in [kernel-api.md](kernel-api.md) for
instance transitions. It should not duplicate rule-commit, effect-claim, or
completion semantics in ad hoc operational code.

The source language owns policy:

- when work should start
- which facts imply readiness
- when to ask humans
- when to retry or escalate
- which typed model decisions are needed

The control plane should not become a gateway that owns every integration.
Plugins and separate kernels own domain behavior; the control plane owns
durability, authority binding, visibility, and lifecycle.

## Concurrency Model

Concurrency exists at three layers:

1. Many instances may run at once.
2. One instance may have many outstanding durable effects.
3. Provider workers may execute effects in parallel subject to capability and
   profile policy.

Rule commits inside one instance are serialized. This avoids exposing authors to
distributed transaction bugs. Concurrency is expressed through durable facts and
effects, not simultaneous mutation of workflow state.
