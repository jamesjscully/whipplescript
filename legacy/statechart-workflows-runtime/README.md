# armature

Armature is being rebuilt as a restricted statechart workflow runtime for
orchestrating coding agents.

The current product surface is native `.armature` workflow files, validated
workflow IR, durable event queues, append-only transition/effect logs, trusted
Rust adapters, and an initial `init` / `validate` / `emit` / `run` / `status` /
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
Operational repair guidance lives in
[`spec/statechart-workflows/operations.md`](spec/statechart-workflows/operations.md);
legacy migration notes live in
[`spec/statechart-workflows/migration.md`](spec/statechart-workflows/migration.md).

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
cargo build -p armature-cli
scripts/check-docs.sh
scripts/check-e2e.sh
scripts/check-formal-models.sh
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

The larger spec implementation example runs end-to-end through the CLI when
deterministic test doubles are supplied for BAML/coerce decisions and
capability reads. Fake output names must be unique; duplicate
`--fake-coerce-output` or `--fake-call-output` names are rejected instead of
silently overriding earlier values, and names may not contain whitespace or
control characters. `run --plan-file <json>` provides the first real scoped
plan/state adapter path: `plan.snapshot()` reads the file as the plan snapshot,
`plan.unfinishedItems()` counts non-done items, `plan.nextReadyItem()` returns
the first ready task object, and plan write effects such as
`plan.markReadyForQuality`, `plan.markDone`, and `plan.markBlocked` update task
status in that JSON file. For plan-only
workflows, `--plan-file` also supplies the built-in JSON plan manifest; larger
workflows can still pass explicit manifests for production plan adapters.
`run --review-file <json>` provides the first human-review bridge:
`askHuman(...)` appends an open review obligation to the JSON file and supplies
the built-in human-review manifest when needed. `emit --review-file <json>`
also supplies the built-in `humanReview.responded` event schema for typed
review responses with `{reviewId, decision, response?}` payloads.
Local agents are native runtime resources: `start` writes durable queued
invocations to SQLite, `send` writes durable messages, and
`armature harness once|run|status` claims invocations, runs configured
providers, records completions, and enqueues typed `finished` workflow events.
The harness supports the generic `command` provider, thin `codex`/`claude`/`pi`
command presets, `timeoutSeconds`, command placeholders such as `{{prompt}}`,
and `harness run --drive-workflow` for a single supervisor loop that processes
provider completions back through the workflow. The next governed execution
surface is harness profile policy: workflows request semantic profiles such as
`research`, `repo-reader`, `repo-writer`, or `human-review`, and policy maps
those profiles to concrete providers, filesystem/network posture, environment
allowlists, timeout, and enforcement mode. The e2e suite covers the
runtime boundary using fake manifests, explicit fake outputs, the JSON plan
file adapter, the JSON review file bridge, and the native command harness. The
current runtime evaluates the
supported expression kernel, executes supported `case` branches, executes
`always` transitions with loop protection, and supports
hierarchical initial-state descent plus parent event fallback. It also persists
adapter effect dispatch records for `start`, `send`, `askHuman`, `raise`, and
capability-call steps, including evaluated payload arguments for the supported
expression subset. Expression-style capability value calls such as
`plan.snapshot()` dispatch through the same adapter boundary. `raise` enqueues
a durable workflow event after the current transition commits. Transition-local
`let` bindings are supported for later expressions in the same transition or
entry execution. Deterministic built-ins cover the v1 list, map, text, and time
helpers documented in `spec/statechart-workflows/expression-primitives.md`.
Expression invariants in IR are checked after
processed transitions, and invariant failures roll the interpreter back before
state is saved. Tests and adapter scaffolding can still inject fake read-call
and `coerce` outputs. The CLI `run` command can also
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
The `validate`, `build`, `check`, `prove`, `emit-model`, and `emit-config`
commands accept the same manifest flag and file-backed adapter flags for static
adapter-backed effect checks. Commands that validate or dispatch adapter-backed
effects also accept `--policy <json>` capability documents.
Policy documents currently support exact `allowed_capabilities`,
`denied_capabilities`, `local` / `team` / `enterprise` modes, and initial BAML
HTTP execution controls. `run --baml-url` checks `baml.coerce`,
`allow_baml_network`, and exact `allowed_baml_urls` entries before any network
call. `store_baml_raw_responses` controls whether BAML HTTP raw responses are
stored or replaced with a redaction marker; enterprise mode redacts by default
while parsed output remains durable. Unknown capabilities warn in local mode and
become errors under stricter modes according to effect category and write-like
capability names. The manifest dispatcher enforces the same supplied policy at
runtime before dispatching each adapter-backed effect. `emit` and `run --event`
can also use manifest event schemas for adapter-originated events that are not
declared directly in the workflow. `emit --policy` validates policy document
shape but does not require event-only manifests to declare workflow effects.
Runtime string interpolation supports path
expressions such as `{{ classification.reason }}`. Static validation checks
nested expression calls and dotted paths against declared data fields, event
bindings, `let` locals, declared coerce functions, declared capabilities,
declared raised events, and supported built-ins. Status JSON includes the
current state, pending event count, queued event summaries, the recent
transition, recent effect summaries, current effect failures, current blockers,
historical recent failures, the current coerce failure only while its event is
still unresolved, historical latest coerce failures, and a first
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
Policy-document validation failures are reported separately from adapter
manifest failures, including duplicate, empty, or whitespace-bearing capability
and policy-token entries.
`overview` renders validation health plus the same runtime projection as a
compact human-readable summary with current state, pending/queued events, active
invocations, latest transition, latest effects, effect idempotency keys,
required capabilities, effect errors, data summaries, policy blockers, current
effect failures, current blockers, historical recent failures, latest coerce
calls, the current coerce failure only while its event is unresolved, and
historical latest coerce failures. Its JSON shape is `{ validation, status }`;
status JSON exposes `current_effect_failures`, `current_coerce_failure`, and
`current_blockers` directly.
invalid source that cannot lower to IR still returns validation diagnostics
with `status: null`.
`status` also accepts `--adapter-manifest` and `--policy` so operators can
project durable state under the same contracts used for validation, build, and
run; status projection still performs no live adapter calls. `status --compact`
prints a short operator view with workflow, state, waiting reason, pending
events, active invocations, current blockers, and latest transition.
`events` and `log` accept the same adapter, policy, and file-backed shortcut
flags as validation-only context before reading durable records. Their
`--limit` values are capped at 10,000 records to keep inspection commands
bounded.
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
The rejection explains the unsupported surface, such as workflow data not yet
being included in generated models.
`check` can run either TLC or Maude when the tools are installed directly or
available through the repository Nix flake. `check` and `emit-model` accept
`--adapter-manifest` and `--policy`, and validate adapter-backed workflow
effects before emitting or checking the formal abstraction.
`build` writes `workflow-ir.json`, generated BAML source in
`baml_src/workflow.baml`, plus generated TLA, TLA check config, and Maude
models, and `artifact-hashes.json` with SHA-256 hashes for reproducibility. If
`--adapter-manifest` or `--policy` is supplied, `build` validates the workflow
against those contracts and writes `adapter-manifests.json` and
`policy-documents.json` bundles beside the other artifacts.
`skills/armature-statechart` contains the companion skill for coding agents. It
documents the restricted workflow boundary, the statechart authoring pattern,
coerce usage, adapter manifests, debug commands, and common repairs.
Schema validation follows BAML-style optional fields: a `?` field may be absent
or `null`, while non-optional fields must be present.

