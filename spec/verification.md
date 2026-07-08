# Verification Strategy

Status: draft

WhippleScript needs two related but separate verification tracks:

1. Validate the architecture before and after implementation.
2. Statically analyze user WhippleScript programs as a product feature.

The same semantics should feed both tracks, but the tools should not be forced
into one shape.

## Tool Roles

### Maude: Rule Kernel And Program Semantics

Maude is the primary tool for the WhippleScript language kernel. Maude system modules
specify rewrite theories, and rewrite rules represent local state transitions.
That matches WhippleScript's core model:

```text
facts + events + effect queue + dependency edges + rewrite rules
```

Use Maude for:

```text
rule commits
guard/readiness evaluation
workflow assertions
effect graph dependency behavior
claimability
completion events
bounded searches for bad rule cycles
generated per-program counterexample checks
diagnostic adequacy for finite static rejection paths
```

Maude should be the first formal target generated from typed WhippleScript IR.

### TLA+/Apalache: Durable Runtime Lifecycles

TLA+ is the right fit for the control-plane lifecycle because the hard bugs are
asynchronous and temporal:

```text
event log append
projection catch-up
effect run start
lease expiry
worker crash
retry
idempotency
recovery
late completion
pause/resume/cancel
```

TLA+ specifications are useful design artifacts for asynchronous systems, and
Apalache provides bounded/symbolic checking for TLA+ transition systems. Use
TLA+/Apalache for architecture validation, not for every user workflow in v0.

### Veil/Lean: Later Assurance Layer

Veil is a Lean-embedded framework for specifying, testing, and proving safety
properties of state transition systems. It provides model checking and SMT-style
automation with Lean available when automation falls short.

That is powerful, but too heavy for the critical path while the kernel is still
moving. Keep Veil as a later hardening target for stable safety invariants:

```text
dependency safety
idempotent recovery
trace conformance
capability enforcement facts
```

Do not require Veil for v0 development gates.

## Architecture Validation

Architecture validation checks whether the runtime design is sound independent
of any particular user program.

### Maude Kernel Model

Represent runtime state as a Maude configuration:

```maude
< events  : EventLog
  facts   : FactSet
  effects : EffectQueue
  deps    : EffectDependencies
  clock   : Time
  control : RuntimeMeta >
```

Each WhippleScript rule lowers to one or more Maude rewrite rules. External systems
are modeled nondeterministically:

```text
effect requested -> effect completed | failed | timed_out
blocked effect -> claimable effect, when dependency predicates are satisfied
```

Initial checks:

- a rule cannot fire unless all fact matches are present and its guard evaluates
  to true
- false or error guards do not commit facts/effects
- assertion failures are observable diagnostics/evidence, not workflow-state
  mutations
- a dependent effect cannot run before its dependency predicate is satisfied
- dependency failure blocks downstream success-only effects instead of running
  them
- source order never creates ordering
- capacity cannot go negative
- no internal rewrite sequence can enqueue unbounded effects
- stuck states are explainable as waiting for external events, human input,
  dependency satisfaction, policy, or unavailable capacity

### Admission And Replay Determinism Model

A dedicated Maude model (`models/maude/admission.maude`, with tests under
`models/maude/tests/admission.maude`) checks the operational half of the
[`admission-and-idempotency.md`](admission-and-idempotency.md) contract — the
parts that are about rewrite/replay determinism rather than concurrency:

- the pure-rule fixpoint reaches a canonical fact set under the deterministic
  ordering, independent of rewrite interleaving
- replaying the recorded log reproduces identical projections, and a
  `re-invoke source` transition is *forbidden* during replay (record-once)
- admission validation is a gate: an invalid candidate value admits no fact
- fact-batch admission is atomic: a partial batch is an unreachable state
- the same admission identity key never produces two distinct facts

Concurrency/recovery properties (exactly-once external effect, idempotency under
re-delivery/retry across crash points) are the TLA+ side above; this Maude model
covers the deterministic rewrite/replay obligations. Where a property depends on
real compiler/runtime output, prefer generated checks over hand-written modules,
per the generated-Maude approach below.

### Diagnostic Adequacy Model

Diagnostic adequacy is a formal obligation, but only for structure. Do not model
English wording. Model these properties:

```text
completeness  rejected static/lifecycle paths emit or require diagnostics
soundness     diagnostics correspond to failed invariants
provenance    diagnostics cite the rejected object or transition
ownership     packages/providers cannot assert diagnostic completeness
```

