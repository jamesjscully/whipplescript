# Troubleshooting

Status: draft

## Workflow Does Not Compile

Run:

```sh
whip check workflow.whip
```

Common issues:

- Equality guards such as `when status == Done` are not implemented yet.
- Effect outputs must be used inside a matching `after <effect> ...` block.
- `as binding` must be on the effect line.
- Unknown record fields are rejected.
- BAML `coerce` calls must match declared function arity.

## Effect Does Not Run

Inspect:

```sh
whip status <instance>
whip effects <instance>
whip runs <instance>
whip trace <instance> --check --json
```

Likely causes:

- instance is paused or cancelled
- upstream dependency has not reached the required terminal status
- capability/profile policy blocked the effect
- provider lease expired and the effect returned to queued

## Human Review Is Pending

List and answer inbox items:

```sh
whip inbox
whip inbox show <item>
whip inbox answer <item> --choice approve --by operator
```

## Formal Tool Missing

Run:

```sh
whip doctor
```

Maude is needed for generated model searches. Java and Apalache are needed for
TLA+ checks. The repo Nix shell provides formal tooling:

```sh
nix develop
```

## E2E Failure

Run:

```sh
scripts/check-e2e.sh
```

Kernel e2e tests write trace artifacts to the system temp directory before
checking conformance. The panic message includes the path when conformance
fails.