Useful CLI examples:

```sh
cargo run -p armature-cli -- validate examples/workflows/minimal.armature --json
cargo run -p armature-cli -- init target/tmp/armature-demo --name DemoWorkflow --json
cargo run -p armature-cli -- validate examples/workflows/spec-implementation.armature --adapter-manifest examples/adapters/spec-implementation.fake-adapter.json --json
cargo run -p armature-cli -- validate examples/workflows/spec-implementation.armature --adapter-manifest examples/adapters/spec-implementation.fake-adapter.json --policy examples/policies/spec-implementation.enterprise-policy.json --json
cargo run -p armature-cli -- validate-adapter examples/adapters/spec-implementation.fake-adapter.json --json
cargo run -p armature-cli -- validate-policy examples/policies/spec-implementation.enterprise-policy.json --json
cargo run -p armature-cli -- emit examples/workflows/minimal.armature --event start --payload '{"message":"hello"}' --json
cargo run -p armature-cli -- run examples/workflows/minimal.armature --event start --payload '{"message":"hello"}' --json
cargo run -p armature-cli -- run examples/workflows/minimal.armature --json
cargo run -p armature-cli -- run examples/workflows/spec-implementation.armature --adapter-manifest examples/adapters/spec-implementation.fake-adapter.json --policy examples/policies/spec-implementation.enterprise-policy.json --event idle --payload '{"activeRuns":0,"unfinishedItems":1}' --fake-call-output 'plan.snapshot="W1 ready"' --fake-coerce-output 'chooseNextStep={"action":"StartWorker","workItemId":"W1","reason":"ready","message":"Implement W1"}' --json
cargo run -p armature-cli -- status examples/workflows/minimal.armature --json
cargo run -p armature-cli -- overview examples/workflows/minimal.armature
cargo run -p armature-cli -- harness status examples/workflows/minimal.armature --json
cargo run -p armature-cli -- harness once examples/workflows/minimal.armature --config harness.json --json
cargo run -p armature-cli -- harness run examples/workflows/minimal.armature --config harness.json --drive-workflow --max-iterations 10 --json
cargo run -p armature-cli -- events examples/workflows/minimal.armature --json
cargo run -p armature-cli -- events examples/workflows/minimal.armature --status failed --json
cargo run -p armature-cli -- events examples/workflows/minimal.armature --status dead_lettered --json
cargo run -p armature-cli -- retry-event examples/workflows/minimal.armature --event-id evt_cli_... --json
cargo run -p armature-cli -- log examples/workflows/minimal.armature --json
cargo run -p armature-cli -- build examples/workflows/minimal.armature --json
cargo run -p armature-cli -- build examples/workflows/spec-implementation.armature --adapter-manifest examples/adapters/spec-implementation.fake-adapter.json --policy examples/policies/spec-implementation.enterprise-policy.json --json
cargo run -p armature-cli -- check examples/workflows/minimal.armature --target tla --json
cargo run -p armature-cli -- check examples/workflows/minimal.armature --target maude --json
cargo run -p armature-cli -- check examples/workflows/spec-implementation.armature --adapter-manifest examples/adapters/spec-implementation.fake-adapter.json --policy examples/policies/spec-implementation.enterprise-policy.json --target tla --json
cargo run -p armature-cli -- prove examples/workflows/minimal.armature --json
cargo run -p armature-cli -- emit-model examples/workflows/minimal.armature --target tla
cargo run -p armature-cli -- emit-config examples/workflows/minimal.armature --target tla
cargo run -p armature-cli -- emit-model examples/workflows/minimal.armature --target maude
cargo run -p armature-cli -- emit-model examples/workflows/spec-implementation.armature --adapter-manifest examples/adapters/spec-implementation.fake-adapter.json --policy examples/policies/spec-implementation.enterprise-policy.json --target maude
```

The repository smoke suite runs the maintained command set through
`scripts/check-docs.sh`, `scripts/check-e2e.sh`, and
`scripts/check-formal-models.sh`. Commands with placeholder ids, such as
`retry-event --event-id evt_cli_...`, are illustrative and need a real event id
from `armature events --json`.

Maude checks are embedded in the generated `.maude` file, so `emit-config` is
only meaningful for TLA in the current implementation.
`prove` validates the workflow and supplied contracts, then runs the current
generated verification bundle: TLA+ and Maude.
