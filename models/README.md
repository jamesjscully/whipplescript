# Formal Models

Status: draft

This directory holds formal and semi-formal models for the new Whippletree kernel.

The split is intentional:

```text
Maude        rule/effect-graph kernel and generated program checks
TLA+         control-plane lifecycle, leases, recovery, and event-log ordering
Veil/Lean    later high-assurance transition-system proofs
Trace        Rust-level conformance checker for implementation traces
```

The models are not product code. They are design tools and future regression
checks.

## Current Tooling Check

As of this pass in the local workspace:

```text
maude: installed, tested with scripts/check-formal-models.sh
java: provided by the repo Nix dev shell
apalache: provided by the repo Nix dev shell
lake/lean: lake found on PATH
```

The Maude kernel checks are currently runnable directly in this environment.
TLA+/Apalache checks run through `scripts/check-tla-models.sh`, which enters the
repo Nix dev shell when needed. Lean/Veil remains a planned follow-on validation
layer rather than an active check.

See also:

- [trace-conformance.md](trace-conformance.md) for the first runtime trace
  checker contract.
