# External System Validation

Status: working notes

This document records external assumptions that have been checked against
documentation and, where possible, tested directly.

## HJSON Historical Probe

HJSON was validated when it was the candidate source format. It is no longer the
selected primary `.armature` authoring surface, but these notes remain useful if
HJSON is later used for policy files, adapter manifests, fixtures, or migration
tools.

Validated against:

- HJSON syntax documentation
- HJSON RFC documentation
- official `hjson` JavaScript CLI
- Rust `deser-hjson` crate

Confirmed:

- HJSON supports comments.
- HJSON supports omitted commas between newline-separated members/items.
- HJSON supports trailing commas.
- HJSON supports triple-quoted multiline strings.
- HJSON can be converted to ordinary JSON.
- Rust-side parsing is available through `deser-hjson`.

Direct probes:

```text
npx hjson -j examples/workflows/minimal.armature
npx hjson -j examples/workflows/spec-implementation.armature
deser-hjson parse of both example workflow files
```

Correction found:

HJSON quoteless strings are line-oriented. Inline arrays or inline objects with
unquoted string values and commas can parse incorrectly.

Bad:

```hjson
can: [read_repo, run_tests]
{ start: worker, with: { work_item: "${next.work_item_id}" } }
```

Good:

```hjson
can: ["read_repo", "run_tests"]
{ start: "worker", with: { work_item: "${next.work_item_id}" } }
```

If HJSON is used anywhere in the project, examples should prefer quoted string
values inside inline arrays and inline objects.

## BAML

Validated against:

- Boundary documentation for TypeScript setup and `baml-cli generate`
- Boundary language reference for functions, `client<llm>`, enums, and
  `ctx.output_format`
- local generation with `@boundaryml/baml@0.220.0`, matching the un-tie repo
  dependency

Confirmed:

- BAML functions require a `client` declaration.
- BAML clients may be declared with `client<llm>`.
- `prompt #"... "#` with `{{ ctx.output_format }}` is valid.
- BAML enums require values that start with an uppercase letter.
- Generated TypeScript client code is produced by `baml-cli generate`.
- The Rust crate exposes a lower-level runtime surface including in-memory
  source loading, dynamic values, parsing, calls, and type builder APIs.

Direct probe:

```text
construct BAML source equivalent to Armature-shaped enum/class/coerce declarations
write that source into baml_src/
run npm exec --package @boundaryml/baml@0.220.0 -- baml-cli generate
attempt a Rust SDK probe with baml = 0.221.0
```

Corrections found:

- Lowercase enum values such as `worker_complete` are invalid.
- BAML functions without a `client` fail generation.
- The workflow examples now use `WorkerComplete`, `StartWorker`, etc.
- Direct Rust SDK use currently requires `protoc` in this local environment; the
  probe failed before runtime execution because `protobuf-compiler` is not
  installed.

## TLA+ / Apalache

Validated against:

- Apalache project documentation
- Apalache TLA+ language documentation

Confirmed from documentation:

- Apalache is a symbolic model checker for TLA+.
- It supports randomized symbolic execution, bounded model checking, and
  inductiveness checking.
- It is suitable for bounded counterexample-oriented design checks.

Tested locally:

- `tlaplus` and Java were provisioned through the repository Nix flake.
- TLC checked `models/statechart-workflows/SpecImplementation.tla` with
  `SpecImplementation.cfg`.
- The checked model generated 137 states, found 88 distinct states, reached
  complete graph depth 13, and reported no errors.
- Maude 3.5.1 was provisioned through the repository Nix flake.
- Maude checked `models/statechart-workflows/SpecImplementation.maude`; the
  invariant-violation search found no solution after exploring 106 states and
  5783 rewrites.

Not directly tested locally:

- Apalache is not currently pinned in the repository flake.

Required before relying on this path:

- decide whether TLC is sufficient for the first formal checkpoint or add
  Apalache
- refine the Maude executable semantics model as handler lookup, event ordering,
  raised events, and effect commit semantics become concrete
- add the repeatable command to CI when the workflow stabilizes

## Veil

Validated against:

- Veil documentation

Confirmed from documentation:

- Veil is embedded in Lean 4.
- Veil specifies transition systems with mutable state, immutable background
  theory, actions, procedures, safety properties, and invariants.
- Veil supports model checking and invariant checking commands.
- Veil 2.0 is documented as a pre-release with rough edges.

Not directly tested locally:

- Lean and Lake are installed.
- Veil is not installed or pinned as a project dependency.

Required before relying on this path:

- add/pin a Veil dependency through Lake
- create a minimal generated or hand-written Veil module
- run the corresponding Veil checks locally and in CI

## un-tie

Validated against local specs and code in `/home/jack/code/un-tie`.

Confirmed:

- un-tie has `.agent-config.json` policy concepts for file, execute, and egress
  controls.
- un-tie uses BAML 0.220.0 in its package configuration.
- existing generated BAML client code documents `baml-cli generate` as the
  regeneration path.

Not directly tested in this validation pass:

- no Armature workflow adapter was run against a live un-tie session because the
  adapter does not exist yet.
