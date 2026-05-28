# E2E Test System

Status: draft

The default e2e suite is deterministic and runs with in-process mock providers:

```sh
scripts/check-e2e.sh
```

It runs the kernel e2e integration tests and CLI control-plane checks. Kernel
e2e tests write trace artifacts to the system temp directory using names like:

```text
whippletree-e2e-<test>-<pid>-trace.txt
```

Those artifacts are written before trace conformance is checked, so a failed
test leaves the abstract lifecycle trace available for debugging.

## Optional Real Providers

Real-provider checks are opt-in:

```sh
WHIPPLETREE_E2E_REAL_PROVIDERS=1 \
WHIPPLETREE_LOFT_TEST_ISSUE=iss_... \
WHIPPLETREE_BAML_TEST_ENDPOINT=http://127.0.0.1:... \
WHIPPLETREE_BAML_TEST_FUNCTION=classifyMessage \
WHIPPLETREE_BAML_TEST_ARGUMENTS_JSON='{"title":"Smoke","body":"Check"}' \
WHIPPLETREE_BAML_TEST_OUTPUT_TYPE=MessageClassification \
scripts/check-real-providers.sh
```

Set `WHIPPLETREE_REAL_PROVIDERS=loft`, `WHIPPLETREE_REAL_PROVIDERS=baml`, or
`WHIPPLETREE_REAL_PROVIDERS=codex` to run a selected subset of no-mock smoke
tests. Comma-separated subsets such as `loft,baml,codex` are accepted. The
default is `loft,baml`.

For the smallest real Codex dogfood check, run:

```sh
scripts/check-codex-message.sh
```

That sends one read-only, non-interactive `codex exec` prompt, requires the
final message and a `turn.completed` JSONL event, and records
`target/codex-message-smoke-report.md`.

For local Loft dogfooding against a sibling checkout, run:

```sh
scripts/check-local-loft-cli.sh ../loft
```

That installs the Loft checkout into `target/loft-cli-venv`, creates an
isolated temporary Loft workspace, exercises create/show/ready/claim/note/
evidence/resource-intent/release through the real CLI, then runs Whippletree's
no-mock `loft.show` smoke through `scripts/check-real-providers-report.sh`.

Optional knobs:

```text
WHIPPLETREE_LOFT_REPO=/path/to/tracked/loft
WHIPPLETREE_LOFT_CLI=/path/to/loft-wrapper
WHIPPLETREE_LOFT_SKIP_REPO_PREFLIGHT=1
WHIPPLETREE_BAML_HEALTH_PATH=/health
```

For local dogfooding with only an OpenAI API key, put `OPENAI_API_KEY` in
`.env` and run:

```sh
scripts/check-openai-coerce.sh
```

That starts `scripts/openai-coerce-server.mjs`, a local BAML-compatible bridge
that serves `/coerce` on `http://127.0.0.1:18765` by default, then runs the same
real-provider Coerce smoke path against it.

Required tools:

```text
loft
baml-cli or baml
```

The OpenAI bridge path sets `WHIPPLETREE_BAML_SKIP_CLI=1`, so it does not require
`baml-cli` for local dogfooding.

To capture the real-provider smoke result as a local artifact while preserving
the underlying check exit code, run:

```sh
scripts/check-real-providers-report.sh
```

The report path defaults to `target/real-provider-smoke-report.md`. Set
`WHIPPLETREE_REAL_PROVIDER_REPORT` to write it elsewhere. The report records
sensitive environment inputs as set/unset rather than values, then includes the
command output for audit.

The real-provider script verifies prerequisite tools, required environment,
Loft fixture repo readiness when Loft is selected, including tracked spec
and fixture files, non-destructive BAML endpoint reachability when BAML is
selected, `doctor`, example compilation, a read-only no-mock `loft.show`
smoke call, a no-mock `baml.coerce` smoke call against the configured endpoint,
and a one-message Codex smoke when `codex` is selected. Provider-destructive
Loft flows stay manual until isolated test fixtures exist for external systems.

Loft fixture shape checks are available separately:

```sh
scripts/check-loft-fixtures.sh
```

The script prefers `WHIPPLETREE_LOFT_FIXTURE_DIR`, then
`vendor/loft/fixtures/whippletree/v0.1`, then the local compatibility fixtures in
`examples/loft-fixtures/v0.1`. It skips only when no fixture source is
available unless `WHIPPLETREE_REQUIRE_LOFT_FIXTURES=1` is set. Set
`WHIPPLETREE_REQUIRE_LOFT_SUBMODULE_FIXTURES=1` to require the source-of-truth
submodule fixture path and reject compatibility-fixture fallback.
