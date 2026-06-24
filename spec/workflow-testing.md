# Workflow Testing

Status: implementation-grade target for user-facing workflow tests

WhippleScript user tests should be small scenario tests over workflows. They
should not ask users to test the runtime kernel, event log, lease protocol, or
construct graph invariants. The platform and formal models own those. User
tests protect workflow intent:

```text
Given this starting world,
when this workflow runs,
then these observable outcomes should hold.
```

## Design Source

coerce's testing surface is a useful reference point: tests are first-class
authoring artifacts, colocated with source, runnable from the CLI and editor,
and built from function inputs plus assertions. WhippleScript should borrow the
low-friction authoring loop, but adapt it to durable orchestration instead of
LLM function calls.

The WhippleScript equivalent is:

```text
test scenario = given + stub + run + expect
```

There is one user-facing abstraction: a deterministic test scenario.

## Non-Goals

This spec does not add:

- a `std.test` package
- a separate testing runtime language
- property testing
- judge-model evals
- human-eval gates
- matrix syntax in v0
- snapshot update workflows in v0
- live provider tests by default
- package-specific test DSLs

Testing is an authoring tool. Test declarations are not deployable workflow
behavior and do not lower into runtime program IR.

## Source Shape

Target syntax:

```whip
test "failed CI gets triaged" {
  workflow TriageFailedRun

  given signal github.workflow_failed {
    repo "acme/app"
    run_id "run_123"
    branch "main"
  } duplicated

  stub agent triager succeeds {
    summary "Migration failed in step 3"
    priority High
  }

  run until idle

  expect issue count where external_id == "run_123" is 1
  expect message sent to ops where body contains "run_123"
  expect no script.run
}
```

`test` is a top-level source artifact. Tests may live beside workflows in `.whip`
source files or in test-only source bundles. `whip check` should parse and
validate test declarations, but `whip compile`, `whip run`, and deployment paths
must not include them in executable workflow IR.

## Four Verbs

The whole user-facing test model is four verbs.

### `given`

`given` seeds the starting world.

It covers:

- workflow input
- facts
- external signals
- clock time
- existing tracker issues
- existing file contents
- existing memory fixture contents
- existing message/channel state when the package supports it

Examples:

```whip
given input {
  repo "acme/app"
}

given fact Issue {
  title "Crash on startup"
  priority High
}

given signal github.workflow_failed {
  repo "acme/app"
  run_id "run_123"
}

given clock at "2026-06-14T09:00:00-04:00"

given file project_files at "docs/template.md" "## Triage template"

given tracker backlog issue {
  external_id "run_123"
  title "Existing issue"
}
```

`given` does not bypass package validation. A `given signal` still goes through
typed signal admission. A `given file` still goes through the declared file
store policy. A `given tracker` issue still uses the package's fixture
projection model.

> **v0 harness status.** `given signal`, `given input`, `given fact`, and `given
> clock` are executed: `given input` is validated against the workflow's input
> contracts and seeded as the declared input fact (the same path as `whip run`),
> `given fact` seeds a pre-existing fact that `when <Type>` rules see on the first
> step, and `given clock at "<timestamp>"` injects a virtual evaluation clock so
> timer and deadline firing (`timer until`, `timeout`) is deterministic and
> instant — the harness binds that timestamp wherever the worker would otherwise
> read the wall clock, so `run until idle` can advance a deadline (or hold it)
> without a real sleep. `given tracker <name> issue { … }` is executed: the harness
> isolates the builtin tracker per scenario and seeds the issue through the real
> projection path, so a `<queue> has ready item` rule fires on it. `given file
> <store> at "<path>" "<content>"` is executed: the harness writes the content to a
> per-scenario temp dir and redirects the named `file store`'s root to it, so a
> `read` runs through the real worker against the fixture (not the workflow's
> declared path). An escaping fixture path (absolute or `..`) and a `<store>` the
> workflow does not declare are rejected as setup errors. v0 takes the content as a
> plain string literal; a `contains """…"""` multiline heredoc form is a planned
> ergonomic variant.

### `stub`

`stub` controls nondeterministic boundaries.

It covers:

- agents
- schema coercion
- tracker providers
- memory providers
- file providers
- messaging providers
- scripts
- provider feature reports
- capability denials

Examples:

