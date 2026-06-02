# Troubleshooting

Status: draft

## Workflow Does Not Compile

Run:

```sh
whip check workflow.whip
```

Common issues:

- Equality guards must be written in `where` clauses, for example
  `when task where task.status == Done`.
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

## Workflow Revision Is Blocked

Preview the candidate first:

```sh
whip revise <instance> candidate.whip --root Workflow --dry-run --json
whip diagnostics <instance>
whip evidence <instance> --json
whip status <instance> --json
```

Common revision diagnostics:

- `revision.incompatible_root_workflow`: the candidate root workflow name does
  not match the running instance.
- `revision.incompatible_input_contract`: the candidate changed an already
  started workflow input contract.
- `revision.incompatible_output_contract` or
  `revision.incompatible_failure_contract`: parent invocations could no longer
  observe the declared terminal payload shape.
- `revision.incompatible_active_fact`: a live unconsumed fact no longer
  typechecks against the candidate schema.
- `revision.stale_program_path`: `whip step` was pointed at source that does
  not match the active revision.
- `revision.source_bundle_unavailable`: the candidate source bundle could not
  be read during revision.
- `provider.cancellation.unsupported`: `--cancel running` requested
  out-of-band cancellation from a provider that cannot acknowledge it. The run
  remains recoverable until it records a normal terminal outcome or lease
  recovery handles it.

Use `--cancel keep` when old-version effects should continue, `--cancel queued`
when queued old work should become terminal `cancelled`, and `--cancel running`
when running providers should receive cancellation requests.

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
