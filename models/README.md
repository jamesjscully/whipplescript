# Formal Models

Status: draft

This directory holds formal and semi-formal models for the new Armature kernel.

The split is intentional:

```text
Maude        rule/effect-graph kernel and generated program checks
TLA+         control-plane lifecycle, leases, recovery, and event-log ordering
Veil/Lean    later high-assurance transition-system proofs
```

The models are not product code. They are design tools and future regression
checks.

## Current Tooling Check

As of this pass in the local workspace:

```text
maude: installed, tested with scripts/check-formal-models.sh
java: not found on PATH
apalache: not found on PATH
lake/lean: lake found on PATH
```

The Maude kernel smoke tests are currently runnable in this environment. TLA+,
Apalache, and Lean/Veil remain planned follow-on validation layers rather than
active checks.