```whip
stub agent triager succeeds {
  summary "Migration failed"
  priority High
}

stub agent triager fails "model refused"
stub agent triager times_out
stub agent triager returns_invalid_output

stub coerce ClassifyIssue returns {
  priority High
}

stub memory project_memory returns_empty
stub tracker backlog claim_lost
stub tracker backlog conflict_on_update
stub file project_files missing "docs/template.md"
stub message ops delivery_failed "channel unavailable"
stub capability script.run denied
stub provider codex feature_unavailable "/goal"
```

Stubs map to deterministic fixture provider outcomes. They do not alter
workflow source, skip authorization, or create hidden runtime behavior.

> **v0 harness status.** The `whip test` driver settles queued effects through
> the fixture provider, with `stub` selecting the outcome: `succeeds` and `fails`
> are honored. `times_out`/`cancels` are **rejected as unsupported** — the fixture
> agent path is a shell-command harness (exit-0/non-zero only) that cannot
> faithfully simulate a timeout, so the harness reports them `invalid` rather than
> silently completing the turn. **Per-agent outcomes are supported**: one scenario
> may stub agents differently (e.g. `stub agent alpha succeeds` + `stub agent beta
> fails`), keyed by agent name; the global outcome backs coerce/non-stubbed effects.
> **Coerce output injection is supported**: `stub coerce <fn> returns { … }` sets
> the typed value a fixture coerce returns (what a workflow branches on). Agent
> turn-output injection (`succeeds { summary … }`) is still deferred — the agent
> turn output is a fixture-generated fixed schema — and reported as `invalid`
> rather than silently ignored.

### `run`

`run` advances the deterministic runtime.

Examples:

```whip
run until idle
run for 5 steps
run until workflow completed
run until workflow failed
run until effect agent.tell requested
run until diagnostic capability.not_granted
```

The default should be `run until idle` when a test omits an explicit `run`.
The runner must still enforce an iteration cap and report an actionable
diagnostic if the workflow does not reach the requested condition.

### `expect`

`expect` asserts visible outcomes.

It covers:

- workflow terminal state
- facts and projections
- package projections such as issues, messages, files, memory writes
- requested/completed/failed effects
- rule firing (the rule lens — see below)
- diagnostics
- forbidden capabilities/effects

Examples:

```whip
expect workflow completed
expect workflow failed with TriageBlocked

expect issue exists where title contains "Migration failed"
expect issue count where external_id == "run_123" is 1
expect message sent to ops
expect file project_files at "reports/triage.md" exists
expect memory project_memory learned where topic == "run_123"

expect effect agent.tell completed
expect diagnostic provider.feature_unavailable
expect no script.run
expect no duplicate issue where external_id == "run_123"

expect rule triage_failed_run fired
expect rule triage_failed_run did not fire
expect rule triage_failed_run fired 2 times
```

Expectations should use projections and package-domain vocabulary by default.
Raw event ids are too brittle for ordinary user tests.

#### The rule lens

WhippleScript is an event-sourced *rule* system, and a stated goal of user tests
is that "rules are neither too broad nor too narrow." Projection-only
expectations can only observe a rule's effects indirectly, so the test surface
also exposes a minimal rule-firing assertion: `expect rule <name> fired`,
`did not fire`, and `fired N times`. This asserts that the named rule committed
(or did not commit) for the scenario — testing rule breadth directly without
exposing raw event ids or the internal commit log. It is the only rule-internal
hook; consume/produce-set assertions remain out of the user surface (they are
platform-owned, covered by `testing-strategy.md`).

## Scenario Grammar And Predicate Language

The verbs above are not free text; they parse through the core parser as a
`test_scenario` construct (see [`construct-grammar.md`](construct-grammar.md)),
which `whip check` validates and `compile`/`run` exclude from IR. The grammar:

```text
test <string> "{" [ workflow_clause ] { given_clause | stub_clause } [ run_clause ] { expect_clause } "}"

workflow_clause := "workflow" Ident   ; binds the scenario to one workflow in a
                                      ; multi-workflow bundle; optional in a
                                      ; single-workflow file

given_clause   := "given" ( "input" record
                          | "fact" TypeRef record
                          | "signal" DottedName record
                          | "clock" "at" StringLiteral
                          | "file" Resource "at" Expr "contains" StringLiteral
                          | "tracker" Resource "issue" record
                          | <package-declared given surface> )
stub_clause    := "stub" <surface> <outcome> [ record | StringLiteral ]
run_clause     := "run" ( "until" stop_condition | "for" Number "steps" )
expect_clause  := "expect" expect_target

stop_condition := "idle" | "workflow" ("completed"|"failed")
                | "effect" EffectName "requested"
                | "diagnostic" Code
expect_target  := "workflow" ("completed" | "failed" ["with" TypeRef])
                | proj_query | "effect" EffectName effect_status
                | "rule" Ident rule_status | "diagnostic" Code
                | "no" (proj_query | EffectName | "duplicate" proj_query)
rule_status    := "fired" [ Number "times" ] | "did" "not" "fire"
effect_status  := "requested" | "completed" | "failed"
```

