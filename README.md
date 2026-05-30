# WhippleScript

WhippleScript is being redesigned as a restricted orchestration language for coding
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

Current user-facing documentation starts in `docs/`. Current design work and
implementation trackers live in `spec/`.

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
crates/whipplescript-core       shared types and contracts
crates/whipplescript-parser     `.whip` source parser and typed IR
crates/whipplescript-store      SQLite-backed runtime store
crates/whipplescript-kernel     deterministic rule/effect runtime kernel
crates/whipplescript-cli        control-plane CLI
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
[`docs/README.md`](docs/README.md) for the documentation map,
[`docs/manual.md`](docs/manual.md) for the end-to-end manual,
[`docs/api-reference.md`](docs/api-reference.md) for exact API surfaces,
[`docs/language-reference.md`](docs/language-reference.md) for `.whip`
authoring, [`spec/quickstart.md`](spec/quickstart.md) for CLI usage, and
[`skills/whipplescript-author/SKILL.md`](skills/whipplescript-author/SKILL.md) when asking
a coding agent to author workflows.

Source composition is intentionally split by role: `use memory` imports a
plugin, `include "schemas/common.whip"` composes source files, `pattern`/`apply`
provides compile-time reusable workflow fragments, `invoke` starts durable child
workflows, and Claude-style skills are attached to agents or turns rather than
imported as top-level language extensions.

## Install

Early builds are installed from source. The command-line binary is named `whip`.

Install from the repository:

```sh
cargo install --git https://github.com/jamesjscully/whipplescript.git --package whipplescript-cli --locked
```

From a local checkout, install into Cargo's bin directory:

```sh
cargo install --path crates/whipplescript-cli --locked
```

Verify the installed binary:

```sh
whip --version
whip doctor
```

The distribution tracker for GitHub Releases, Homebrew, crates.io, and signed
artifacts lives in [`spec/distribution-tracker.md`](spec/distribution-tracker.md).

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

Set `WHIPPLESCRIPT_RELEASE_READINESS_FULL=1` to include clippy, workspace tests,
formal models, TLA, and e2e. Set `WHIPPLESCRIPT_RELEASE_STRICT_EXTERNAL=1` when the
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
WHIPPLESCRIPT_E2E_REAL_PROVIDERS=1 \
WHIPPLESCRIPT_LOFT_TEST_ISSUE=iss_... \
WHIPPLESCRIPT_BAML_TEST_ENDPOINT=http://127.0.0.1:... \
WHIPPLESCRIPT_BAML_TEST_FUNCTION=classifyMessage \
WHIPPLESCRIPT_BAML_TEST_ARGUMENTS_JSON='{"title":"Smoke","body":"Check"}' \
WHIPPLESCRIPT_BAML_TEST_OUTPUT_TYPE=MessageClassification \
scripts/check-real-providers.sh
```

Set `WHIPPLESCRIPT_REAL_PROVIDERS=loft`, `WHIPPLESCRIPT_REAL_PROVIDERS=baml`, or
`WHIPPLESCRIPT_REAL_PROVIDERS=codex` to run only the provider smoke tests that are
configured. Comma-separated subsets such as `loft,baml,codex` are accepted. The
default is `loft,baml`.

For the smallest real Codex validation test, run:

```sh
scripts/check-codex-message.sh
```

It sends one non-interactive `codex exec` prompt, requires the final message to
match `WHIPPLESCRIPT_CODEX_SMOKE_EXPECTED`, and writes
`target/codex-message-smoke-report.md`. Override the prompt, expected response,
model, or profile with `WHIPPLESCRIPT_CODEX_SMOKE_PROMPT`,
`WHIPPLESCRIPT_CODEX_SMOKE_EXPECTED`, `WHIPPLESCRIPT_CODEX_MODEL`, and
`WHIPPLESCRIPT_CODEX_PROFILE`.

Set `WHIPPLESCRIPT_LOFT_REPO` when the Loft fixture repo is not available at
`vendor/loft`. Set `WHIPPLESCRIPT_BAML_HEALTH_PATH` to add a non-destructive HTTP
health probe after the default TCP reachability check.

To capture a smoke-test artifact while preserving the underlying exit code, use:

```sh
scripts/check-real-providers-report.sh
```

Set `WHIPPLESCRIPT_REAL_PROVIDER_REPORT=/path/to/report.md` to choose the report
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
