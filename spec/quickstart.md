# CLI Quickstart

Status: draft

This quickstart uses the deterministic local workflow path. It does not require
real provider credentials.

## 1. Check Tooling

```sh
cargo build --workspace
cargo run -p whippletree-cli -- doctor
```

For formal and e2e checks:

```sh
scripts/check-formal-models.sh
scripts/check-tla-models.sh
scripts/check-e2e.sh
```

## 2. Compile A Workflow

```sh
cargo run -p whippletree-cli -- check examples/minimal-noop.whip
cargo run -p whippletree-cli -- compile examples/minimal-noop.whip
```

Use generated model searches when Maude is installed:

```sh
cargo run -p whippletree-cli -- check --model-search examples/loft-worker-with-review.whip
```

## 3. Run An Instance

```sh
cargo run -p whippletree-cli -- --store .whippletree/quickstart.sqlite \
  run examples/minimal-noop.whip \
  --input '{"ticket":"quickstart"}' \
  --json
```

Save the returned `instance_id`.

## 4. Inspect State

```sh
cargo run -p whippletree-cli -- --store .whippletree/quickstart.sqlite status <instance_id>
cargo run -p whippletree-cli -- --store .whippletree/quickstart.sqlite log <instance_id>
cargo run -p whippletree-cli -- --store .whippletree/quickstart.sqlite facts <instance_id>
cargo run -p whippletree-cli -- --store .whippletree/quickstart.sqlite trace <instance_id> --check --json
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
