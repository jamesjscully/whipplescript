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
items, evidence, artifacts, capability registries, profiles, package manifests,
and provider registrations.

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

## Workflow Revision

Use `whip revise` when a non-terminal running instance should move to a newer
program version. Revision is an operator control-plane action, not a workflow
rule operation.

Always preview first:

```sh
whip revise <instance> candidate.whip --root Workflow --dry-run --cancel keep
```

The dry-run reports compatibility, active version/epoch, candidate hashes,
impacted old-version effects, cancellation actions, and diagnostics/evidence
that activation would create.

Activate only after reviewing the dry-run:

```sh
whip revise <instance> candidate.whip --root Workflow --cancel keep
```

Cancellation policies:

| Policy | Use when | Behavior |
| --- | --- | --- |
| `keep` | Old work should finish under its original version. | Existing effects keep their old version/epoch attribution and remain runnable. |
| `queued` | Queued old work should be abandoned. | Queued, blocked, and claimable old effects become terminal `cancelled`. |
| `running` | Running old work should be asked to stop. | Queued old effects cancel; running effects receive cancellation requests and finish through provider acknowledgement or normal terminal outcome. |

Rollback is another revision to a prior compatible source bundle:

```sh
whip revise <instance> previous.whip --root Workflow --dry-run
whip revise <instance> previous.whip --root Workflow --cancel keep
```

Terminal instances cannot be revised. Parent and child workflow instances revise
independently; revising a parent does not revise or terminate an already-running
child invocation.

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
