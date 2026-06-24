# Troubleshooting

Common problems in roughly the order new users hit them.

For a searchable catalog of compiler, runtime, revision, assertion, and
fixture diagnostics, see the [Diagnostics guide](diagnostics.md).

## `whip` is not found

Cargo installs to `~/.cargo/bin`. Add it to `PATH` and open a new shell:

```sh
export PATH="$HOME/.cargo/bin:$PATH"
whip --version
```

If `cargo` itself is missing, install Rust from <https://rustup.rs/>.

## `run` produced no facts

Expected: `run` only starts the instance and records `external.started`.
Use `dev` for the full local loop, or advance the instance yourself:

```sh
whip --store <store> step <instance_id> --program <workflow.whip>
```

## I lost the instance id / used the wrong store

Instances live in the store file that created them. List what a store holds:

```sh
whip --store .whipplescript/quickstart.sqlite instances
```

Every command that reads an instance needs that same `--store`, or set it
once with `export WHIPPLESCRIPT_STORE=<path>`.

## A counter/lease/queue example behaves differently on repeated runs

Counters, leases, ledgers, and the builtin queue tracker live in a
**workspace-scoped** coordination store (`.whipplescript/coordination.sqlite`,
or `WHIPPLESCRIPT_COORDINATION_STORE`) that is *separate* from the per-instance
run store and is **not** reset by `--store` / `WHIPPLESCRIPT_STORE`. This is by
design — a `counter failure_budget { cap 3 reset daily }` is a budget shared by
every instance for that key until the period rolls over.

So re-running `examples/circuit-breaker.whip` four times in one day consumes the
budget across runs: the first three failures land on the `ok` branch and the
fourth trips to `over` (`whip counters` shows `consumed=3`). That is correct
breaker behaviour, not a per-run bug. To exercise such an example from a clean
budget, point the coordination store at a throwaway path too:

```sh
WHIPPLESCRIPT_COORDINATION_STORE=$(mktemp -u) \
WHIPPLESCRIPT_STORE=$(mktemp -u) whip dev examples/circuit-breaker.whip --until idle
```

Inspect or confirm shared state with `whip counters`, `whip leases`, and
`whip ledger`.

## Multiple workflows need `--root`

When a source bundle declares several workflows, name the root:

```sh
whip check examples/revision-ticket-v1.whip --root RevisionTicket
```

The same applies to `run`, `dev`, `step`, and `revise`.

## `whip check` reports a liveness error

```text
error: workflow `X` has no rule that reaches `complete` or `fail`
```

Add a rule that runs `complete` or `fail`, or tag the workflow `@service` if
it intentionally runs forever.

```text
error: rule `X` can never fire: nothing produces `Y`
```

Make `Y` producible — seed it from a `table`, `record` it in another rule, or
declare it as a workflow `input`. If it arrives from outside the workflow,
tag the rule `@external`.

## `whip doctor` reports missing tools

Most are optional. Fixture-backed development needs none of the formal-model,
model-decision, or native-provider tools. Install optional tools only for formal
checks or real-provider work.

## `cargo install --git` fails

Install from a checkout instead; if that works, the Git path failure is a
network, lockfile, or toolchain issue:

```sh
git clone https://github.com/jamesjscully/whipplescript.git
cd whipplescript
cargo install --path crates/whipplescript-cli --locked
```

## Real provider checks are skipped or fail

Real-provider smoke tests are opt-in and gated by environment variables:

```sh
WHIPPLESCRIPT_E2E_REAL_PROVIDERS=1 \
WHIPPLESCRIPT_REAL_PROVIDERS=coerce,codex \
scripts/check-real-providers.sh
```

Read the per-provider JSON report first —
`target/real-provider-reports/<provider>.json` records environment posture,
check counts, and redacted preflight results. Provider failures surface as
diagnostics, evidence, and run/effect status, not as generic command
failures.

## A native agent turn failed and I can't tell why

`whip status` only shows the instance stuck or failed. The provider's reason
for a failed agent turn is recorded as control-plane metadata on the effect —
it crosses the evidence redaction boundary precisely because it is operational,
not model output. Read it directly:

```sh
whip diagnostics <instance>   # failure diagnostic carries the provider reason
whip effects <instance>       # effect evidence summary carries it too
```

Common reasons: `usageLimitExceeded` (Codex/ChatGPT plan quota — wait for the
reset window), an auth rejection (re-run the provider's own login, e.g.
`codex login`), or "no model configured" (set `default_model` in the provider
config or `model` in `~/.codex/config.toml`). The reason is capped and
secret-redacted, so it names the cause without echoing a token. Prompts and
model output stay shape-redacted and never appear here.

## Native provider strict mode fails

Strict mode validates real native adapters:

```sh
WHIPPLESCRIPT_E2E_REAL_PROVIDERS=1 \
WHIPPLESCRIPT_REAL_PROVIDER_NATIVE_STRICT=1 \
WHIPPLESCRIPT_PROVIDER_CONFIGS=examples/provider-configs/native/native.example.json \
scripts/check-real-providers-report.sh
```

Common messages:

- `WHIPPLESCRIPT_PROVIDER_CONFIGS is required in native strict mode` — set it
  to a colon-separated list of provider config files.
- `command-wrapper provider is not accepted in native strict mode` — strict
  mode requires the Codex, Claude, and Pi native surfaces.
- `missing required native provider config` — add bindings for `codex-main`,
  `claude-main`, and `pi-main`.

For a single-provider probe, use surface mode instead:

```sh
WHIPPLESCRIPT_E2E_REAL_PROVIDERS=1 \
WHIPPLESCRIPT_REAL_PROVIDER_NATIVE_SURFACE=1 \
WHIPPLESCRIPT_REAL_PROVIDERS=codex \
scripts/check-real-providers-report.sh
```

## Destructive provider tests are refused

By design. They require an explicit disposable-target marker and
acknowledgement:

```sh
WHIPPLESCRIPT_REAL_PROVIDER_DESTRUCTIVE_TESTS=1 \
WHIPPLESCRIPT_REAL_PROVIDER_DISPOSABLE_TARGET=native-provider-ci-sandbox \
WHIPPLESCRIPT_REAL_PROVIDER_DISPOSABLE_ACK=I_UNDERSTAND_THIS_PROVIDER_TARGET_IS_DISPOSABLE \
scripts/check-real-providers-report.sh
```

Provider-specific variants exist (for example `WHIPPLESCRIPT_PI_DESTRUCTIVE_TESTS`).
Reports record only whether the markers are set, never their values.
