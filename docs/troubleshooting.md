# Troubleshooting

This page covers common failures in the first few minutes.

## `cargo` Or `rustc` Is Missing

Install Rust from <https://rustup.rs/>, then open a new shell:

```sh
rustc --version
cargo --version
```

## `whip` Is Not Found

Cargo installs binaries into `~/.cargo/bin` by default. Add it to `PATH`:

```sh
export PATH="$HOME/.cargo/bin:$PATH"
```

Then open a new shell and run:

```sh
whip --version
whip doctor
```

## `run` Produced No Facts

That is expected. `run` starts an instance and records `external.started`; it
does not evaluate rules or execute providers.

For first experiments, use:

```sh
whip --store .whipplescript/quickstart.sqlite \
  dev examples/minimal-noop.whip \
  --provider fixture \
  --until idle \
  --json
```

If you already used `run`, step the instance:

```sh
whip --store .whipplescript/quickstart.sqlite \
  step <instance_id> --program examples/minimal-noop.whip
```

## I Lost The Instance ID

List instances in the store:

```sh
whip --store .whipplescript/quickstart.sqlite instances
```

Then inspect one:

```sh
whip --store .whipplescript/quickstart.sqlite status <instance_id>
```

## I Used The Wrong Store

Every command that reads an instance must use the same store path that created
it. If you ran with:

```sh
--store .whipplescript/tutorial.sqlite
```

use that same `--store` for `status`, `facts`, `effects`, and `trace`.

You can also set:

```sh
export WHIPPLESCRIPT_STORE=.whipplescript/tutorial.sqlite
```

## Multiple Workflows Need `--root`

If a source bundle exposes multiple workflows, select the root workflow:

```sh
whip check examples/revision-ticket-v1.whip --root RevisionTicket
```

The same applies to `run`, `dev`, `step`, and `revise` where a root workflow is
ambiguous.

## Real Provider Checks Are Skipped Or Fail

Real provider smoke tests are opt-in. Start with the fixture provider unless
you are intentionally testing external integrations.

When using real-provider scripts, check the required environment variables:

```sh
WHIPPLESCRIPT_E2E_REAL_PROVIDERS=1 \
WHIPPLESCRIPT_REAL_PROVIDERS=loft,baml,codex \
scripts/check-real-providers.sh
```

Provider failures should become diagnostics, evidence, run status, and effect
status. They should not be hidden as generic command failures.

The report wrapper writes the combined report plus per-provider JSON reports:

```text
target/real-provider-smoke-report.md
target/real-provider-preflight.jsonl
target/real-provider-reports/<provider>.json
```

Check the provider JSON report first. It records set/unset environment posture,
evidence refs, check counts, and redacted preflight records.

## Native Provider Strict Mode Fails

Strict native validation is intentionally stricter than command-wrapper smoke
coverage:

```sh
WHIPPLESCRIPT_E2E_REAL_PROVIDERS=1 \
WHIPPLESCRIPT_REAL_PROVIDER_NATIVE_STRICT=1 \
WHIPPLESCRIPT_PROVIDER_CONFIGS=examples/provider-configs/native/native.example.json \
scripts/check-real-providers-report.sh
```

Common failures:

- `WHIPPLESCRIPT_PROVIDER_CONFIGS is required in native strict mode`: set it to
  a colon-separated list of provider config files. The legacy
  `WHIPPLESCRIPT_NATIVE_PROVIDER_CONFIGS` name is still accepted.
- `command-wrapper provider is not accepted in native strict mode`: strict mode
  requires Codex, Claude, and Pi native providers, not Loft/BAML compatibility
  wrappers.
- `missing required native provider config`: add native bindings for
  `codex-main`, `claude-main`, and `pi-main`.

Use native-surface mode when you only need a provider-specific probe:

```sh
WHIPPLESCRIPT_E2E_REAL_PROVIDERS=1 \
WHIPPLESCRIPT_REAL_PROVIDER_NATIVE_SURFACE=1 \
WHIPPLESCRIPT_REAL_PROVIDERS=codex \
scripts/check-real-providers-report.sh
```

## Destructive Provider Test Is Refused

Destructive provider suites require an explicit disposable target marker and
acknowledgement:

```sh
WHIPPLESCRIPT_REAL_PROVIDER_DESTRUCTIVE_TESTS=1 \
WHIPPLESCRIPT_REAL_PROVIDER_DISPOSABLE_TARGET=native-provider-ci-sandbox \
WHIPPLESCRIPT_REAL_PROVIDER_DISPOSABLE_ACK=I_UNDERSTAND_THIS_PROVIDER_TARGET_IS_DISPOSABLE \
scripts/check-real-providers-report.sh
```

Provider-specific forms are accepted too, such as
`WHIPPLESCRIPT_PI_DESTRUCTIVE_TESTS`,
`WHIPPLESCRIPT_PI_DISPOSABLE_TARGET`, and
`WHIPPLESCRIPT_PI_DISPOSABLE_ACK`. Reports record only whether markers are set,
not their values.

## `cargo install --git` Fails

Use the checkout path first:

```sh
git clone https://github.com/jamesjscully/whipplescript.git
cd whipplescript
cargo install --path crates/whipplescript-cli --locked
```

If that works, the Git install failure is likely a network, lockfile, or remote
toolchain issue.

## `whip doctor` Reports Missing Tools

Some tools are optional. For fixture-backed quickstart and tutorial flows, you
do not need Maude, Apalache, BAML, Codex, Claude, Pi, or Loft.

Install optional tools only when you need formal checks or real provider smoke
tests.

For native provider work, start with:

```sh
scripts/check-native-provider-surfaces.sh
scripts/check-codex-app-server-schema.sh
scripts/check-claude-agent-sdk-surface.sh
scripts/check-pi-rpc-surface.sh
```