For Maude, add diagnostic adequacy searches to the construct graph, package
contract, lowering, and generated artifact bridges:

- missing required port implies `construct.missing_requirement`
- ambiguous exactly-one or optional-one resolution implies
  `construct.ambiguous_resolution`
- resource/type/phase/version mismatch implies the corresponding construct
  diagnostic code
- unsupported lowering class implies `lowering.unsupported_class`
- duplicate lowered core-object ownership implies
  `lowering.duplicate_core_object_owner`
- package-supplied acceptance or diagnostic-complete facts are rejected or
  ignored

For runtime/TLA+/trace conformance, check that denied or non-authoritative
transitions are durable and explainable:

- capability denial records a diagnostic and starts no provider run
- assertion failure/error records diagnostic/evidence and mutates no user facts
  or effects
- provider failure records terminal diagnostic or terminal evidence
- stale completion records diagnostic/evidence and does not become authoritative
- script hard-off records `security.script_disabled` and crosses no exec
  boundary

The full diagnostic quality bar remains a fixture/snapshot concern described in
[`error-handling.md`](error-handling.md). Formal models should pin codes,
provenance classes, and rejection relationships, not rendered prose.

### TLA+/Apalache Control-Plane Model

Model control-plane actions:

```text
AppendEvent
DeriveFacts
CommitRule
EnqueueEffectGraph
SatisfyDependency
ClaimEffect
StartRun              # this models the external side effect starting
CompleteRun
FailRun
ResolveUncertainRun  # recovery: started-without-terminal -> single uncertain terminal
ExpireLease
RecoverLease
PauseInstance
ResumeInstance
CancelInstance
```

`StartRun` is the external side effect; `ResolveUncertainRun` is the recovery
resolution of a run that started but whose terminal was never appended (no
idempotent provider re-query available), resolving it to a single `uncertain`
terminal rather than re-executing. Admission *identity/validation/batch* — the
`AdmitFact` / `AdmitFactBatch` / single-fact-per-key obligations — are checked in
the Maude admission model, not duplicated as TLA actions.

Initial safety invariants:

- every run references an existing effect
- no effect has more than one successful terminal completion
- no run records more than one terminal event (`NoDuplicateTerminalRunEvents`)
- **exactly-once external effect** (`TerminaledRunStaysTerminal`): a run that
  recorded a terminal — including an `uncertain` recovery resolution of a
  started-without-terminal run — never reverts to an executing status, so its
  external side effect is never silently re-executed; a retry is a fresh run
- no provider run starts unless the effect is claimable
- no claimable effect has unsatisfied dependencies
- retry reuses effect identity unless the program creates a new attempt
- paused instances do not commit new effectful rewrites
- recovery does not reorder the per-instance event log
- projections are derivable from the log and committed rule steps

The `TerminaledRunStaysTerminal` invariant and the `ResolveUncertainRun` recovery
action implement the run-lifecycle exactly-once half of the
[`admission-and-idempotency.md`](admission-and-idempotency.md) contract and are
Apalache-checked in `ControlPlaneLifecycle.tla`. The admission *identity*
(single-fact-per-key), *validation gate*, and *fact-batch atomicity* obligations
are the Maude admission model's domain (above), not duplicated here.

Initial liveness/fairness goals, checked only after safety stabilizes:

- under fair workers, claimable effects eventually run or become terminal
- expired leases eventually recover or become terminal
- unprocessed events eventually reach the projection cursor

Current implementation names these goals in `ControlPlaneLifecycle.tla` as
`FairSpec` and `LivenessGoals`, with per-property formulas for claimable
effects, running leased effects, projection catch-up, and recovery completion.
The default repository check typechecks those temporal formulas. Full temporal
proof remains a later hardening activity rather than a v0 release blocker.

## Runtime Trace Conformance

The implementation should emit enough evidence to replay its behavior against
the abstract lifecycle model.

Trace checker input:

```text
events
facts
effects
effect_dependencies
runs
leases
evidence
diagnostics
```

Trace conformance should reject:

- run started while dependency unsatisfied
- completion for unknown effect
- duplicate terminal completion
- run started after cancellation
- dependency output used before success
- blocked_by_dependency for an effect whose dependency edge listens for failure
- event sequence gaps inside one instance

This is the bridge between formal specs and the Rust implementation.
Current `trace --check` validation reconstructs abstract lifecycle records from
the per-instance event log, but first checks the raw store event sequence for
gaps so reconstruction cannot hide missing durable events. Reconstructed
`effect.blocked` records preserve structured blocked statuses when present;
`blocked_by_dependency` is accepted only when the effect has an unsatisfied
dependency in the reconstructed lifecycle state.

