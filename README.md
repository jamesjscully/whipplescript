# WhippleScript

WhippleScript is a language for coordinating work across multiple AI agents.
You declare typed facts, agents, and rules; the runtime turns matched rules
into durable effects, executes them through providers, and records every
event, fact, and provider run in an inspectable store.

A whippletree distributes load from different sources. So does this.

> WhippleScript is pre-1.0. The language, CLI, and provider interfaces may
> change between releases. See [current state](docs/current-state.md) for
> what is stable enough to rely on today.

## Why

Chat transcripts make poor orchestration records. When several agents hand
work to each other — with review gates, retries, and human approval in the
middle — you want the process written down as code that can run again, and a
durable record of what actually happened.

WhippleScript separates the two concerns:

- **Rules decide.** Deterministic policy: what happens next, given the current
  facts. No I/O, no model calls.
- **Effects do.** Agent turns, typed model decisions, human review requests,
  and child workflows are durable effects, executed by workers through
  providers, with results recorded as events.

The result is a workflow you can step, pause, resume, revise, and audit.

## A taste

A triage workflow: an agent proposes a plan for each open ticket, and a human
signs off on the high-severity ones.

```whip
rule triage_open_ticket
  when Ticket as ticket where ticket.status == "open"
  when triager is available
=> {
  tell triager as turn """markdown
  Suggest an owner and a fix plan for this ticket:

  {{ ticket.title }} (severity: {{ ticket.severity }})
  """

  after turn succeeds as triaged {
    done ticket -> record TriagedTicket {
      id ticket.id
      title ticket.title
      severity ticket.severity
      plan triaged.summary
      status "triaged"
    }
  }
}

rule request_signoff
  when TriagedTicket as ticket where ticket.severity == "high"
=> {
  askHuman as signoff """markdown
  {{ ticket.title }} was triaged with this plan:

  {{ ticket.plan }}

  Approve or reject the plan.
  """
}

rule approve_plan
  when human answered signoff as answer where answer.choice == "approve"
=> {
  complete result {
    decision answer.choice
    decidedBy answer.answered_by
  }
}
```

The [tutorial](docs/tutorial.md) builds this workflow from scratch and runs
it end to end — including answering the human review from the CLI.

## Install

Prebuilt binaries are published on GitHub Releases:

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/jamesjscully/whipplescript/releases/latest/download/whipplescript-installer.sh | sh
```

Or install from source:

```sh
git clone https://github.com/jamesjscully/whipplescript.git
cd whipplescript
cargo install --path crates/whipplescript-cli --locked
whip doctor
```

See [install](docs/install.md) for Windows, checksums, and troubleshooting.

## Run something

The fixture provider executes workflows deterministically with no
credentials, so you can validate orchestration before wiring up real agents:

```sh
whip --store .whipplescript/quickstart.sqlite \
  dev examples/minimal-noop.whip --provider fixture --until idle --json
```

Then inspect the run:

```sh
whip --store .whipplescript/quickstart.sqlite status <instance_id>
whip --store .whipplescript/quickstart.sqlite facts  <instance_id>
whip --store .whipplescript/quickstart.sqlite log    <instance_id>
```

## Documentation

| | |
| --- | --- |
| [Quickstart](docs/quickstart.md) | Install, run an example, inspect the result. |
| [Tutorial](docs/tutorial.md) | Build a triage workflow with a human approval gate. |
| [Concepts](docs/concepts.md) | The execution model: facts, rules, effects, workers. |
| [Language reference](docs/language-reference.md) | Every construct in `.whip` source. |
| [CLI & API reference](docs/api-reference.md) | Commands, JSON shapes, status values, crate APIs. |
| [Runtime & operations](docs/runtime-operations.md) | Stores, lifecycle, failures, revision, recovery. |
| [Providers & plugins](docs/providers.md) | Fixture and native providers, credentials, plugins. |
| [Examples](docs/examples.md) | The checked example catalog. |
| [Troubleshooting](docs/troubleshooting.md) | Common first-session problems. |
| [Current state](docs/current-state.md) | What works today and what is still settling. |

When pointing a coding agent at WhippleScript, start it with
[`skills/whipplescript-author/SKILL.md`](skills/whipplescript-author/SKILL.md).

## Contributing

The workspace is plain Cargo:

```text
crates/whipplescript-core     shared types and contracts
crates/whipplescript-parser   .whip parser and typed IR
crates/whipplescript-store    SQLite-backed runtime store
crates/whipplescript-kernel   deterministic rule/effect kernel
crates/whipplescript-cli      the whip CLI
docs/                         user documentation
spec/                         design records and implementation trackers
models/                       formal models (Maude, TLA+)
```

Before sending changes:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

`scripts/check-release-readiness.sh` runs the full release gate, including the
formal model checks (a Nix dev shell with the tooling is provided:
`nix develop`). Remaining work is tracked in
[`spec/implementation-plan.md`](spec/implementation-plan.md).
