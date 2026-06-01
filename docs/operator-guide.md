# Operator Guide

This page is a user-facing map for operating local WhippleScript instances. The
deeper design record remains in [`../spec/operator-guide.md`](../spec/operator-guide.md).

## Pick A Store

Use an explicit store for each experiment:

```sh
whip --store .whipplescript/dev.sqlite doctor
```

Every command that inspects or changes an instance must use the same store.

## Inspect An Instance

```sh
whip --store .whipplescript/dev.sqlite status <instance>
whip --store .whipplescript/dev.sqlite log <instance>
whip --store .whipplescript/dev.sqlite facts <instance>
whip --store .whipplescript/dev.sqlite effects <instance>
whip --store .whipplescript/dev.sqlite runs <instance>
whip --store .whipplescript/dev.sqlite evidence <instance> --json
whip --store .whipplescript/dev.sqlite trace <instance> --check --json
```

## Control Lifecycle

```sh
whip --store .whipplescript/dev.sqlite pause <instance>
whip --store .whipplescript/dev.sqlite resume <instance>
whip --store .whipplescript/dev.sqlite cancel <instance>
whip --store .whipplescript/dev.sqlite retry <instance> <effect>
```

Pause and resume are nonterminal. Cancel is terminal. Retry moves eligible
failed or timed-out effects back to queued when policy allows.

## Revise A Running Instance

Preview first:

```sh
whip --store .whipplescript/dev.sqlite \
  revise <instance> candidate.whip --root Workflow --dry-run
```

Activate only after compatibility checks are acceptable:

```sh
whip --store .whipplescript/dev.sqlite \
  revise <instance> candidate.whip --root Workflow --cancel keep
```

Use [Runtime And Operations Reference](runtime-operations.md) for details on
effects, provider failures, workflow invocation, and revision behavior.
