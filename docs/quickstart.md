# Quickstart

Run a workflow and inspect what happened, in about five minutes. Everything
here uses the deterministic fixture provider — no agent credentials required.

If facts, rules, and effects are new terms, skim [concepts](concepts.md)
first; it is a two-minute read.

## 1. Install

```sh
git clone https://github.com/jamesjscully/whipplescript.git
cd whipplescript
cargo install --path crates/whipplescript-cli --locked
whip doctor
```

Prebuilt binaries and platform notes are in [install](install.md). For usage
of any command, run `whip help <command>`.

## 2. Check a workflow

```sh
whip check examples/multi-agent-bounded-concurrency.whip
```

`check` parses, type-checks, and lowers the source, then prints the compiled
summary — declared agents, each rule's reads and writes, and the dependency
edges between rules:

```text
== examples/multi-agent-bounded-concurrency.whip
workflow MultiAgentBoundedConcurrency
agents
  agent implementer harness=<fallback> provider=codex profile=repo-writer capacity=2 ...
  agent reviewer harness=<fallback> provider=claude profile=repo-reader capacity=1 ...
rules
  rule implement_ready_work
  rule review_completed_turn
```

`check` also enforces two static liveness rules: every workflow must be able
to reach `complete` or `fail`, and every rule's reads must be producible.
See [liveness checks](language-reference.md#liveness-checks).

## 3. Run a workflow

`dev` starts an instance, steps rules, executes effects with the fixture
provider, and evaluates assertions, in a loop until the instance is idle:

```sh
mkdir -p .whipplescript
whip --store .whipplescript/quickstart.sqlite \
  dev examples/minimal-noop.whip \
  --provider fixture \
  --until idle \
  --json
```

Note the `instance_id` in the output. The interesting parts of the report:

```json
{
  "workflow": "MinimalNoop",
  "instance_id": "ins_...",
  "steps": [
    {"committed_rules": 1, "facts_created": 1, "effects_created": 0}
  ],
  "workers": [
    {"provider": "fixture", "ran_effects": 0}
  ]
}
```

One rule fired and recorded one fact. The workflow then ran
`complete result { ... }`, so the instance is finished.

## 4. Inspect the run

Every command that touches an instance takes the same `--store` that created
it (or set `WHIPPLESCRIPT_STORE` once).

```sh
whip --store .whipplescript/quickstart.sqlite status <instance_id>
whip --store .whipplescript/quickstart.sqlite facts  <instance_id>
whip --store .whipplescript/quickstart.sqlite log    <instance_id>
whip --store .whipplescript/quickstart.sqlite --json trace <instance_id> --check
```

`status` reports the instance as `completed`. `facts` shows the recorded
fact:

```text
StartupSeen StartupSeen:... {"source":"external.started","state":"observed"}
```

`trace --check` replays the effect lifecycle against the runtime's
conformance model and reports `"conformance": {"ok": true}`.

## 5. The pieces behind `dev`

`dev` composes three commands you can also run separately:

```sh
# start an instance (records the start event, nothing else)
whip --store .whipplescript/quickstart.sqlite \
  run examples/minimal-noop.whip --json

# advance deterministic rules for that instance
whip --store .whipplescript/quickstart.sqlite \
  step <instance_id> --program examples/minimal-noop.whip

# execute any ready effects through a provider
whip --store .whipplescript/quickstart.sqlite \
  worker <instance_id> --provider fixture
```

This separation matters once workflows wait on real agents or human input:
the instance is durable, so stepping and working can happen later, from
another process, or after a restart.

## Next

- The [tutorial](tutorial.md) builds a workflow from scratch: agent triage,
  a human approval gate, and a completed instance.
- The [examples catalog](examples.md) maps each shipped example to what it
  demonstrates.
- The [language reference](language-reference.md) covers every construct.