The predicate sub-language used by `proj_query` (`where …`, `count … is`,
`exists`) is a small, total, side-effect-free filter over projection rows — the
same expression kernel used by guards, restricted to projection fields:

```text
proj_query      := projection_noun [ "exists" | "count" predicate "is" Number
                                   | "where" predicate ]
projection_noun := DottedName   ; a fact name — a user fact like `Issue` or a
                                 ; runtime fact like `agent.turn.completed`
predicate   := comparison { ("and" | "or") comparison }
comparison  := FieldPath ( "==" | "!=" | "<" | "<=" | ">" | ">=" ) Literal
             | FieldPath "contains" StringLiteral
             | FieldPath "in" "[" Literal { "," Literal } "]"
```

Operators, field paths, and literals reuse the
[`expression-kernel`](expression-kernel.md); no new operators are introduced by
tests. Queries are evaluated against typed projection rows, never raw events.



Users should not manually retest the platform invariants covered by Maude,
TLA+, trace conformance, and static analysis.

The platform owns:

- false/error guards do not partially commit work
- assertions fail without mutating workflow state
- effects do not run inline
- dependency edges block downstream effects until satisfied
- provider runs require claimable effects
- leases prevent duplicate provider execution
- stale completions do not become authoritative
- capability denial prevents provider execution
- packages cannot smuggle hidden lifecycle or authority behavior
- accepted construct graphs lower into ordinary core IR
- event log and projections remain replayable

User scenarios should focus on intent:

- the workflow reacts to the right signal
- rules are neither too broad nor too narrow
- failures reach the right fallback path
- duplicate inputs do not create duplicate user-visible work
- schedules use the intended local-time policy
- memory/files/messages/tracker operations happen with the intended scope
- no forbidden effect or capability is requested

## Built-In Risk Utilities

Risk utilities are named modifiers and canned stub outcomes inside `given` and
`stub`. They are not a separate test system.

### Input Risks

```whip
given signal github.workflow_failed { ... } duplicated
given signal github.workflow_failed { ... } malformed
given signal github.workflow_failed { ... } unauthorized
given signal github.workflow_failed { ... } out_of_order

given clock daily_triage missed 3 times
```

Mappings:

```text
duplicated     same logical signal id delivered more than once
malformed      payload fails package/source validation
unauthorized   source provider rejects authentication or policy
out_of_order   ordered source receives a later item before an earlier item
missed         recurring source observes missed occurrences under its policy
```

### Boundary Risks

```whip
stub agent triager succeeds { ... }
stub agent triager fails "model refused"
stub agent triager times_out
stub agent triager cancelled
stub agent triager returns_invalid_output
stub agent triager feature_unavailable "/goal"

stub coerce ClassifyIssue returns { ... }
stub coerce ClassifyIssue invalid_output
stub coerce ClassifyIssue fails "schema backend unavailable"
```

### Resource Risks

```whip
stub tracker backlog claim_lost
stub tracker backlog conflict_on_update

stub memory project_memory returns_empty
stub memory project_memory index_unavailable
stub memory project_memory learn_rejected

stub file project_files missing "docs/template.md"
stub file project_files permission_denied "docs/private.md"
stub file project_files invalid_format "data/issues.csv"

stub message ops delivery_failed "channel unavailable"
stub message ops unsupported_feature "threaded_reply"
```

### Authority Risks

```whip
stub capability script.run denied
stub script deploy_release disabled
stub provider codex feature_unavailable "/goal"
```

These utilities should produce ordinary diagnostics, provider outcomes, and
fixture events. The user sees domain names; the runtime sees normal deterministic
fixture mechanics.

## Package Risk Utility Contract

The platform owns the common risk vocabulary. Packages map their surfaces into
that vocabulary.

A package must declare fixture outcomes for every executable source surface that
crosses a boundary. Metadata-only constructs do not need risk utilities.

Required outcomes are determined by surface class:

| Surface class | Required fixture outcomes |
| --- | --- |
| `effect_operation` | `succeeds`, `fails`, `times_out`; also `returns_invalid_output` when output is structurally consumed |
| `agent_turn` | `succeeds`, `fails`, `times_out`, `cancelled`, `returns_invalid_output`, `feature_unavailable` |
| `schema_coercion` | `returns`, `invalid_output`, `fails`, `times_out` |
| `claim_operation` | `acquired`, `unavailable`, `lost`, `expired`, `conflict` |
| `signal_source` | `valid`, `duplicated`, `malformed`, `unauthorized`; also `out_of_order` when ordering is meaningful |
| `clock_source` | `on_time`, `missed`, `duplicated` |
| `resource_read` | `exists`, `missing`, `permission_denied`; also `invalid_format` when decoding is involved |
| `resource_write` | `written`, `permission_denied`, `conflict`, `invalid_format` when encoding is involved |
| `message_send` | `sent`, `delivery_failed`, `unsupported_feature` |
| `memory_recall` | `returns_results`, `returns_empty`, `fails`, `times_out`, `index_unavailable` |
| `memory_learn` | `learned`, `learn_rejected`, `index_unavailable`, `fails`, `times_out` |
| `script_run` | `succeeds`, `fails`, `denied`, `disabled`, `times_out` |

Package contracts should declare:

```text
surface id
surface class
required fixture outcomes implemented
domain aliases, if any
deterministic fixture response shape for each outcome
diagnostic code for failure/denial outcomes
projection changes produced by each outcome
whether the outcome is terminal, retryable, or branchable
```

Conceptual manifest fragment:

```json
{
  "fixture_outcomes": [
    {
      "surface": "memory.recall",
      "class": "memory_recall",
      "outcomes": {
        "returns_results": {"terminal": "succeeded"},
        "returns_empty": {"terminal": "succeeded"},
        "index_unavailable": {
          "terminal": "failed",
          "diagnostic": "memory.index_unavailable"
        },
        "fails": {"terminal": "failed"},
        "times_out": {"terminal": "timed_out"}
      }
    }
  ]
}
```

`whip package check` should validate:

- every runtime-facing package surface declares a surface class
- every required outcome for that class is present
- outcome names use the platform vocabulary or declared domain aliases
- each outcome maps to deterministic fixture behavior
- output payloads validate against declared output schemas
- failure outcomes declare stable diagnostic codes
- branchable outcomes expose the branch/projection shape tests can assert
- standard packages ship at least one conformance fixture per required outcome

Missing required fixture outcomes are package-contract errors for standard
packages and should become package-contract errors for third-party packages once
the fixture contract is no longer experimental.

Packages may add extra domain-specific outcomes, but the standard outcomes must
remain available so user tests do not fragment into package-specific vocabulary.

## Required Fixture Families

The workflow test implementation should add positive fixtures for:

- a minimal `test` scenario using `given`, `stub`, `run`, and `expect`
- duplicate signal input that does not create duplicate user-visible work
- provider failure reaching an explicit fallback path
- authority denial producing a diagnostic and no provider run
- package projection expectation such as issue/message/file/memory state
- at least one standard fixture outcome for every standard package surface

It should add negative fixtures for:

- unknown fixture outcome
- fixture outcome unsupported by the surface class
- missing required fixture outcome in a standard package contract
- package-specific outcome alias without a platform mapping
- `test` declaration leaking into compiled runtime IR
- `stub` targeting a metadata-only construct
- `given signal ... out_of_order` on an unordered source class
- `expect` over an ambiguous package projection
- fixture payload that fails the package's declared output schema

## Harness Mechanics

`whip test` compiles each scenario into a driver over the ordinary kernel — it
adds no second runtime. The mechanics are:

- **Isolation**: each scenario runs in a fresh in-memory runtime store (an
  isolated SQLite store); `--parallel <n>` runs scenarios in separate stores with
  no shared catalog mutation.
- **`given` seeding**: `given` does not write facts directly. Each `given`
  lowers to the same admission path its surface uses at runtime — `given signal`
  appends through typed signal admission, `given fact` seeds an initial fact
  projection through the kernel's projection seed, `given clock at` sets the
  deterministic clock source, `given file`/`given tracker` seed the package's
  fixture projection. Seeding that would violate a package's policy or schema is
  a scenario `invalid`, not a silent pass.
- **`stub` injection**: a `stub` registers a deterministic outcome on the local
  **fixture provider** for that surface; when the kernel claims an effect for that
  surface, the fixture provider returns the stubbed terminal outcome (success
  payload, failure, timeout, `claim_lost`, etc.). Stubs never bypass capability
  authorization — a denied capability still produces `capability.not_granted`.