## Static Analysis Of WhippleScript Programs

Compiler checks should be fast, local, and explainable:

```text
type checking
expression kernel typing
read/write/consume sets
guard satisfiability over literal/enum domains
effect graph validation
output binding scopes
rule dependency graph
recursion strata
idempotency key derivability
capability/profile requirements
capacity/resource constraints
```

Generated Maude checks should be optional at first:

```sh
whip check workflow.whip --model-search
```

They should provide bounded counterexamples for unsafe orchestration patterns,
not require authors to understand Maude.

Current implementation generates temporary Maude modules from typed IR plus the
emitted compiler artifacts and runs the searches through:

```sh
whip check --model-search workflow.whip
```

This executable path requires Maude, Python, and the Python `jsonschema`
package. `whip doctor` reports the Python interpreter and `jsonschema` import
check separately so a missing bridge dependency is visible before running a
generated search.

Artifact evidence admission is also executable without Maude. `whip
verify-report <report.json>` accepts successful `check --json` arrays,
successful `compile --json` objects, and full
`whipplescript.verified_artifacts.v0` bundles emitted with
`--emit construct-graph`, `--emit lowered-ir`, or `--emit artifacts`;
it rejects missing or wrong report schema identities and
malformed success-report envelopes, enforces the bundle's emit-specific
artifact surface, recomputes `ir_hash` from the embedded
snapshot, checks `construct_graph.graph_id` against
`construct_graph.source_digest`, checks
`lowered_ir_report.accepted_program_digest` against `graph_id + snapshot`,
recomputes the Rust construct-graph and lowered-IR validator facts, checks
graph/lowered identity, rejects validator diagnostics, compares the exact
validator-owned `derived_facts` traces, enforces the verifier platform catalog
including package-authorable lowerings and lowering lifecycle/static/authority
profiles, and, when the report source path is readable, recompiles the source
and re-emits the construct graph and lowered IR report from the admitted
registry context to catch self-consistent stale artifacts. It also validates
`model_search` ledger counters, IR obligation artifacts, and artifact-search
obligation artifacts when a ledger is present. The
companion `scripts/validate-artifact-reports.py` checker applies the same
artifact admission contract through the Python bridge/schema path: graph-only
verified bundles receive construct-graph admission, and lowered/full bundles
add lowered-IR identity and trace admission. The report schema gate remains the
deeper structural JSON Schema check for report envelopes and verified artifact
bundles. Together they make report CI fail on stale, weakened, padded, spoofed,
erased, or duplicated artifact evidence before the more expensive formal
searches run. `scripts/check-artifact-admission-differential.sh` keeps the
native and Python admission implementations in lockstep by mutating real
emitted reports and requiring `whip verify-report` and
`scripts/validate-artifact-reports.py` to agree. Its negative corpus includes
construct-graph identity collisions, lowered core-object duplicate IDs, and
duplicate lowering ownership across node, node/edge, edge/edge, and dependency
owners.

For downstream tooling, `whip verify-report --emit
construct-graph|lowered-ir|artifacts <report.json>` runs that same admission
sequence and then emits a `whipplescript.verified_artifacts.v0` bundle. Generated
checker bridges and later compiler stages should consume that admitted bundle,
or perform an equivalent admission check themselves, rather than reading
unchecked embedded artifacts directly from a report envelope. The bundle retains
the admitted `snapshot`, `ir_hash`, and construct-graph identity. Lowered/full
bundles also retain lowered-IR identity so consumers can still recompute the
full lowered artifact digest chain.
`lowered-ir` bundles include the construct graph because lowering preservation
is graph-relative; both `lowered-ir` and full `artifacts` bundles can be fed
back into `whip verify-report`. `construct-graph` bundles remain targeted
partial inputs for construct-graph bridge and Python artifact-admission checks;
they can be fed back into native `whip verify-report` for graph-level
revalidation or re-emitted as construct-graph bundles, but asking native
`whip verify-report` to emit `lowered-ir` or `artifacts` from them is rejected
because no `lowered_ir_report` has been admitted. The formal release gate
exercises that handoff directly: it
emits construct-graph and lowered-IR verified bundles from a generated check
report, feeds those bundles to the generated Maude bridges, requires their
search counts to match the original admitted report, and checks that multi-entry
verified bundles are rejected unless `--entry-index` selects the artifact to
lower.

