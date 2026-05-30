# Maude Models

Maude is the primary formal target for the WhippleScript rule kernel.

Use it to model:

```text
rule commits
guard/readiness evaluation
workflow assertions
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
expression-kernel.maude     guards, assertions, optional reads, AgentRef targets
policy-capacity-retry.maude policy/capacity blocks, lease expiry, retry
ralph-loop.maude            external-event-bounded Ralph loop
workflow-composition.maude  pattern elaboration, workflow completion, invocation
```

The shell script runs every Maude test file and checks the expected number of
`No solution.` and `Solution 1` results for each suite. This is intentionally
simple: generated checks can later emit a richer manifest, but the first kernel
already fails CI when an expected safety search starts finding a path.

## Expression Kernel Model

The Maude kernel includes the finite expression-kernel abstraction from
`spec/expression-kernel.md`. It adds guard and assertion semantics without
turning Maude into a JSON/string interpreter.

Target checks:

```text
false guard cannot fire a rule
error guard cannot commit facts/effects
true guard preserves existing effect dependency searches
assertion failure cannot mutate workflow state
optional missing path cannot be read without a presence proof
enum/literal guards cannot match values outside their domain
dynamic tell cannot target an undeclared agent
```

Recommended shape:

```text
fact(F) + guard(R, F, true)  -> ruleReady(R, F, G)
fact(F) + guard(R, F, false) -> no rewrite
fact(F) + guard(R, F, error) -> diagnostic, no graph
```
