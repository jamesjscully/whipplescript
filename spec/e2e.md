# E2E Test System

Status: draft

The default e2e suite is deterministic and runs with in-process mock providers:

```sh
scripts/check-e2e.sh
```

It runs the kernel e2e integration tests and CLI control-plane checks. Kernel
e2e tests write trace artifacts to the system temp directory using names like:

```text
whipplescript-e2e-<test>-<pid>-trace.txt
```

Those artifacts are written before trace conformance is checked, so a failed
test leaves the abstract lifecycle trace available for debugging.

The deterministic CLI e2e suite includes `examples/provider-language-e2e.whip`
and its acceptance fixture. That workflow drives logical `codex` and `claude`
agents through four table-seeded language-generation tasks, then
reviews every completed turn with typed schema coercion. The default run uses
the fixture worker, so it checks orchestration, dependencies, effect/fact
projection, assertion reads, trace summaries, provider-run/evidence metadata,
and coerce-backend argument rehydration without requiring real provider
credentials.

This workflow is also a language-feature regression target. The intended
source shape is one shared `LanguageTask` schema with a typed
`AgentRef<codex | claude>` provider field, not one duplicate task class
per provider. The test should not ask coerce or any language model to decide
provider identity, model identity, or route selection.

The deterministic suite also includes the companion-skill acceptance fixture.
That workflow seeds three phase-review tasks, routes them through typed
`AgentRef<codex | claude>` metadata, tells each logical reviewer to update
the same visible tracker path, and records
`CompanionReviewDispatch` facts after successful fixture turns. Source
assertions prove one dispatch per logical reviewer and three completed
`agent.tell` effects. The test fails if routing moves into coerce output, prompt
text inspection, or duplicate provider-specific task schemas.

The provider-language example encodes these assertions in WhippleScript source
and `examples/provider-language-e2e.accept.json` pins the final report:

```text
count(LanguageE2EResult where provider == "codex") == 2
count(LanguageE2EResult where provider == "claude") == 2
count(agent.turn.completed) == 4
count(coerce.succeeded) == 4
```

`coerce.succeeded` is the current fixture projection for the coerce-backed
schema-coercion provider. The target semantic projection is
`schema.coerce.succeeded`.

Language/script quality may be reviewed by a schema-coercion backend for local
validation, but exact
script detection can also be tested through a deterministic validator
capability. E2E coverage should include both paths: model-judged review for
coerce integration and non-LLM validation for deterministic CI. The deterministic
path is the `exec "<validator>" -> Schema` form: a non-LLM checker emits a typed
JSON verdict that rules branch on, with no provider access (see
`examples/deterministic-validation.whip` and the Deterministic validation
section of `docs/language-reference.md`).

## Expression Kernel Coverage

The e2e suite is the integration gate for the expression kernel after parser,
validation, and unit-level runtime tests have landed. It should not duplicate
every parser fixture, but it must prove the compiled source path works end to
end for the language forms authors are expected to use.

Required deterministic coverage:

| Area | E2E expectation |
| --- | --- |
| Guard routing | One shared `LanguageTask` schema routes `codex` and `claude` with deterministic `where` guards or typed `AgentRef` fields. |
| Boolean and comparison operators | Source includes `&&`, `||`, `!`, equality, inequality, and numeric ordering in guards or assertions. |
| Membership and presence | Source includes `in`, `not in`, `exists path`, optional-present access, and null/missing-sensitive checks. |
| Projection functions | Source assertions use `count`, `exists(collection)`, and `empty` over fact and effect projections. |
| Map/index paths | At least one guard or assertion reads a string-keyed map index. |
| Pattern branches | Rule-body `case` selects enum/literal, optional `Some`/`None`, and tagged terminal-output union branches before recording facts or effects. |
| Assertion output | Failed assertions produce structured JSON output, nonzero exit, and no state mutation after the failed checkpoint. |
| Guard failure | A false guard does not enqueue effects; a guard error is diagnosable and does not commit a partial rule. |
| Dynamic routing | Dynamic `tell` targets use `AgentRef<...>` values and reject plain strings before provider execution. |

Golden IR fixtures are the validation bridge between parser coverage and e2e
behavior. They should snapshot guards, assertions, projection queries, branch
guards, table rows, typed `record`/effect arguments, dynamic `AgentRef`
targets, map indexes, arrays, and `Missing` versus `null` preserving nodes. The
fixtures should be small, source-stable, and reviewed alongside runtime e2e
changes so implementation cannot silently create a second expression dialect.

## Companion-Skill validation

The companion-skill validation test authors deterministic routing and validation
metadata directly in WhippleScript source:

- provider/model identity is represented by literals, enums, or `AgentRef`
  values supplied by the workflow, not by a coerce classifier or model answer
- provider-language or phase-review tasks are seeded from typed facts with one
  shared task schema; provider-language now uses typed static table rows for
  deterministic fixture data
- companion instructions encourage agents to emit artifacts and trace evidence,
  while source assertions check counts, route coverage, and terminal effect
  states
- source assertions check counts, route coverage, and terminal effect states in
  CI; a deterministic validator capability for exact script/fixture properties
  is realized by the `exec "<validator>" -> Schema` path (a non-LLM checker whose
  typed verdict rules branch on), kept separate from typed schema-coercion review
- assertions prove that all expected provider/language pairs completed and that
  no duplicate provider-specific task classes were needed

This test is considered stale if it asks an LLM to infer provider identity,
duplicates the task schema per provider, or validates exact script membership
only through model judgment.

## Stale E2E Cleanup

As expression-kernel coverage lands, remove or rewrite e2e cases that encode
pre-kernel behavior:

- provider-language fixtures with one task class per provider should become a
  shared schema plus guard or `AgentRef` routing
- Rust-only assertions for provider counts, turn counts, and coerce counts should
  move into source assertions where practical, leaving Rust to check CLI exit
  status and JSON shape
- tests that inspect prompt text to recover route identity should use typed
  correlation metadata, effect projections, or recorded result facts
- older guard string tests should be replaced by typed expression parser,
  golden IR, validation, and runtime/e2e coverage
- model-judged exact-script checks should be paired with deterministic
  validator-capability coverage before being treated as CI signal

## Optional Real Providers

Real-provider checks are opt-in:

```sh
WHIPPLESCRIPT_E2E_REAL_PROVIDERS=1 \
WHIPPLESCRIPT_COERCE_TEST_ENDPOINT=http://127.0.0.1:... \
WHIPPLESCRIPT_COERCE_TEST_FUNCTION=classifyMessage \
WHIPPLESCRIPT_COERCE_TEST_ARGUMENTS_JSON='{"title":"Smoke","body":"Check"}' \
WHIPPLESCRIPT_COERCE_TEST_OUTPUT_TYPE=MessageClassification \
scripts/check-real-providers.sh
```

Set `WHIPPLESCRIPT_REAL_PROVIDERS=coerce` or
`WHIPPLESCRIPT_REAL_PROVIDERS=codex` to run a selected subset of no-mock smoke
tests. Comma-separated subsets such as `coerce,codex` are accepted. The
default is `coerce`.

Set `WHIPPLESCRIPT_COERCE_HEALTH_PATH` to add an optional HTTP health-path probe
(relative to the coerce endpoint) on top of the endpoint TCP reachability check.

For the smallest real Codex provider smoke check, run:

```sh
scripts/check-codex-message.sh
```

That sends one read-only, non-interactive `codex exec` prompt, requires the
final message and a `turn.completed` JSONL event, and records
`target/codex-message-smoke-report.md`.

Destructive provider tests are refused unless the target is explicitly marked
disposable. Set `WHIPPLESCRIPT_REAL_PROVIDER_DESTRUCTIVE_TESTS=1` for all
selected providers, or `WHIPPLESCRIPT_CODEX_DESTRUCTIVE_TESTS=1`,
`WHIPPLESCRIPT_CLAUDE_DESTRUCTIVE_TESTS=1`, or
`WHIPPLESCRIPT_COERCE_DESTRUCTIVE_TESTS=1` for one provider. The matching run must
also set either provider-specific disposable marker variables, such as
`WHIPPLESCRIPT_CODEX_DISPOSABLE_TARGET` and
`WHIPPLESCRIPT_CODEX_DISPOSABLE_ACK`, or the global
`WHIPPLESCRIPT_REAL_PROVIDER_DISPOSABLE_TARGET` and
`WHIPPLESCRIPT_REAL_PROVIDER_DISPOSABLE_ACK`. The acknowledgement value must be
exactly:

```text
I_UNDERSTAND_THIS_PROVIDER_TARGET_IS_DISPOSABLE
```

The earlier local coerce bridge (`scripts/check-openai-coerce.sh` +
`scripts/openai-coerce-server.mjs`, which served a fictional `/coerce` endpoint)
has been removed — no real provider implements that shape. The replacement is
provider-native structured outputs (OpenAI Responses / Anthropic Messages),
a separate credential-gated build (see `spec/coerce.md`). Until it lands, coerce
is validated deterministically through `FakeCoerceClient` under the fixture
provider.

To capture the real-provider smoke result as a local artifact while preserving
the underlying check exit code, run:

```sh
scripts/check-real-providers-report.sh
```

The report path defaults to `target/real-provider-smoke-report.md`. Set
`WHIPPLESCRIPT_REAL_PROVIDER_REPORT` to write it elsewhere. The report records
sensitive environment inputs as set/unset rather than values, then includes the
command output for audit. The underlying readiness script also writes a JSONL
boundary-preflight artifact at `target/real-provider-preflight.jsonl` by default;
set `WHIPPLESCRIPT_REAL_PROVIDER_PREFLIGHT_REPORT` to choose another path. Each
record names the provider, boundary phase, check id, status, and a redacted
message, so config, adapter resolution, workspace preparation, launch, health,
and result-validation failures are visible without scraping free-form output.
The report wrapper also writes per-provider JSON reports under
`target/real-provider-reports/` by default. Set
`WHIPPLESCRIPT_REAL_PROVIDER_REPORT_DIR` to choose another directory. Those
provider reports contain redacted environment posture, evidence refs, check
counts, and the provider's own preflight records.

Native provider compatibility checks can be requested without the all-provider
strict gate by setting `WHIPPLESCRIPT_REAL_PROVIDER_NATIVE_SURFACE=1`. This
selects the native Codex app-server and Claude Agent SDK validation
paths for whichever providers are listed in `WHIPPLESCRIPT_REAL_PROVIDERS`.
The optional GitHub Actions workflow `Native Provider Validation` runs a
Codex/Claude matrix in that mode and uploads the generated reports. Its
`strict=true` dispatch input runs the all-provider strict gate; missing native
provider config paths or required live prerequisites then fail the workflow.

The real-provider script verifies prerequisite tools, required environment,
non-destructive coerce endpoint reachability when coerce is
selected, `doctor`, example compilation, a no-mock schema-coercion smoke call
against the configured endpoint, and a one-message Codex smoke when `codex` is
selected. Provider-destructive flows must pass the disposable-target marker gate
before any provider test is allowed to run.