The generated IR searches verify that downstream effects cannot run while an
upstream dependency is still queued, that satisfying terminal states release the
downstream effect, that non-satisfying terminal states do not release
success/failure-specific branches, and that guards, terminal branches,
revision-scoped effects, and assertions preserve their finite model contracts.

When `construct_graph` and `lowered_ir_report` are present, `--model-search`
also runs generated artifact bridge searches. These prove construct graph node
and edge acceptance, graph aggregation, lowered IR node/edge preservation,
lowered core-object ownership/accounting, graph lowering preservation, and
runtime lifecycle handoff for the covered package-backed capability-call slice.
The same artifact-search ledger also includes `artifact.platform_catalog`
entries proving that the compiler-emitted platform catalog's lowering classes
satisfy static-safety and authority-profile obligations in
`lowering-class-lifecycle.maude`. The CLI writes a temporary
`whipplescript.verified_artifacts.v0` bundle and feeds that admitted bundle to
the construct-graph and lowered-IR bridge scripts, then feeds the same explicit
verifier catalog to the platform-catalog bridge, so the generated checker path
uses the same artifact admission boundary as standalone bridge consumers.
The JSON report separates `ir_searches` from `artifact_searches` so CI can prove
both paths ran. Successful reports also include an `obligations` ledger with one
entry per generated search. Each entry records the obligation category, source
span, formal predicate edge, expected outcome, and actual outcome; the release
gate checks the ledger length, category distribution, expected/actual match,
category-local order, and exact artifact identity fields. For artifact searches,
the reusable `scripts/validate-model-search-report.py` checker re-derives the
expected construct-graph, lowered-IR, and platform-catalog obligations from the
emitted report and admitted verifier catalog, so a stale node id, port edge,
dependency ref, aggregate predicate, catalog lowering, or source span is
rejected even when aggregate counters still add up. The checker performs the
same ledger validation for any successful `check --json --model-search` or
`compile --json --model-search` report. `whip verify-report` performs the same
native artifact-ledger identity checks for reports that include `model_search`.
Artifact-search entries are also compared against the compiler-emitted
`artifact_model_search_obligations` artifact, which is bound to the report
`source_hash`, `ir_hash`, package-contract digest, construct-graph ID, and
lowered accepted-program digest. The Python report validator validates that
artifact against the standalone
`artifact_model_search_obligations_v0` schema; native `whip verify-report`
enforces the admission-critical fields directly and compares every non-IR ledger
row to the emitted artifact by category-local row index, category, description,
formal endpoints, expected outcome, and source span. This catches ledger
tampering that changes an artifact row without changing its durable obligation
artifact, and it also catches stale durable artifact rows before re-derived
artifact obligations are checked against the admitted graph/lowered/catalog
state.
IR obligation entries are also compared against the compiler-emitted
`ir_model_search_obligations` artifact, which is bound to the report
`source_hash` and `ir_hash` and records the generated formal edge, expected
outcome, and source span for each IR search. The Python report validator
validates that artifact against the standalone
`ir_model_search_obligations_v0` schema; native `whip verify-report` enforces
the admission-critical fields directly and then compares every IR ledger row to
the emitted artifact. Each IR row must also be supported by the embedded
`snapshot` structure: guard searches require guarded rule triggers, dependency
searches require matching effect dependencies, terminal-branch searches require
terminal branch metadata, assertion searches require an existing assertion
index, and revision searches require matching rule/effect or dependency
structure. This closes ledger tampering that changes an IR row without changing
its emitted obligation artifact, or rewrites the ledger and obligation artifact
together to point at a nonexistent snapshot endpoint. The validators also derive
the exact ordered IR-search sequence from `snapshot`, including the generated
description, expected search count, predicate distribution,
`(upstream, predicate, downstream)` endpoint multiset, per-endpoint outcome
distribution, and row order. A ledger and obligation artifact that agree with
each other but omit a generated IR search, swap one generated predicate family
for another, duplicate one supported endpoint while dropping another, flip a
generated expected outcome, reorder otherwise valid rows, or carry a stale
generated description are rejected.
IR obligation source spans are also compared against construct-graph anchors
when the obligation has a unique graph-backed source: guard obligations use
compiler-owned `model_search.guard_source` facts, terminal-branch obligations
use compiler-owned `model_search.terminal_branch_source` facts, dependency
obligations use the matching `effect_dependencies` span, assertion read-only
obligations use the ordered assertion node span, revision rule obligations use
the rule `when0` node span, and revision effect-attribution obligations use the
unique matching effect node span. Dependency coverage includes `succeeds`,
`fails`, `completes`, and `revision-completes-cancelled` obligations. Full
re-derivation of ambiguous guard, terminal-branch, and dependency source spans
remains future work for portable report bundles when the source path is not
readable and the construct graph does not provide a unique anchor. For reports
whose source path is readable, native `whip verify-report` already recompiles
the source for graph/lowered artifact identity and also regenerates the IR
model-search obligations, so coordinated tampering of the ledger,
IR-obligation artifact, and graph source evidence still fails against the
source-derived span.

