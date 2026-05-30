# Statechart Workflow Formal Models

This directory is for hand-written and generated models of the workflow
semantics.

The first model should be hand-written and small. Its job is to pressure-test
the language and runtime semantics before implementation hardens.

Initial model files:

```text
SpecImplementation.tla
SpecImplementation.cfg
SpecImplementation.maude
```

Maude was added at the Phase 1 reevaluation checkpoint because the first TLA
model intentionally abstracts away executable small-step behavior. The Maude
model currently checks the same bounded workflow safety envelope as an
executable rewrite system. It should grow toward handler lookup, event ordering,
raised events, and durable effect commit semantics as those details harden.

The model should include bounded work items, workflow states, `finished` events,
`idle` observation events, nondeterministic coerce outputs, active invocation
counters, visible work item statuses, capability facts, and human-review
visibility.

Phase 0 remodeling checkpoint for the Option A/BAML HTTP direction:

- no semantic remodel is needed before implementing the `CoerceExecutor`
  boundary
- real `coerce` execution through BAML HTTP remains outside the formal model
- durable `coerce_calls` idempotency and replay are runtime/storage invariants,
  not statechart safety choices
- the useful formal obligation remains: for any schema-valid coerce output, the
  workflow preserves its control-state, active-invocation, visibility, and
  declared-effect invariants
- the existing hand-written TLA+ and Maude models continue to represent coerce
  as nondeterministic choices folded into workflow transitions

Invariant coverage assignment for the next implementation slices:

```text
max active workers/quality          TLA, Maude, runtime enforcement, tests
visible started/failed work         TLA, Maude, status e2e
declared effect surface             TLA, generated model, static validation
coerce output schema validity       static validation, runtime enforcement, tests
coerce idempotent replay            SQLite/runtime tests, e2e recovery tests
BAML HTTP transport failures        runtime tests, opt-in integration tests
expression primitive type safety    static validation, runtime tests
workflow data expression invariants runtime enforcement now, generated model later
adapter capability authority        static validation, runtime policy, adapter tests
```

The hand-written model should be written against the native `.whip` DSL and
WorkflowIR semantics. Generated models should consume validated WorkflowIR, not
raw source text.

The current `SpecImplementation.tla` file is a hand-written Phase 1 model.
`tlaplus`, Java, and Maude are available through the repository Nix flake:

```sh
nix develop -c tlc -deadlock -config models/statechart-workflows/SpecImplementation.cfg models/statechart-workflows/SpecImplementation.tla
nix develop -c maude models/statechart-workflows/SpecImplementation.maude
nix develop -c maude --version
scripts/check-formal-models.sh
```

`scripts/check-formal-models.sh` checks both tracks: the source-controlled
hand-written TLA+/Maude models in this directory and the generated TLA+/Maude
models produced from `examples/workflows/spec-implementation.whip` through
`whip check`.

TLC result recorded during the initial implementation pass:

```text
137 states generated
88 distinct states found
complete state graph depth 13
no errors found
```

Maude result recorded during the initial implementation pass:

```text
search found no invariant-violating solution
106 states explored
5783 rewrites
```

Apalache is not currently pinned. Phase 1 should either add an Apalache
installation path or continue with TLC plus Maude first.

Generated models should eventually live under `.whipplescript/build/models/` for a
specific workflow build. This directory is for source-controlled design models
and fixtures.
