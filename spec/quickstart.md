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

## 5. Preview A Revision

Start the revision v1 example:

```sh
whip --store .whipplescript/quickstart.sqlite \
  run examples/revision-ticket-v1.whip \
  --root RevisionTicket \
  --input '{"ticket":{"title":"quickstart","body":"initial workflow"}}' \
  --json
```

Save the returned `instance_id`, then preview the compatible v2 candidate
without mutating the store:

```sh
whip --store .whipplescript/quickstart.sqlite \
  revise <instance_id> examples/revision-ticket-v2.whip \
  --root RevisionTicket \
  --dry-run \
  --cancel keep
```

If the dry-run is compatible, activate from the control plane:

```sh
whip --store .whipplescript/quickstart.sqlite \
  revise <instance_id> examples/revision-ticket-v2.whip \
  --root RevisionTicket \
  --cancel keep
```

Use `--cancel queued` to terminal-cancel queued old-version effects, or
`--cancel running` to request cancellation for running old-version provider
work. Source rules may propose candidate files, but they do not activate
revisions.

## 6. Use Examples As Starting Points

Checked examples live in `examples/`:

- `minimal-noop.whip`
- `revision-ticket-v1.whip`
- `revision-ticket-v2.whip`
- `revision-running-cancel.whip`
- `revision-parent-child.whip`
- `revision-repair-planner.whip`
- `revision-validation-approval.whip`
- `ralph.whip`
- `loft-worker-with-review.whip`
- `coerce-branch.whip`
- `human-review.whip`
- `multi-agent-bounded-concurrency.whip`
- `openclaw-lite.whip`
- `plugin-memory.whip`
- `provider-language-e2e.whip`

Each has a matching `.ir` snapshot used by parser tests.