The standalone formal gate also runs the package-contract bridge. It generates
`whip package check --json std/manifests/memory.json`, admits the embedded
`package_contract` only after schema, digest, platform-catalog, and empty
diagnostic checks, then proves package effect-contract acceptance and
capability-call declaration source lowering against `package-contract.maude`.
The package construct-grammar bridge consumes the same admitted artifact and
proves the package-declared capability-call construct against
`construct-grammar.maude`, including the capability-only construct shape used by
the current memory package.
The standalone formal gate also runs the platform-catalog bridge. It admits the
compiler-emitted `whip package catalog` payload and proves that every declared
lowering class satisfies the Maude lifecycle model's static-safety and authority
profile rules. It intentionally does not infer output profiles from the catalog;
output object and runtime-entrypoint preservation remains a lowered-IR report
obligation.

The same bridge scripts also accept a successful `compile --json` report. This
keeps compile artifacts executable: CI can feed compiler output through
`whip verify-report --emit artifacts` and then into
`scripts/construct-graph-to-maude.py` and `scripts/lowered-ir-to-maude.py`
without first wrapping it in a check-report array. Before emitting Maude, those
bridge scripts check the surrounding digest chain: `ir_hash` from `snapshot`,
package contract digest, empty package-contract and contract-registry
diagnostics, package-contract platform version and construct catalog identity,
construct-graph package references, graph id from source digest, and, for
lowered IR, accepted-program digest from `graph_id + snapshot`.
The construct-graph bridge emits accepted-program obligations after those
admission checks, so generated searches cover graph acceptance plus the
compiler-emitted adequacy predicates for the admitted graph artifact.
The lowered-IR bridge likewise requires compiler-emitted validator predicates
for graph coverage, aggregate and per-field node/edge/dependency preservation,
aggregate and per-field node lifecycle inputs, aggregate and per-field output
compatibility, core-object entrypoints and owners, and graph-wide lowering
boundary evidence before emitting preservation and runtime-handoff searches.
The CLI `--model-search` path binds bridge admission to the compiler-owned
catalog by writing the current `whip package catalog` payload to a temporary
file and passing it to bridge subprocesses with `--platform-catalog`.
`scripts/check-formal-models.sh` does the same for standalone bridge runs by
generating `whip package catalog` and passing it to each bridge invocation with
`--platform-catalog`. Bridge invocations outside those paths must bind the
compiler-emitted catalog explicitly; direct script users may pass
`--platform-catalog <path>` or set `WHIPPLESCRIPT_PLATFORM_CATALOG_PATH`.
Missing verifier catalog binding is an admission error.
`compile --json --model-search` runs those generated obligations through the
CLI and reports the same `model_search` counters as `check --model-search`,
including the `artifact.platform_catalog` rows. Standalone bridge runs over
multi-entry check reports or verified artifact bundles must pass
`--entry-index <n>`; otherwise the bridge rejects the report rather than
silently verifying only one entry.

Generated per-program checks must keep those dependency/expression searches and
the artifact bridge searches in sync with the emitted reports. The generator
should emit finite Maude modules per checked WhippleScript program or artifact,
with symbols for the program's rules, facts, effects, dependency edges, guard
outcomes, assertion checkpoints, construct graph nodes/ports/edges, lowered
core objects, and source-span anchors. Validation must use generated searches
over those modules, not only hand-written abstract examples.

For every guarded rule, generated searches should assert:

- a `ruleCommitted(<rule>)` state is reachable only from a matching fact set
  where the lowered guard predicate evaluates to `true`
- no fact write, consume, or effect graph commit for that rule is reachable
  from the same matching fact set when the guard evaluates to `false`
- no fact write, consume, or effect graph commit for that rule is reachable
  when guard evaluation produces `error`
- an error guard may reach a diagnostic/evidence state, but that state must
  preserve the pre-rule user facts and effect queue

For every assertion checkpoint, generated searches should assert:

