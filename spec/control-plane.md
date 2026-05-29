# Control Plane

Status: draft

Whippletree runs many workflow instances concurrently. A `.whip` source file is
not itself a process. It compiles into a versioned program, and each execution is
a durable instance managed by the local or hosted control plane.

## Core Objects

```text
Program       compiled source plus version hash
Instance      one durable running copy of a program
Event         append-only observation for an instance
Fact          current materialized truth for an instance
Effect        durable request for external work
EffectEdge    durable dependency between two effects
Run           provider execution attempt for an effect
Capability    registered external authority or effect surface
Profile       policy bundle for agent/tool authority
Runtime       daemon/control-plane process
Artifact      durable file/log/output associated with a run
Evidence      causal record linking events, rules, effects, runs, and artifacts
Skill         deterministic context bundle for an agent or turn
Plugin        package-provided effect/fact-schema/resource extension
InboxItem     pending human review request
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
- declared capabilities
- declared profiles
- declared skills
- declared fact/event schemas
- analysis results
- optional generated verification artifacts
- optional generated BAML artifacts

Deploying a program does not run it. Starting creates an instance.

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

whip plugins
whip skills
whip inbox
whip trace <instance>
whip evidence <run-or-effect>
```

`whip dev` is a convenience command for dogfooding. It should compile,
start one instance, run local effect workers, and stream useful status.

`whip run` may create and start an instance without driving it. The control
plane still needs an explicit driving surface for local dogfooding:

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
   - `NewEffect` records for `tell`, `coerce`, `claim`, `askHuman`, `call`,
     and `emit`
   - `NewEffectDependency` records for `after` blocks
   - evidence/diagnostic records for policy decisions and lowering details
6. Commit each rule atomically through the kernel.
7. Stop when no additional rules are ready, the configured step limit is hit,
   or the instance becomes paused/cancelled/terminal.

`whip step` must never execute providers. It only creates durable facts and
outbox effects.

Rule readiness includes pure guard evaluation. For a rule with `when Class as
binding where <expr>`, the stepper must bind candidate facts first, evaluate the
typed expression against those bindings, and discard candidates whose guard is
false before lowering the rule body. Guard evaluation is part of deterministic
stepping and must not consult providers, BAML, plugins, the filesystem, the
network, wall-clock time, or random sources.

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

`whip dev` composes `step` and `worker` for one local dogfood session. It should
stream status, stop at idle/blocked/terminal states, and make every provider
decision visible in the store.

## Rule Lowering Requirements

The control plane needs a production lowering pass from typed IR and source spans
to concrete store writes. This pass must not rely on ad hoc prompt parsing.

For each rule body construct:

```text
record Class { ... }        -> typed fact projection
tell agent ...              -> agent.tell effect with target/profile/skills
coerce function(...)        -> baml.coerce effect with function and arguments
claim issue with loft       -> loft.claim effect
askHuman ...                -> human.ask effect
call plugin.capability ...  -> capability.call effect
emit event                  -> event.emit effect
after effect succeeds       -> dependency edge
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

Dogfood acceptance for this layer:

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
