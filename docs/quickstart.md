# WhippleScript Quickstart

This quickstart uses the deterministic fixture provider. It does not require
real agent credentials.

## 1. Install The CLI

From a checkout:

```sh
git clone https://github.com/jamesjscully/whipplescript.git
cd whipplescript
cargo install --path crates/whipplescript-cli --locked
whip doctor
```

See [Install WhippleScript](install.md) for source install alternatives,
planned binary releases, and troubleshooting.

## 2. Check A Workflow

```sh
whip check examples/multi-agent-bounded-concurrency.whip
```

This verifies that the workflow parses, type-checks, and lowers to the current
intermediate representation.

Expected shape:

```text
== examples/multi-agent-bounded-concurrency.whip
workflow MultiAgentBoundedConcurrency
agents
  agent implementer profile=repo-writer capacity=2
  agent reviewer profile=repo-reader capacity=1
rules
  rule implement_ready_work
  rule review_completed_turn
```

## 3. Run A Local Workflow

Use `dev` first. It starts an instance, steps deterministic rules, runs fixture
workers, and evaluates workflow assertions.

```sh
mkdir -p .whipplescript
whip --store .whipplescript/quickstart.sqlite \
  dev examples/minimal-noop.whip \
  --provider fixture \
  --until idle \
  --json
```

Save the returned `instance_id`.

Expected shape:

```json
{
  "workflow": "MinimalNoop",
  "steps": [
    {"committed_rules": 1, "facts_created": 1}
  ],
  "workers": [
    {"provider": "fixture", "ran_effects": 0}
  ]
}
```

## 4. Inspect What Happened

```sh
whip --store .whipplescript/quickstart.sqlite status <instance_id>
whip --store .whipplescript/quickstart.sqlite log <instance_id>
whip --store .whipplescript/quickstart.sqlite facts <instance_id>
whip --store .whipplescript/quickstart.sqlite trace <instance_id> --check --json
```

You should see a `StartupSeen` fact from `examples/minimal-noop.whip`.

Expected `facts` shape:

```text
StartupSeen StartupSeen:... {"source":"external.started","state":"observed"}
```

Expected `trace --check --json` shape:

```json
{
  "schema": "whipplescript.local_trace.v0",
  "conformance": {"ok": true}
}
```

## 5. Understand `run`, `step`, And `dev`

`run` starts an instance and records the start event. It does not evaluate rules
or run providers.

```sh
whip --store .whipplescript/quickstart.sqlite \
  run examples/minimal-noop.whip \
  --input '{"ticket":"quickstart"}' \
  --json
```

Use `step` to advance deterministic rules for that instance:

```sh
whip --store .whipplescript/quickstart.sqlite \
  step <instance_id> --program examples/minimal-noop.whip
```

Use `worker` to run already-materialized effects through a provider. Use `dev`
when you want the local validation loop to compose `run`, `step`, and fixture
workers for you.

## 6. Try More Examples

Checked examples live in [`../examples/`](../examples/):

- `multi-agent-bounded-concurrency.whip`
- `loft-worker-with-review.whip`
- `codex-poem-coerce-review.whip`
- `multi-provider-poem-review.whip`
- `human-review.whip`
- `plugin-memory.whip`
- `provider-language-e2e.whip`

Use [Language Reference](language-reference.md) when you are ready to write a
workflow, [Concepts](concepts.md) when you want the core terms, and
[Runtime And Operations Reference](runtime-operations.md) when you want to
understand stores, effects, workers, and failures.
