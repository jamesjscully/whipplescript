# Armature

Armature is being redesigned as a restricted orchestration language for coding
agents.

The new direction is an event-sourced relational rule machine:

- facts and events describe what is true
- rules rewrite those facts into new facts and durable effects
- agent turns are effects, not arbitrary host-language calls
- runtime state is append-only and auditable
- static analysis and formal modeling are first-class design constraints

The previous Rust statechart implementation has been moved to
`legacy/statechart-workflows-runtime/`. Older TypeScript-oriented runtime work
remains in `legacy/v0.3-runtime/`.

Current design work starts in `spec/`.

The target core is intentionally small: rule runtime, control plane, registries,
agent harnesses, skills, BAML coercion, Docket integration, human review, and
evidence/observability. Memory, Thoth, external trackers, browser automation,
research tools, notifications, dashboards, and evaluators should begin as
plugins unless the kernel must understand them.

Formal model scaffolding starts in `models/`. Maude is the first runnable model
target for the rule/effect-graph kernel; TLA+/Apalache is planned for durable
control-plane lifecycle checks; Veil/Lean is deferred until the kernel semantics
stabilize.

## Active Workspace

The active implementation starts at the repository root.

```text
Cargo.toml                 Rust workspace
crates/armature-core       shared types and contracts
crates/armature-parser     `.armature` source parser and typed IR
crates/armature-store      SQLite-backed runtime store
crates/armature-kernel     deterministic rule/effect runtime kernel
crates/armature-cli        control-plane CLI
spec/                      current design specs and implementation tracker
models/                    formal models and checks
scripts/                   root project checks
```

The current Rust crates are Stage 0 scaffolding. They compile and provide a
minimal CLI smoke path, but the parser, store, and runtime kernel are not
implemented yet. The source of truth for planned work is
[`spec/implementation-plan.md`](spec/implementation-plan.md).

## Developer Checks

Run the current root checks:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
scripts/check-formal-models.sh
scripts/check-tla-models.sh
scripts/check-e2e.sh
```

`scripts/check-formal-models.sh` requires Maude. `scripts/check-tla-models.sh`
requires Apalache and Java; if they are not already on `PATH`, it uses the repo
Nix flake to provide them. The e2e script is a Stage 0 smoke test that proves
the new CLI workspace is executable; later stages will replace it with real
workflow execution tests.

## Development Shell

The repo includes a Nix dev shell for formal tooling:

```sh
nix develop
```

It provides:

```text
OpenJDK 21
Maude
Apalache 0.57.1
```
