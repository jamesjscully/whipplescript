# Database Migration Story

WhippleScript stores runtime state in SQLite. The database is durable workflow
infrastructure, not the authoring surface.

## Current Contract

- The store records schema version `4` in `whipplescript_meta`.
- Startup creates missing tables for the current version.
- Startup rejects newer schema versions instead of trying to run against an
  unknown layout.
- Migrations are explicit Rust code in the engine storage layer.
- Existing runtime records are append-only where practical: events, transition
  logs, effect logs, and coerce calls should not be rewritten to hide history.

## Existing Migration Behavior

The engine already handles the event identity migration from the older global
event uniqueness shape to workflow-scoped event identity. Tests cover opening a
legacy-shaped store and migrating it to the current schema.

The legacy shape used a unique `event_id` index without `workflow_id`, which
made the same external event id collide across workflows. The current schema
uses:

```text
UNIQUE(workflow_id, event_id)
```

and preserves the existing `event_json` envelopes during migration.

## Rules For Future Migrations

1. Additive schema changes are preferred.
2. Data migrations must be deterministic and idempotent.
3. New indexes should be safe to create if absent.
4. Runtime code must fail closed on unsupported newer versions.
5. Migrations must preserve enough data for `status`, `events`, `log`, and
   coerce replay diagnostics.
6. If a migration changes projection semantics, add a recovery or projection
   test that opens a pre-migration fixture.
7. Do not require operators to edit SQLite manually.

## Testing Requirements

Every schema migration should include:

- a fresh-store test
- a legacy-store opening test
- a projection test when status/log behavior changes
- a rejection test for a newer unsupported schema version

## Operational Guidance

Before running a new WhippleScript version against important workflow state:

```sh
cp workflow.sqlite workflow.sqlite.backup
whip status workflow.whip --store workflow.sqlite --json
```

If opening the store fails due to a newer schema version, use the matching
WhippleScript binary for that store. Do not downgrade by editing metadata.

## Non-Goals

- online multi-writer migration coordination
- manual SQL migration instructions for normal users
- support for arbitrary direct database writes by adapters
- treating SQLite as the workflow API
