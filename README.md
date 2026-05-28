# Whippletree

Whippletree is being redesigned as a restricted orchestration language for coding
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
agent harnesses, skills, BAML coercion, Loft integration, human review, and
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
crates/whippletree-core       shared types and contracts
crates/whippletree-parser     `.whip` source parser and typed IR
crates/whippletree-store      SQLite-backed runtime store
crates/whippletree-kernel     deterministic rule/effect runtime kernel
crates/whippletree-cli        control-plane CLI
spec/                      current design specs and implementation tracker
models/                    formal models and checks
scripts/                   root project checks
```

The current Rust crates implement the v0 spine: parser/IR snapshots, durable
SQLite store, runtime kernel, control-plane CLI, trace conformance, mock e2e
tests, generated Maude checks, BAML coerce integration, Loft effect contracts,
human review, skills, evidence, and plugin registration.

The source of truth for remaining work is
[`spec/implementation-plan.md`](spec/implementation-plan.md). Start with
[`spec/quickstart.md`](spec/quickstart.md) for CLI usage and
[`skills/whippletree-author/SKILL.md`](skills/whippletree-author/SKILL.md) when asking
a coding agent to author workflows.

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

For a single readiness artifact, run:

```sh
scripts/check-release-readiness.sh
```

Set `WHIPPLETREE_RELEASE_READINESS_FULL=1` to include clippy, workspace tests,
formal models, TLA, and e2e. Set `WHIPPLETREE_RELEASE_STRICT_EXTERNAL=1` when the
Loft submodule and real-provider prerequisites must be hard failures. The
default report path is `target/release-readiness-report.md`.
CI uploads the release readiness report and real-provider smoke report as a
single `readiness-reports` artifact.

`scripts/check-formal-models.sh` requires Maude. `scripts/check-tla-models.sh`
requires Apalache and Java; if they are not already on `PATH`, it uses the repo
Nix flake to provide them. `scripts/check-e2e.sh` runs deterministic mock
provider e2e tests and CLI control-plane checks.

Optional real-provider prerequisites are checked with:

```sh
WHIPPLETREE_E2E_REAL_PROVIDERS=1 \
WHIPPLETREE_LOFT_TEST_ISSUE=iss_... \
WHIPPLETREE_BAML_TEST_ENDPOINT=http://127.0.0.1:... \
WHIPPLETREE_BAML_TEST_FUNCTION=classifyMessage \
WHIPPLETREE_BAML_TEST_ARGUMENTS_JSON='{"title":"Smoke","body":"Check"}' \
WHIPPLETREE_BAML_TEST_OUTPUT_TYPE=MessageClassification \
scripts/check-real-providers.sh
```

Set `WHIPPLETREE_REAL_PROVIDERS=loft`, `WHIPPLETREE_REAL_PROVIDERS=baml`, or
`WHIPPLETREE_REAL_PROVIDERS=codex` to run only the provider smoke tests that are
configured. Comma-separated subsets such as `loft,baml,codex` are accepted. The
default is `loft,baml`.

For the smallest real Codex dogfood test, run:

```sh
scripts/check-codex-message.sh
```

It sends one non-interactive `codex exec` prompt, requires the final message to
match `WHIPPLETREE_CODEX_SMOKE_EXPECTED`, and writes
`target/codex-message-smoke-report.md`. Override the prompt, expected response,
model, or profile with `WHIPPLETREE_CODEX_SMOKE_PROMPT`,
`WHIPPLETREE_CODEX_SMOKE_EXPECTED`, `WHIPPLETREE_CODEX_MODEL`, and
`WHIPPLETREE_CODEX_PROFILE`.

Set `WHIPPLETREE_LOFT_REPO` when the Loft fixture repo is not available at
`vendor/loft`. Set `WHIPPLETREE_BAML_HEALTH_PATH` to add a non-destructive HTTP
health probe after the default TCP reachability check.

To capture a smoke-test artifact while preserving the underlying exit code, use:

```sh
scripts/check-real-providers-report.sh
```

Set `WHIPPLETREE_REAL_PROVIDER_REPORT=/path/to/report.md` to choose the report
path. The default is `target/real-provider-smoke-report.md`.

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