- assertion `pass` preserves the normal reachable state space
- assertion `fail` cannot create, consume, or mutate user facts
- assertion `fail` cannot enqueue, release, start, or complete effects
- assertion `error` has the same non-mutation guarantees as `fail`
- failure/error diagnostics or evidence are allowed only on diagnostic/evidence
  surfaces, not as workflow-state commits

For effect dependencies, generated expression-kernel checks must preserve the
existing generated searches:

- downstream effects cannot run while an upstream dependency is still queued
- satisfying terminal states release matching downstream branches
- non-satisfying terminal states do not release success/failure-specific
  branches
- dependency checks still run for effect graphs committed through a true guard
  and are not weakened by adding guard/assertion searches

When a generated search returns an unexpected result, the CLI reports a
source-span diagnostic at the matching `after <effect> <predicate>` dependency
anchor and includes the expected and actual Maude result.

The CLI test suite includes an expected-failure generated-check fixture: it
compiles a real WhippleScript example, injects one unsafe dependency-release rewrite
into the generated Maude module, and asserts that Maude finds the resulting
counterexample when `maude` is available on `PATH`.

Do not generate TLA+ per user program in v0. TLA+ models the runtime/control
plane; Maude models user program behavior.

### Expression-Kernel Maude Model

The expression-kernel model should be a finite abstraction of
[expression-kernel.md](expression-kernel.md), not an interpreter for JSON or
strings. It should add a readiness gate ahead of the existing rule/effect graph:

```text
fact match + guard true  -> rule can fire
fact match + guard false -> no rule rewrite
fact match + guard error -> diagnostic, no graph commit
```

The hand-written Maude model should cover:

```text
typed true/false/error guard results
optional present/missing paths
enum/literal domain checks
membership over finite arrays/maps
count/empty over finite projections
assertion pass/fail/error
declared vs undeclared AgentRef targets
```

Generated per-program Maude should lower parsed guards and assertions to
abstract predicates over finite fact/effect symbols. It should not try to prove
semantic truth of provider/model output. A schema-coercion result is modeled
only as a typed success/failure/timeout/cancel event.

Initial expression-kernel searches:

- no `ruleCommitted(<rule>)`, fact mutation, consume, or `graphCommitted`
  state is reachable from a false guard
- no `ruleCommitted(<rule>)`, fact mutation, consume, or `graphCommitted`
  state is reachable from an error guard
- a guard error can produce a diagnostic/evidence state while preserving the
  previous facts and effect queue
- a true guard permits the same effect graph checks already used by dependency
  searches
- an assertion failure cannot create, consume, or mutate user facts or enqueue,
  release, start, or complete effects
- an assertion error has the same workflow-state non-mutation guarantee as
  assertion failure
- an undeclared dynamic agent target cannot create an `agent.tell` effect
- two evaluations over the same projection cannot produce different guard
  results

## CI Policy

Run TLA+/Apalache in default CI for v0 because it checks the durable
control-plane lifecycle rather than per-user workflows. Keep generated
per-program Maude checks opt-in through `whip check --model-search` so
ordinary authoring checks do not require every formal tool or Python bridge
package locally.

The CI path should call `scripts/check-tla-models.sh`; that script owns the
Apalache/Nix fallback and keeps the workflow definition independent of local
tool installation details.

## Scope Boundary

Verification does not prove:

- prompt correctness
- external agent correctness
- filesystem correctness
- semantic truth of model classifications
- correctness of external provider internals

It proves orchestration-kernel properties under typed contracts for external
results.

## Source Notes

The strategy is based on:

- Maude documentation describing rewrite theories and system modules as local
  state-transition systems:
  <https://maude.cs.uiuc.edu/maude1/manual/maude-manual-html/maude-manual_13.html>
- TLA+ guidance emphasizing asynchronous systems, atomicity choice, and
  specification as a way to reveal design errors:
  <https://lamport.org/pubs/lamport-spec-tla-plus.pdf>
- Apalache documentation describing TLA+ specs as transition systems and
  symbolic/bounded model checking:
  <https://apalache-mc.org/>
- Veil documentation describing Lean-embedded transition-system verification:
  <https://veil.dev/docs/>

## Implementation Path

1. Hand-write the Maude effect-graph kernel model.
2. Hand-write the TLA+ control-plane lifecycle model.
3. Add a lightweight trace-conformance contract to the runtime-store and
   observability specs.
4. Encode Ralph loop and Loft-claim-before-agent-turn examples.
5. Add generated Maude from typed rule IR once the parser/IR exists.
6. Reevaluate Veil after the kernel semantics stop moving.