- **Determinism / replay equality**: with the clock fixed by `given clock` and
  all boundaries stubbed, a scenario is deterministic. Equality (for `run`
  re-execution and `whip test replay`) is defined as **equal canonical
  projections**: serialize the named projections and terminal state in the
  canonical form (sorted keys, stable field order) and compare bytes. Raw event
  ids, timestamps, and effect ids are excluded from the comparison.
- **Iteration cap**: the runner enforces a fixed iteration cap; reaching it
  before the `run` stop condition is a scenario failure with an actionable
  diagnostic, not a hang. This cap is the scenario `run` cap and is distinct from
  the acceptance-fixture `max_iterations` (see
  [`acceptance-fixtures.md`](acceptance-fixtures.md)).

## CLI

Target command:

```sh
whip test [<source-or-dir>...] [--json] [--list] [-i <pattern>] [-x <pattern>] [--parallel <n>] [--dotenv-path <path>] [--pass-if-no-tests]
```

Default behavior:

- discover `.whip` source and test declarations
- discover `whip.lock` like `check`
- validate package fixture outcome contracts before running tests
- run deterministic tests only
- use local fixture providers, not real providers
- isolate each scenario in a fresh runtime store
- default to `run until idle` if no `run` appears
- report failures through shared diagnostics

Pattern ids should be simple:

```text
WorkflowName::test name
::test name
WorkflowName::
```

`*` matches within those names. Exclude patterns override include patterns.

Exit codes:

```text
0  selected tests passed
1  test failures occurred
2  test source, package, or fixture setup is invalid
3  test execution was cancelled
4  no tests were selected, unless --pass-if-no-tests
```

## Reports

`whip test --json` should emit `whipplescript.test_report.v0`.

Conceptual shape:

```json
{
  "schema": "whipplescript.test_report.v0",
  "status": "passed",
  "summary": {
    "selected": 1,
    "passed": 1,
    "failed": 0,
    "invalid": 0,
    "skipped": 0
  },
  "scenarios": [
    {
      "id": "TriageFailedRun::failed CI gets triaged",
      "status": "passed",
      "workflow": "TriageFailedRun",
      "source_span": null,
      "steps": [],
      "expectations": [],
      "diagnostics": [],
      "dev_report_ref": null
    }
  ]
}
```

The report should not expose raw event ids as the main assertion interface.
Detailed traces and dev reports may be linked for debugging.

## Replay

Replay is useful, but it should stay outside the v0 core scenario syntax.

Target command:

```sh
whip test replay <trace-or-event-log> --workflow <WorkflowName> [--json]
```

Replay compares projections and expected observable behavior, not fragile raw
event ids. It is for regression debugging after real runs, not the everyday
authoring path.

> **Implementation status.** `whip test replay <instance-id>` is implemented over a
> `--store` instance: it replays that instance's recorded event log into a throwaway
> copy of the store and verifies the reconstructed canonical projection (terminal
> status + active facts/effects, volatile ids/timestamps/epochs excluded, arrays
> sorted) is byte-identical to the live-built one. Exit 0 equal · 1 diverged · 2
> setup error. The standalone trace/event-log *file* + `--workflow` form above is the
> eventual target; a portable file-based trace format is a future extension.

## Real Providers And Evals

Real-provider tests and evals are deferred from the default user-facing test
surface.

Later commands may add:

```sh
whip test --real-providers
whip eval
```

Those modes must be opt-in, redacted, and reported separately from deterministic
workflow correctness. Model-judged quality must not be the only evidence that a
workflow's orchestration behavior is correct.

## Editor Integration

The LSP should expose:

- test discovery
- run current test
- run all tests for workflow
- test diagnostics
- expectation failures with source spans
- fixture outcome completions from package contracts

Editor support should call the same `whip test` services. It must not implement
a separate test runner.

## Acceptance Criteria

User-facing workflow testing is implemented when:

- top-level `test` declarations parse and validate as non-runtime source
  artifacts
- `given`, `stub`, `run`, and `expect` cover deterministic scenario tests
- tests can seed facts, inputs, signals, clock time, and package resource state
- tests can stub package/provider boundary outcomes through the standard risk
  vocabulary
- package contracts declare fixture outcomes by surface class
- `whip package check` validates fixture outcome coverage for standard packages
- `whip test` discovers, filters, lists, runs, and reports deterministic
  scenario tests
- each scenario runs in an isolated store with fixture providers
- expectations assert projections/effects/diagnostics without raw event-id
  coupling
- real-provider and eval modes are not part of the default deterministic test
  path
