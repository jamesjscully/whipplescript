# Release Checklist

Use this before publishing a statechart-workflow release or asking users to
upgrade important workflow stores.

## Required Checks

```sh
cargo fmt --all --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo build -p whipplescript-cli
scripts/check-docs.sh
scripts/check-e2e.sh
scripts/check-formal-models.sh
git diff --check
```

## Compatibility Checks

- New `.whip` syntax is documented in `grammar.md` and `authoring-format.md`.
- New expression behavior is documented in `expression-primitives.md`.
- New adapter effects or events are documented in `component-contracts.md`.
- New policy fields are documented in `policy.md` and have example JSON.
- New storage migrations are documented in `database-migrations.md` and tested.
- Generated TLA+/Maude behavior still matches the hand-written model notes.

## Product Checks

- README command examples still run or are clearly marked as illustrative.
- `skills/whipplescript-statechart/SKILL.md` reflects the current CLI surface.
- At least one template validates, builds, runs, processes a typed completion,
  reaches settled status, and the local human-review response bridge is checked
  through `scripts/check-docs.sh`.
- `overview` and `status` explain blocked effects without reading custom logs.
- Capability errors identify the effect and exact required capability.
- Legacy script-runner behavior is described as migration context, not the
  primary product surface.

## Release Notes Checklist

Include:

- new DSL syntax or validation rules
- new adapter manifest fields or built-in adapter shortcuts
- new policy behavior, especially stricter enterprise defaults
- database schema migration notes
- formal model coverage changes
- known limitations and deferred integrations

## Store Upgrade Guidance

For important workflow stores, tell operators to back up SQLite state before
running a new binary:

```sh
cp workflow.sqlite workflow.sqlite.backup
whip status workflow.whip --store workflow.sqlite --json
```

If the binary rejects a newer schema version, use the matching newer WhippleScript
binary. Do not edit schema metadata manually.
