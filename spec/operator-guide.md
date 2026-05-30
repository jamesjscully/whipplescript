# Operator Guide

Status: draft

WhippleScript runtime state is stored in SQLite. The default CLI store path is:

```text
.whipplescript/store.sqlite
```

Use `--store <path>` or `WHIPPLESCRIPT_STORE` to isolate environments.

## Store Operations

Create or open a store:

```sh
whip --store .whipplescript/prod.sqlite doctor
```

Inspect an instance:

```sh
whip --store .whipplescript/prod.sqlite status <instance>
whip --store .whipplescript/prod.sqlite log <instance>
whip --store .whipplescript/prod.sqlite effects <instance>
whip --store .whipplescript/prod.sqlite runs <instance>
whip --store .whipplescript/prod.sqlite evidence <instance> --json
whip --store .whipplescript/prod.sqlite trace <instance> --check --json
```

The store records events, facts, effects, dependencies, runs, leases, inbox
items, evidence, artifacts, capability registries, profiles, and plugin
manifests.

## Control Actions

Pause prevents new provider starts:

```sh
whip pause <instance>
```

Resume allows queued effects to run again:

```sh
whip resume <instance>
```

Cancel prevents new provider starts permanently:

```sh
whip cancel <instance>
```

Retry moves failed or timed-out effects back to queued:

```sh
whip retry <instance> <effect>
```

## Profiles And Providers

Profiles describe authority. Providers execute effect kinds. Capability bindings
grant a program access to a provider for a capability.

Default profiles include:

- `permissive`
- `repo-reader`
- `repo-writer`
- `internet-research`
- `human-review`

When an effect is blocked, inspect:

```sh
whip effects <instance>
whip status <instance> --json
whip evidence <instance> --json
```

The effect projection includes `policy_block_reason` when profile or capability
policy prevents a provider run.

## Recovery

The event log is the source of truth. Projections can be rebuilt from committed
rule events. E2E coverage exercises restart/rebuild behavior; operators should
preserve the SQLite file and collect trace/evidence output before manual repair.

Recommended incident bundle:

```sh
whip status <instance> --json
whip log <instance> --json
whip facts <instance> --json
whip effects <instance> --json
whip runs <instance> --json
whip evidence <instance> --json
whip trace <instance> --check --json
```
