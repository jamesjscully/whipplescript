# CLI Quickstart

Status: draft

This quickstart uses the deterministic local workflow path. It does not require
real provider credentials.

For the authoring model, read
[`../docs/language-reference.md`](../docs/language-reference.md). For runtime
lifecycle and failure behavior, read
[`../docs/runtime-operations.md`](../docs/runtime-operations.md).

## 1. Check Tooling

```sh
cargo build --workspace
cargo run -p whipplescript-cli -- doctor
```

For formal and e2e checks:

```sh
scripts/check-formal-models.sh
scripts/check-tla-models.sh
scripts/check-e2e.sh
```

## 2. Compile A Workflow

```sh
cargo run -p whipplescript-cli -- check examples/minimal-noop.whip
cargo run -p whipplescript-cli -- compile examples/minimal-noop.whip
```

Use generated model searches when Maude is installed:

```sh
cargo run -p whipplescript-cli -- check --model-search examples/loft-worker-with-review.whip
```

## 3. Run An Instance

```sh
cargo run -p whipplescript-cli -- --store .whipplescript/quickstart.sqlite \
  run examples/minimal-noop.whip \
  --input '{"ticket":"quickstart"}' \
  --json
```

Save the returned `instance_id`.

## 4. Inspect State

```sh
cargo run -p whipplescript-cli -- --store .whipplescript/quickstart.sqlite status <instance_id>
cargo run -p whipplescript-cli -- --store .whipplescript/quickstart.sqlite log <instance_id>
cargo run -p whipplescript-cli -- --store .whipplescript/quickstart.sqlite facts <instance_id>
cargo run -p whipplescript-cli -- --store .whipplescript/quickstart.sqlite trace <instance_id> --check --json
```

## 5. Use Examples As Starting Points

Checked examples live in `examples/`:

- `minimal-noop.whip`
- `ralph.whip`
- `loft-worker-with-review.whip`
- `coerce-branch.whip`
- `human-review.whip`
- `multi-agent-bounded-concurrency.whip`
- `openclaw-lite.whip`
- `plugin-memory.whip`
- `provider-language-e2e.whip`

Each has a matching `.ir` snapshot used by parser tests.
