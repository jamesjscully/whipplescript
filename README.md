# WhippleScript

A whippletree distributes load from different sources. WhippleScript is a
programming language for coordinating work across multiple AI agents.

WhippleScript is for programmers who want agents to collaborate in a repeatable
way: route work to specialized agents, wait for results, ask for review, retry
failures, and inspect what happened afterward.

> Warning: WhippleScript is early and not yet stable. The language, CLI, runtime
> behavior, and provider/plugin interfaces may change as the project settles.
> The current build is best for local experiments and prototype agent
> orchestration, not stable production dependencies.

## What You Can Use It For

- Split a goal across several agents with clear handoffs.
- Put human review or approval in the middle of an agent workflow.
- Keep a durable record of agent turns, facts, effects, logs, and traces.
- Retry, resume, or audit long-running work instead of losing context in chat.
- Connect agent turns to tools, skills, providers, schemas, and plugins.
- Test orchestration rules locally before trusting them with real work.

The target use case is not "write one better prompt." It is "describe the way
work should move between agents, tools, and people so the process can run again
and be inspected afterward."

## Try It

Install the released `whip` binary from GitHub Releases, or use Rust and Cargo
to install the current source build.

For prebuilt release installers and checksum verification, see
[`docs/install.md`](docs/install.md).

Clone the repo and install the CLI from the checkout:

```sh
git clone https://github.com/jamesjscully/whipplescript.git
cd whipplescript
cargo install --path crates/whipplescript-cli --locked
whip doctor
```

Check a multi-agent workflow:

```sh
whip check examples/multi-agent-bounded-concurrency.whip
```

Run a no-credentials local workflow with the fixture provider:

```sh
mkdir -p .whipplescript
whip --store .whipplescript/quickstart.sqlite \
  dev examples/minimal-noop.whip \
  --provider fixture \
  --until idle \
  --json
```

Inspect the result with the returned `instance_id`:

```sh
whip --store .whipplescript/quickstart.sqlite status <instance_id>
whip --store .whipplescript/quickstart.sqlite log <instance_id>
whip --store .whipplescript/quickstart.sqlite facts <instance_id>
```

For the full walkthrough, use the [Quickstart](docs/quickstart.md).
For a more useful first workflow, follow the [Tutorial](docs/tutorial.md).

## What A Workflow Looks Like

This is the shape of a simple multi-agent workflow:

```whip
workflow MultiAgentBoundedConcurrency

agent implementer {
  profile "repo-writer"
  capacity 2
}

agent reviewer {
  profile "repo-reader"
  capacity 1
}

rule implement_ready_work
  when {
    WorkItem as item
    implementer is available
  }
=> {
  tell implementer """
  Implement this work item:

  {{ item.title }}

  {{ item.body }}
  """
}

rule review_completed_turn
  when {
    worker completed turn for loft issue as turn
    reviewer is available
  }
=> {
  tell reviewer """
  Review this completed turn:

  {{ turn.summary }}

  Changed files:
  {{ turn.changedFiles }}
  """
}
```

See the checked examples in [`examples/`](examples/), especially:

- [`examples/multi-agent-bounded-concurrency.whip`](examples/multi-agent-bounded-concurrency.whip)
- [`examples/loft-worker-with-review.whip`](examples/loft-worker-with-review.whip)
- [`examples/codex-poem-coerce-review.whip`](examples/codex-poem-coerce-review.whip)
- [`examples/multi-provider-poem-review.whip`](examples/multi-provider-poem-review.whip)

## Docs By Goal

- I want to install it: [`docs/install.md`](docs/install.md)
- I want to run an example: [`docs/quickstart.md`](docs/quickstart.md)
- I want the main tutorial: [`docs/tutorial.md`](docs/tutorial.md)
- I want the core concepts: [`docs/concepts.md`](docs/concepts.md)
- I want to choose an example: [`docs/examples.md`](docs/examples.md)
- I want to write a workflow: [`docs/language-reference.md`](docs/language-reference.md)
- I want the full guide: [`docs/manual.md`](docs/manual.md)
- I want to know what works today: [`docs/current-state.md`](docs/current-state.md)
- I want to understand runtime state and failures:
  [`docs/runtime-operations.md`](docs/runtime-operations.md)
- I want exact CLI and API surfaces: [`docs/api-reference.md`](docs/api-reference.md)
- I want to plug in capabilities or providers:
  [`docs/providers.md`](docs/providers.md)
- I hit a setup or runtime problem: [`docs/troubleshooting.md`](docs/troubleshooting.md)

When asking a coding agent to author workflows, start with
[`skills/whipplescript-author/SKILL.md`](skills/whipplescript-author/SKILL.md).

## Good For / Not Yet Good For

Good for:

- local experiments with multi-agent workflows
- durable agent handoffs
- human review gates
- testing orchestration ideas
- inspecting what happened after a run
- building toward provider/plugin integrations

Not yet good for:

- depending on stable syntax
- unattended production automation
- assuming provider integrations will stay unchanged
- plug-and-play hosted agent orchestration

## How It Works

You describe who should do what, when to wait, when to ask for review, and what
counts as done. WhippleScript keeps the run durable and inspectable.

At a high level:

```text
facts/events + rules -> durable facts/effects
effects + workers    -> provider runs
provider results     -> events/facts
workflow terminals   -> completed/failed instances
```

Source composition is split by role:

- `include "schemas/common.whip"` composes source files.
- `use memory` imports a plugin.
- `pattern` and `apply` create reusable workflow fragments.
- `invoke` starts durable child workflows.
- skills are attached to agents or turns rather than imported as language
  extensions.

## For Contributors

The active implementation starts at the repository root.

```text
Cargo.toml                      Rust workspace
crates/whipplescript-core       shared types and contracts
crates/whipplescript-parser     `.whip` source parser and typed IR
crates/whipplescript-store      SQLite-backed runtime store
crates/whipplescript-kernel     deterministic rule/effect runtime kernel
crates/whipplescript-cli        control-plane CLI
docs/                           user-facing documentation
spec/                           design specs and implementation trackers
models/                         formal models and checks
scripts/                        root project checks
```

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

The source of truth for remaining work is
[`spec/implementation-plan.md`](spec/implementation-plan.md). Distribution work
is tracked in [`spec/distribution-tracker.md`](spec/distribution-tracker.md).

The repo also includes a Nix dev shell for formal tooling:

```sh
nix develop
```
