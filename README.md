# armature

Armature is being rebuilt as a restricted statechart workflow runtime for
orchestrating coding agents.

The current product surface is native `.armature` workflow files, validated
workflow IR, durable event queues, append-only transition/effect logs, trusted
Rust adapters, and an initial `validate` / `emit` / `run` / `status` /
`overview` / `events` / `log` / `build` / `check` / `emit-model` /
`emit-config` / `prove` / `validate-adapter` / `validate-policy` CLI.
Workflow files do not execute arbitrary TypeScript, shell, or host-language
code.

## Active Workspace

New implementation work lives in these crates:

```text
crates/armature-workflow   native DSL parser, IR, schemas, diagnostics, validation
crates/armature-engine     durable queue/log/state and interpreter skeleton
crates/armature-adapters   trusted adapter manifests and dispatchers for BAML, humans, agents, legacy bridges
crates/armature-modelgen   TLA+/Apalache/Maude/Veil model generation
crates/armature-cli        small workflow CLI for validate/emit/run/status/overview/events/log/build/check/model iteration
```

Specs live in [`spec/statechart-workflows`](spec/statechart-workflows).
Examples live in [`examples/workflows`](examples/workflows).
Fake adapter manifests live in [`examples/adapters`](examples/adapters).
Example capability policies live in [`examples/policies`](examples/policies).
Formal-model work starts in [`models/statechart-workflows`](models/statechart-workflows).
The companion coding-agent skill lives in
[`skills/armature-statechart`](skills/armature-statechart).

## Legacy Runtime

The previous v0.3 task/service/script runner has been moved to
[`legacy/v0.3-runtime`](legacy/v0.3-runtime). It remains useful reference
material for CLI patterns, process/log capture, packaging, tests, and migration
ideas, but it is not the foundation for the new workflow product.

Compatibility with the old runtime should happen only through explicit adapters
or migration tooling.

## Development

Run the active Rust workspace checks with:

```sh
cargo fmt --all --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

The parser currently accepts the minimal native DSL fixture and the static
surface of the larger spec implementation fixture: top-level `agent`,
`capability`, `enum`, `class`, `coerce` declarations, nested states, guards,
entry and `always` blocks, `case` arm bodies, `let`, object/list/call
expressions, and basic effect statements.

```text
examples/workflows/minimal.armature
examples/workflows/simple-supervisor.armature
```

The larger spec implementation example is still ahead of full runtime
execution. It parses and validates for static declarations and structured
`case` branches, but real capability adapters and real BAML dispatch are not
executed by the runtime yet. The current runtime
evaluates the supported guard-expression subset, executes supported `case`
branches, executes `always` transitions with loop protection, and supports
hierarchical initial-state descent plus parent event fallback. It also persists
fake effect dispatch records for `start`, `send`, `askHuman`, `raise`, and
capability-call steps, including evaluated payload arguments for the supported
expression subset. `raise` enqueues a durable workflow event after the current
transition commits. Transition-local `let` bindings are supported for later
expressions in the same transition or entry execution. Deterministic built-ins
currently include list `append`, `now`, and `elapsedSince`. Expression
invariants in IR are checked after processed transitions, and invariant
failures roll the interpreter back before state is saved. Tests and adapter
scaffolding can inject fake read-call and `coerce` outputs; real capability and
BAML dispatch are still future adapter steps. The CLI `run` command can also
load JSON adapter manifests with `--adapter-manifest`; when present,
adapter manifests are validated, adapter-backed effects are checked against the
manifest, and failures are logged durably. Manifest `required_capabilities` are
preserved on adapter outcomes, including runtime policy denials, and surfaced
through status/log JSON.
The runtime enforces declared agent `maxActive` limits before dispatching
`start` effects; over-capacity starts are recorded as durable failed effects.
Static validation requires bounded starts to have a processable `finished`
event with a required `name` string for active-invocation retirement. Processed
completion names are matched against the longest started-agent prefix.
The `validate` command accepts the same manifest flag for static
adapter-backed effect checks. Commands that validate or dispatch
adapter-backed effects also accept `--policy <json>` capability documents.
Policy documents currently support exact `allowed_capabilities`,
`denied_capabilities`, and `local` / `team` / `enterprise` modes. Unknown
capabilities warn in local mode and become errors under stricter modes
according to effect category and write-like capability names. The manifest
dispatcher enforces the same supplied policy at runtime before dispatching each
adapter-backed effect. `emit` and `run --event` can also use manifest event
schemas for adapter-originated events that are not declared directly in the
workflow. Runtime string interpolation supports path
expressions such as `{{ classification.reason }}`. Static validation checks
nested expression calls and dotted paths against declared data fields, event
bindings, `let` locals, declared coerce functions, declared capabilities,
declared raised events, and supported built-ins. Status JSON includes the
current state, pending event count, queued event summaries, the recent
transition, recent effect summaries, recent failures, and a first
active-invocation projection derived from durable `start` effects and processed
`finished` events, including declared `max_active` limits when present in IR.
Validation failures are reported through CLI diagnostics outside `validate` too,
so commands such as `build`, `check`, and `emit-model` do not collapse workflow
schema errors to a generic failure. Parser diagnostics include source locations
for common current-token errors, and CLI-loaded workflows report the actual file
path. Validator diagnostics use declaration and step spans for common static
errors such as invalid `maxActive`, undeclared agents, undeclared capabilities,
bad transitions, bad raises, bad assignments, and invalid expression paths or
calls. Adapter-manifest workflow diagnostics also point at the effect step or
handler that requested unsupported adapter authority. CLI-loaded workflows also
preserve `workflow.source_path` in emitted IR and build artifacts.
`overview` renders validation health plus the same runtime projection as a
compact human-readable summary with current state, pending/queued events, active
invocations, latest transition, latest effects, required capabilities, effect
errors, and recent failures. Its JSON shape is `{ validation, status }`; invalid
source that cannot lower to IR still returns validation diagnostics with
`status: null`.
`emit-model` can currently emit small TLA+ and Maude state-transition
overapproximations from validated IR. Both generated backends include bounded
active invocation counters and max-active safety checks. The generated TLA+
model also abstracts `coerce` calls as nondeterministic choices over finite
declared output spaces, with a `CoerceType` invariant that keeps each function's
stored value inside its declared schema abstraction. It also tracks the last
abstract effect label with a `DeclaredEffectType` invariant; ordinary effects
such as `send`, `askHuman`, `raise`, and capability calls are represented as
stuttering observations, while bounded `start` effects update active counters.
Generated TLA+ and Maude artifacts also annotate declared built-in invariants
with their current coverage layer.
Because generated models do not yet include workflow data, `emit-model` and
`check` reject IR with expression invariants instead of silently omitting them.
`check` can run either TLC or Maude when the tools are installed directly or
available through the repository Nix flake. `check` and `emit-model` accept
`--adapter-manifest` and `--policy`, and validate adapter-backed workflow
effects before emitting or checking the formal abstraction.
`build` writes `workflow-ir.json`, generated BAML source in
`baml_src/workflow.baml`, plus generated TLA, TLA check config, and Maude
models. If
`--adapter-manifest` or `--policy` is supplied, `build` validates the workflow
against those contracts and writes `adapter-manifests.json` and
`policy-documents.json` bundles beside the other artifacts.
`skills/armature-statechart` contains the companion skill for coding agents. It
documents the restricted workflow boundary, the statechart authoring pattern,
coerce usage, adapter manifests, debug commands, and common repairs.
Schema validation follows BAML-style optional fields: a `?` field may be absent
or `null`, while non-optional fields must be present.

Current CLI smoke commands:

```sh
cargo run -p armature-cli -- validate examples/workflows/minimal.armature --json
cargo run -p armature-cli -- validate examples/workflows/minimal.armature --adapter-manifest path/to/adapter.json --json
cargo run -p armature-cli -- validate examples/workflows/spec-implementation.armature --adapter-manifest examples/adapters/spec-implementation.fake-adapter.json --json
cargo run -p armature-cli -- validate examples/workflows/spec-implementation.armature --adapter-manifest examples/adapters/spec-implementation.fake-adapter.json --policy examples/policies/spec-implementation.enterprise-policy.json --json
cargo run -p armature-cli -- validate-adapter examples/adapters/spec-implementation.fake-adapter.json --json
cargo run -p armature-cli -- validate-policy examples/policies/spec-implementation.enterprise-policy.json --json
cargo run -p armature-cli -- emit examples/workflows/minimal.armature --event start --payload '{"message":"hello"}' --json
cargo run -p armature-cli -- emit examples/workflows/minimal.armature --event finished --payload '{"name":"worker-1"}' --adapter-manifest path/to/adapter.json --json
cargo run -p armature-cli -- run examples/workflows/minimal.armature --event start --payload '{"message":"hello"}' --json
cargo run -p armature-cli -- run examples/workflows/minimal.armature --adapter-manifest path/to/adapter.json --json
cargo run -p armature-cli -- run examples/workflows/minimal.armature --json
cargo run -p armature-cli -- status examples/workflows/minimal.armature --json
cargo run -p armature-cli -- overview examples/workflows/minimal.armature
cargo run -p armature-cli -- events examples/workflows/minimal.armature --json
cargo run -p armature-cli -- log examples/workflows/minimal.armature --json
cargo run -p armature-cli -- build examples/workflows/minimal.armature --json
cargo run -p armature-cli -- build examples/workflows/spec-implementation.armature --adapter-manifest examples/adapters/spec-implementation.fake-adapter.json --policy examples/policies/spec-implementation.enterprise-policy.json --json
cargo run -p armature-cli -- check examples/workflows/minimal.armature --target tla --json
cargo run -p armature-cli -- check examples/workflows/minimal.armature --target maude --json
cargo run -p armature-cli -- check examples/workflows/spec-implementation.armature --adapter-manifest examples/adapters/spec-implementation.fake-adapter.json --policy examples/policies/spec-implementation.enterprise-policy.json --target tla --json
cargo run -p armature-cli -- emit-model examples/workflows/minimal.armature --target tla
cargo run -p armature-cli -- emit-config examples/workflows/minimal.armature --target tla
cargo run -p armature-cli -- emit-model examples/workflows/minimal.armature --target maude
cargo run -p armature-cli -- emit-model examples/workflows/spec-implementation.armature --adapter-manifest examples/adapters/spec-implementation.fake-adapter.json --policy examples/policies/spec-implementation.enterprise-policy.json --target maude
```

Maude checks are embedded in the generated `.maude` file, so `emit-config` is
only meaningful for TLA in the current implementation.
`prove` is reserved for stronger proof-oriented backends and currently returns a
clear not-implemented diagnostic after validating the workflow and supplied
contracts. Use `--json` for a structured unavailable response.
