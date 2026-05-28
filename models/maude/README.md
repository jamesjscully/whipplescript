# Maude Models

Maude is the primary formal target for the Whippletree rule kernel.

Use it to model:

```text
rule commits
effect nodes
effect dependency edges
claimability
completion/failure outcomes
bounded searches for bad rule cycles
```

The reusable kernel model lives at:

```sh
models/maude/kernel.maude
```

Hand-written executable checks live under:

```sh
models/maude/tests/
```

Run all Maude checks:

```sh
scripts/check-formal-models.sh
```

Current test suites:

```text
coerce-branches.maude       BAML-style coerce success/failure branches
loft-claim-turn.maude       claim-gated coding-agent turn lifecycle
effect-dependencies.maude   success/failure/completes dependency release
policy-capacity-retry.maude policy/capacity blocks, lease expiry, retry
ralph-loop.maude            external-event-bounded Ralph loop
```

The shell script runs every Maude test file and checks the expected number of
`No solution.` and `Solution 1` results for each suite. This is intentionally
simple: generated checks can later emit a richer manifest, but the first kernel
already fails CI when an expected safety search starts finding a path.
