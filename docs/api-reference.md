# CLI reference

Implemented CLI commands, their exit behavior, and the compact source
construct index. Runtime JSON contracts live in
[JSON reference](json-reference.md). Internal Rust crate APIs live in
[Rust API reference](rust-api.md). Semantics and examples live in the
[language reference](language-reference.md), [manual](manual.md), and
[runtime & operations](runtime-operations.md).

## Global CLI options

All CLI commands use the same global shape:

```sh
whip [--store path] [--json] [--input JSON] <command> [args]
```

| Option | Meaning |
| --- | --- |
| `--store path` | SQLite store path. Defaults to `.whipplescript/store.sqlite`, or `WHIPPLESCRIPT_STORE` when set. Use `:memory:` for in-memory tests. |
| `--json` | Emit machine-readable JSON where the command supports it. |
| `--input JSON` | Start input for `run` and `dev`. The payload must be keyed by declared workflow input names. |

The current command set is:

```text
package, check, compile, run, revise, step, worker, dev, accept, instances,
status, log, facts, effects, runs, artifacts, inbox, signal, items, leases,
ledger, counters, evidence, diagnostics, trace, otel-export, telemetry, pause,
resume, cancel, retry, recover, doctor
```

Run `whip <command> --help` or `whip help <command>` to print the usage line
for any command.

### Environment variables

| Variable | Meaning |
| --- | --- |
| `WHIPPLESCRIPT_STORE` | Default store path when `--store` is omitted. |
| `WHIPPLESCRIPT_ITEMS_STORE` | Path for the builtin work-queue tracker (defaults to `.whipplescript/items.sqlite`). |
| `WHIPPLESCRIPT_COORDINATION_STORE` | Path for workspace-scoped lease, ledger, and counter state (defaults to `.whipplescript/coordination.sqlite`). |
| `WHIPPLESCRIPT_EXEC_ALLOW` | Dev-profile raw `exec "<command>"` allow-list: colon-separated glob prefixes such as `scripts/*:bin/ci-*`. Commands that do not match fail without running. |
| `WHIPPLESCRIPT_EXEC_PROFILE` | `dev` (default) or `hosted`. Hosted rejects raw exec strings and requires script capabilities. |
| `WHIPPLESCRIPT_SCRIPT_MANIFEST` | JSON manifest path for hosted script capabilities. Equivalent to `--script-manifest`. |
| `WHIPPLESCRIPT_RUN_ID` | Run identity stamped onto items filed by an agent through `whip items add`. |
| `WHIPPLESCRIPT_PROVIDER_CONFIGS` | Colon-separated provider binding config paths for the worker. |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | OTLP/HTTP endpoint for `otel-export` (defaults to `http://localhost:4318`). |
| `OTEL_SERVICE_NAME` | Service name attached to exported OpenTelemetry resource spans. |

## CLI commands

### `doctor`

```sh
whip doctor
whip --json doctor
whip --json doctor --providers
whip --json doctor --provider-config examples/provider-configs/native/native.example.json
```

Opens or creates the configured store, reports schema version, and checks
optional tools:

```text
maude
python3 or python
python3 -c 'import jsonschema'
java
apalache-mc or apalache
coerce-cli or coerce
codex
claude
pi
loft
```

The formal/report helper scripts use Python `jsonschema`. From a checkout, use
`nix develop` or install `requirements-dev.txt` before running generated model
searches or report-schema validation outside the packaged CI environment.

With `--provider-config`, JSON output includes `provider_config_checks`. Each
check contains the config path and redacted validation `results`.
With `--providers`, JSON output includes `provider_health_checks`, a
deterministic non-live posture for Codex, Claude, and Pi. It reports CLI
availability, credential-reference posture, and deeper checks that require
explicit real-provider validation without printing credential values.

### `package`

```sh
whip package catalog
whip package check <manifest.json>...
whip package lock [--output <path>] <manifest.json>...
```

Emits the current platform construct catalog, validates package manifests, and
creates `whipplescript.package_lock.v0` lockfiles. First-class package manifests use
`whipplescript.package_manifest.v0` with separated `libraries`,
`capabilities`, `providers`, `profiles`, and `bindings`. Package validation
derives and validates the normalized library/effect contract
registry. It also checks package-level references: current packages may expose
only `capability.call` effect contracts, required capabilities must be declared
by the package, and bindings must name a provider kind registered for the bound
capability. Metadata-only declaration forms are accepted when they use a
non-reserved keyword, an accepted scope, accepted field kinds, and
`lowering_target: "metadata_only"`; they are reported in the contract registry
for tooling. The accepted executable declaration target is
`lowering_target: "capability_call"` for rule-body forms that name a declared
`target_capability` with a matching `capability.call` effect contract. The
initial executable form is memory `recall from <pool> for <query> as <binding>`,
which requires a package lock authorizing `recall` to lower to the memory recall
capability.
`package catalog` is a manifest-free machine-readable view of the accepted
construct families, lowering classes, scopes, field kinds, interface kinds,
phases, cardinalities, and reserved keywords.

`package lock` pins each manifest by exact SHA-256. `check`, `compile`, `run`,
`dev`, and `worker` can load the lock with `--package-lock <path>`. When that flag
is omitted, each command discovers a `whip.lock`: an explicit `--package-lock`
wins, otherwise it searches from the workflow file's directory upward, otherwise
from the current directory upward. A project with a `whip.lock` at its root
therefore needs no explicit path even when run from elsewhere. If several source
files on one command line resolve to different locks, the command fails and asks
for `--package-lock`. If no lock is given and none is discovered, any non-`std.`
package import (for example `use memory`) or package construct use (for example
`recall`) fails with a diagnostic that names the blockers and suggests
`whip package sync`; pure `std.` programs are unaffected.
Each locked manifest is pinned by a SHA-256 recomputed over the manifest bytes at
load time, so editing a manifest after locking fails lock load with the stable
`package_lock` error kind (in `--json` output); re-run `package lock` or
`package sync` to repin. Runtime commands also enforce locked package output
contracts. A package capability effect with `validation: runtime_boundary` must
return a value that matches its `output_schema`; otherwise the run fails before
any package success fact is derived.

### `check`

```sh
whip check [--model-search] [--root Workflow] \
  [--exec-profile dev|hosted] [--script-manifest <path>] \
  [--package-lock <path>] \
  <workflow.whip>...
whip --json check [--model-search] [--root Workflow] \
  [--exec-profile dev|hosted] [--script-manifest <path>] \
  [--package-lock <path>] \
  <workflow.whip>...
```

Parses, resolves includes, type-checks, lowers to IR, enforces the
[liveness checks](language-reference.md#liveness-checks), and prints the IR
snapshot. With `--model-search`, also runs generated Maude checks when
available. The generated artifact bridge checks require Maude, Python, and the
Python `jsonschema` package.

With `--exec-profile hosted`, raw `exec "..."` is a check error and named
`exec <capability> with <record>` forms must resolve in the supplied script
manifest.

With `--package-lock`, imported package libraries such as `use memory` resolve
against the pinned package manifest and appear in `contract_registry`.

JSON output is an array with one report per input path. Successful entries
include source hashes, the IR snapshot, and `source_metadata`:

```json
[
  {
    "schema": "whipplescript.check_report.v0",
    "path": "examples/provider-language-e2e.whip",
    "status": "ok",
    "workflow": "ProviderLanguageE2E",
    "source_hash": "...",
    "ir_hash": "...",
    "snapshot": "...",
    "source_metadata": {
      "tags": [
        {"name": "fixture", "target_kind": "workflow", "target": "ProviderLanguageE2E"}
      ],
      "descriptions": [
        {
          "value": "Static provider x language task rows",
          "target_kind": "table",
          "target": "language_tasks"
        }
      ],
      "targets": {
        "workflow:ProviderLanguageE2E": {
          "target_kind": "workflow",
          "target": "ProviderLanguageE2E",
          "tags": ["fixture", "acceptance"],
          "description": "Fixture-backed provider x language acceptance workflow"
        }
      }
    }
  }
]
```

Diagnostic entries use `"status": "error"` and include structured source spans.

Exit behavior:

| Exit | Meaning |
| --- | --- |
| `0` | All inputs compile and optional model searches pass. |
| `1` | Diagnostics or generated checks failed. |
| `2` | CLI usage error. |

### `fmt`

```sh
whip fmt <workflow.whip>...           # format in place
whip fmt --check <workflow.whip>...   # report unformatted files, exit non-zero
```

Formats WhippleScript source to the canonical layout. Formatting is idempotent:
running `fmt` on already-formatted source makes no change.

v0 limitations (both strictly non-destructive — `fmt` reports an error and leaves
the file untouched rather than damaging it):

- **Comments are preserved** across every declaration: own-line comments above a
  top-level declaration (or a file header) and **trailing** comments on a
  single-line declaration (`workflow Demo  # ...`); comments inside a `rule`/
  `apply`/`coerce`/`table`/`flow` body; and — inside
  `class`/`agent`/`enum`/`signal`/`queue`/`file store` bodies (including a
  data-carrying `enum` variant's nested field block) — both own-line comments
  (interleaved by source position) and trailing comments on a field/clause line.
  The only comments `fmt` cannot place are ones with nowhere to attach — e.g. a
  comment trailing a declaration's opening-brace line (`class Task {  # ...`), with
  no field on that line; those cause `fmt` to **refuse the file rather than drop the
  comment**. A no-loss check guarantees no comment is ever silently lost.
- Bodies — `rule`/`apply`/`coerce` (including multi-line `"""..."""` strings and
  nested `record`/`complete` blocks, with string content preserved) and `table`
  rows — format idempotently. As a safety net `fmt` also **self-checks
  idempotency** and refuses any file it cannot format stably (rather than writing
  drifting/corrupting output), so even an uncovered construct is never damaged.

`--check` exits non-zero if any input is unformatted (or refused) and writes
nothing.

### `lint`

```bash
whip lint <source-or-dir>... [--root <workflow>]
whip --json lint <source-or-dir>...              # structured findings
whip lint --rule <id> <source-or-dir>...         # run only the named rule(s)
whip lint --deny <id> --allow <id> <source-or-dir>...   # configure actions
```

A positional may be a file or a directory (discovered recursively for `.whip` files,
like `whip test`). `--rule <id>` (repeatable) restricts the run to the named rules;
with none given, every rule runs. With a single source the JSON report is `{schema,
path, findings}`; with several it is `{schema, reports: [{path, findings}, …]}`.

Static-analysis warnings over a program that already compiles. Lint is a superset
of `check` for errors: a program that fails to compile reports the compile errors
(non-zero exit). Beyond errors it surfaces quality findings.

Unused-declaration analyses flag a construct that is only usable from within the
program but is never referenced (so it is unambiguously dead — no false positives):

- **`lint.unused_coerce`** — a `coerce` function declared but never called.
- **`lint.unused_lease`** — a `lease` declared but never acquired.
- **`lint.unused_ledger`** — a `ledger` declared but never appended to.
- **`lint.unused_counter`** — a `counter` declared but never consumed.
- **`lint.unused_queue`** — a `queue` declared but never filed into or claimed.
- **`lint.unused_file_store`** — a `file store` declared but never read or written.
- **`lint.unused_class`** — a `class` declared but referenced nowhere.
- **`lint.unused_enum`** — an `enum` declared but referenced nowhere.

Other analyses:

- **`lint.noop_rule`** — a rule whose body is empty: it fires but produces no
  record, effect, `done`, or terminal (a forgotten or half-written body).
- **`lint.coerce_result_unused`** — a `coerce <fn>(…) as <binding>` call whose result
  `<binding>` is never used: the coercion runs but its result is discarded (dead work
  or a forgotten `after <binding>` handler).
- **`lint.broad_file_grant`** — a `file store` grant whose `read`/`write` glob matches
  everything under the root (`**` / `**/*`): broader than any concrete call needs.
- **`lint.deep_after_nesting`** (`info`) — a rule nesting `after` blocks ≥4 levels
  deep; a long effect chain reads more clearly as a `flow`.
- **`lint.tool_grant_requires_owned_harness`** — an agent with a `tools [...]`
  grant (DR-0025) that does not use the owned harness (`provider owned`, or a
  harness of kind `owned`); the grant is dead because sub-workflow tools are only
  resolved and offered in the owned brokered loop.

Every finding resolves to the source span of the declaration it concerns. The text
output prefixes each finding with `:line:col` (1-based) and `--json` emits one finding
per entry with `code`, `severity`, `default_severity`, `configured_action`, `message`,
and a `range` (`start`/`end` with 0-based `line` + UTF-16 `character`, matching LSP)
— schema `whipplescript.lint.v0`.

**Configured actions.** Each rule's *action* (a separate axis from its severity) is
resolved from `--allow <id>`/`--deny <id>` over a project `whip.lint.json` over the
`warn` default — CLI wins, then config, then default:

- **`allow`** — the finding is suppressed (not emitted).
- **`warn`** — the finding is reported; the run still succeeds (the default).
- **`deny`** — the finding is reported and the run exits non-zero.

A project `whip.lint.json` next to the program sets per-rule actions:

```json
{
  "schema": "whipplescript.lint_config.v0",
  "rules": { "lint.unused_class": "deny", "lint.noop_rule": "allow" }
}
```

An invalid config (bad schema or action) is a `lint.internal` error (non-zero exit).

### `lsp`

```sh
whip lsp     # a Language Server over stdio, launched by an editor
```

A minimal [Language Server](https://microsoft.github.io/language-server-protocol/)
spoken over stdio (hand-rolled JSON-RPC; no extra runtime dependency). v0 provides:

- **Diagnostics on edit** — on `textDocument/didOpen` and `didChange` it
  re-compiles the document (full-text sync) and publishes
  `textDocument/publishDiagnostics`, the same parse/validation diagnostics
  `whip check` produces, as live editor squiggles. When the document compiles, lint
  findings are also published as diagnostics tagged `whip lint` (at each finding's own
  severity — the canonical `error`/`warning`/`info`/`hint` set maps 1:1 to LSP
  severities), each pointing at the offending declaration. `didClose` clears them.
- **Document symbols** — `textDocument/documentSymbol` returns the top-level
  declarations (workflow, classes, agents, rules, signals, coerces, coordination
  resources, …) for an editor outline/breadcrumb view.
- **Go to definition** — `textDocument/definition` resolves the identifier under
  the cursor to its top-level declaration. Top-level names are program-unique, so a
  reference (e.g. `when Ticket`, a `coerce` call, a `signal` name) jumps to its
  declaration; local bindings resolve to nothing for now.
- **Hover** — `textDocument/hover` shows the declaration source for the symbol
  under the cursor (so hovering a reference reveals the target's definition).
- **Completion** — `textDocument/completion` offers language keywords plus the
  document's declared top-level names (the editor filters by the typed prefix);
  context/scope-aware filtering is future work.
- **Find references** — `textDocument/references` lists every occurrence of the
  top-level symbol under the cursor (honoring `includeDeclaration`).
- **Rename** — `textDocument/rename` renames a top-level symbol across the
  document, editing every code occurrence but **not** the same word inside a prompt
  string or comment (so content is never corrupted).
- **Formatting** — `textDocument/formatting` formats the document with the same
  comment-preserving formatter as `whip fmt` (returns no edits if the document does
  not parse or `fmt` would refuse it, so content is never corrupted).
- **Document highlight** — `textDocument/documentHighlight` marks every occurrence
  of the symbol under the cursor (the editor's always-on cursor highlighting).
- **Workspace symbols** — `workspace/symbol` searches symbols across every open
  document by a case-insensitive substring query (an empty query returns all). v0
  indexes the documents the editor has opened; filesystem-wide indexing will gate on
  the shared symbol-index service below.

Cross-file/scope-aware navigation (multi-document references, local bindings,
filesystem-wide symbols) is planned and will gate on a shared symbol-index service.

### `test`

```sh
whip test <workflow.whip|dir>...               # files and/or directories
whip --json test <workflow.whip|dir>...        # emit whipplescript.test_report.v0
whip test <workflow.whip|dir>... --list        # enumerate selected scenario ids, run none
whip test <workflow.whip|dir>... -i <pattern>  # include only matching scenarios (repeatable)
whip test <workflow.whip|dir>... -x <pattern>  # exclude matching scenarios (repeatable)
whip test <workflow.whip|dir>... --pass-if-no-tests
```

Runs the `test "…" { workflow … given … stub … run … expect … }` scenarios
declared in the given programs, each on an isolated store (spec/workflow-testing.md).
Each positional is a `.whip` file or a directory; a directory is discovered
recursively for `.whip` files (hidden entries and `target/` are skipped). All sources
are compiled and their scenarios aggregated into one report; each scenario runs against
its own program text. Every discovered file must compile — a malformed source is exit 2,
never a silent skip.

**Scenario ids.** Each scenario is identified as `<workflow>::<name>` — the workflow
is the scenario's `workflow` clause, or the program's root workflow — both in the
report's `id` field and for selection.

**Selection.** `-i`/`--include` and `-x`/`--exclude` filter scenarios by that id. A
pattern containing `::` matches the two halves independently — an empty side matches
anything, so `Sel::` selects a whole workflow and `::*passes` selects by test name
across workflows — while a pattern without `::` matches the test name alone. `*` is
the only wildcard and matches any run of characters. Includes are OR'd (no `-i` means
all); excludes override includes. `--list` prints the selected ids (one per line, or
a JSON `tests` array under `--json`) without running them.

**Exit codes** (per spec/workflow-testing.md): `0` all selected scenarios passed ·
`1` a scenario ran and failed an expectation · `2` setup is invalid (a compile
error, or a scenario the harness cannot run faithfully) · `4` no scenarios were
selected — unless `--pass-if-no-tests` downgrades that to `0`.

The driver runs the scenario in rounds — each round drains rule evaluation to idle
(what `whip step` does) and then settles queued effects (including agent turns)
through the deterministic **fixture** provider. The `run` clause controls how many
rounds: `run until idle` (or `until workflow completed|failed`) drives to a fixed
point — until the workflow reaches a terminal state or stops making progress —
while `run for N steps` runs exactly N rounds (stopping early only if the workflow
turns terminal), which lets a test inspect an intermediate state. The
`stub <surface> <outcome>` clause selects the fixture outcome: `succeeds` or
`fails`. So a workflow that tells an agent and reacts to `completed turn` runs
end-to-end under `stub agent <name> succeeds`. (`times_out`/`cancels` are rejected
as unsupported — the fixture agent path is a shell-command harness that can only
exit 0/non-zero, so it cannot faithfully simulate a timeout, and the harness will
not silently pretend the turn succeeded.)

v0 driver scope: `workflow <Name>` header; `given signal`, `given input`
(validated against the workflow's input contract and seeded as the declared input
fact), `given fact <Type> { … }` (a pre-existing fact), `given clock at
"<timestamp>"` (inject a virtual evaluation clock so `timer until`/`timeout`
deadlines fire — or stay pending — deterministically and without a real sleep),
`given tracker <name> issue { … }` (seed an existing issue into the builtin tracker,
isolated per scenario, surfaced through the real `queue.item.ready` projection),
`given file <store> at "<path>" "<content>"` (seed a deterministic fixture file
into a declared `file store`, isolated per scenario in a temp dir the store root
is redirected to, so a `read` runs through the real worker against the fixture);
`stub agent <name>
succeeds|fails` and `stub coerce <fn> returns { … }` (inject the typed coerce
output a workflow branches on); `run until idle|workflow completed|failed` and
`run for N steps`; `expect workflow completed|failed`,
`expect rule <name> fired|did not fire|fired N times`, effect expects
`expect effect <kind> requested|completed|failed` and `expect no <kind>` (over the
settled effect log, matched by effect kind such as `agent.tell`), fact projection
expects `expect <fact> exists | where <pred> | count where <pred> is N` — where
`<fact>` is a dotted fact name, so it can target runtime facts such as
`agent.turn.completed` as well as single-identifier user facts — and
`expect diagnostic <code>` (a runtime diagnostic recorded during the run, matched
by code). Every `expect` target is evaluated.

Honest by construction: a scenario the driver cannot run faithfully — an
unsupported stub outcome, or a `given input` that violates the input contract —
is reported as **invalid**
with the reason in the scenario's `diagnostics`, never a pass. The runner never
silently skips an assertion. (Stubbing different agents differently in one
scenario — e.g. `stub agent alpha succeeds` + `stub agent beta fails` — is
supported.)

JSON output is `whipplescript.test_report.v0`: a top-level `status`
(`passed | failed | invalid | no_tests`) and `summary`
(`selected/passed/failed/invalid/skipped`), plus one object per scenario with its
`id`, bound `workflow`, `steps` (the `given`/`stub`/`run` clauses), `expectations`
(each `expect` with a `passed`/`failed` status), and `diagnostics` (failure and
harness-error detail).

See `examples/tested-agent-turn.whip` for a worked example: a workflow plus three
scenarios exercising `given input`, `stub … succeeds|fails`, `run until idle` /
`run for N steps`, and the workflow/rule/effect/projection/diagnostic expects.

#### `test replay`

```sh
whip test replay <instance-id>
whip --json test replay <instance-id>
```

Regression-debugging tool (outside the scenario syntax): replays a recorded
instance's event log into a throwaway copy of the `--store` and checks that the
reconstructed projection is byte-identical to the live-built one — the
**replay-equality** invariant (spec/workflow-testing.md). The canonical projection
is the instance's terminal status plus its active facts and effects, with volatile
fields (event/fact/effect ids, timestamps, revision epochs) excluded and arrays
sorted, so the comparison is order- and id-independent. The user's store is never
mutated (the rebuild runs on the temp copy). Exit `0` = equal, `1` = diverged
(the JSON report then carries both `recorded` and `replayed` projections), `2` =
setup error (unknown instance or unreadable store).

This operates on a `--store` instance rather than a standalone trace/event-log
file + `--workflow`; a portable file-based trace format is a future extension.

### `compile`

```sh
whip compile <workflow.whip> [--root Workflow] [--package-lock <path>]
whip --json compile <workflow.whip> [--model-search] [--root Workflow] [--package-lock <path>]
```

Prints the compiled IR snapshot. JSON output includes:

```json
{
  "schema": "whipplescript.compile_report.v0",
  "path": "examples/minimal-noop.whip",
  "workflow": "MinimalNoop",
  "source_hash": "...",
  "ir_hash": "...",
  "snapshot": "...",
  "source_metadata": {
    "tags": [],
    "descriptions": [],
    "targets": {}
  }
}
```

With `--model-search`, JSON compile reports also run the generated Maude checks
over the emitted IR, construct graph, and lowered IR report. This option
requires `--json`; non-JSON `compile` keeps stdout reserved for the IR snapshot.
The generated artifact bridge checks require Maude, Python, and the Python
`jsonschema` package. The CLI passes its current platform construct catalog to
the bridge scripts with `--platform-catalog`, so inherited
`WHIPPLESCRIPT_PLATFORM_CATALOG_PATH` settings do not affect generated checks.
Standalone bridge-script runs must pass `--platform-catalog <path>` or bind the
same catalog through `WHIPPLESCRIPT_PLATFORM_CATALOG_PATH`.

### `run`

```sh
whip [--store path] [--input JSON] run <workflow.whip> \
  [--root Workflow] [--package-lock <path>]
```

Compiles the source bundle, creates a program version if needed, creates an
instance, appends `external.started`, and seeds declared workflow input facts.
It does not run ready rules or providers. With `--package-lock`, locked package
manifests are registered into the store before the instance starts.

JSON output:

```json
{
  "instance_id": "inst_...",
  "program_id": "prg_...",
  "version_id": "ver_...",
  "workflow": "WorkflowName",
  "store": ".whipplescript/store.sqlite"
}
```

### `step`

```sh
whip [--store path] step <instance> --program <workflow.whip> [--root Workflow]
```

Runs deterministic rule evaluation for one instance until no further rule commit
is possible. It may create facts, consume facts, enqueue effects, add dependency
edges, and execute workflow terminal actions. It never executes providers.

Human output:

```text
step <instance> committed_rules=N facts=N consumed=N effects=N
```

JSON output includes:

```json
{
  "instance_id": "inst_...",
  "committed_rules": 1,
  "facts_created": 1,
  "facts_consumed": 0,
  "effects_created": 2,
  "guards": [],
  "branches": []
}
```

### `worker`

```sh
whip [--store path] worker <instance> \
  [--provider fixture] \
  [--provider-config <path>] \
  [--exec-profile dev|hosted] \
  [--script-manifest <path>] \
  [--package-lock <path>] \
  [--program <workflow.whip>] \
  [--root Workflow] \
  [--once] \
  [--fail | --timeout | --cancel] \
  [--max-child-iterations N]
```

Starts currently claimable effects and completes them through the selected
provider. The default provider is the deterministic fixture provider.
`--provider-config <path>` can be repeated to bind source harness ids to
concrete provider configs; worker also reads colon-separated
`WHIPPLESCRIPT_PROVIDER_CONFIGS`. `--fail`, `--timeout`, and `--cancel` force
fixture terminal outcomes for failure-path tests.

Hosted script execution uses `--exec-profile hosted --script-manifest <path>`
or `WHIPPLESCRIPT_EXEC_PROFILE=hosted` plus
`WHIPPLESCRIPT_SCRIPT_MANIFEST=<path>`. The worker registers `script.<name>`
capabilities for the instance program, verifies SHA-256 before spawn, and
runs argv-direct with JSON stdin.

With `--package-lock`, the worker registers locked package manifests before
claimable-effect policy checks.

Supported fixture effect kinds:

```text
agent.tell
coerce
human.ask
capability.call
workflow.invoke
queue.file
queue.claim
queue.release
queue.finish
timer.wait
exec.command
signal.emit
lease.acquire
lease.release
ledger.append
counter.consume
```

JSON output includes:

```json
{
  "instance_id": "inst_...",
  "provider": "fixture",
  "ran_effects": 1,
  "terminal_events": ["evt_..."]
}
```

### `dev`

```sh
whip [--store path] [--input JSON] dev <workflow.whip> \
  [--root Workflow] \
  [--provider fixture] \
  [--provider-config <path>] \
  [--exec-profile dev|hosted] \
  [--script-manifest <path>] \
  [--package-lock <path>] \
  [--until idle] \
  [--max-iterations N] \
  [--include-tag TAG] \
  [--exclude-tag TAG] \
  [--stream ndjson] \
  [--fail | --timeout | --cancel]
```

Convenience local validation loop. It starts a new instance, alternates `step`
and `worker`, stops when idle or when `--max-iterations` is reached, then
evaluates source assertions. The text summary reports the final instance outcome
(`status completed`/`failed`/`cancelled`, or — when still `running` and idle — the
reason: a pending human ask with the `whip inbox answer …` command, or a pointer to
`whip status` for blocked effects and failures). `--provider-config <path>` can be repeated and is
passed to the embedded worker loop. `--include-tag <tag>` and `--exclude-tag
<tag>` can be repeated to select which source assertions are evaluated and
reported; they do not skip rules, effects, providers, or table seeding.
Exclusion takes precedence when both filters match an assertion.

`--exec-profile hosted --script-manifest <path>` applies the hosted script
capability checks before the instance starts and passes the same manifest to
the embedded worker.

`--package-lock <path>` applies package resolution before the instance starts
and passes the same lock to the embedded worker loop.

`--stream ndjson` emits compact line-delimited JSON progress envelopes with
schema `whipplescript.dev_stream.v0`. Current events are `dev.started`,
`dev.events`, `dev.step`, `dev.worker`, `dev.idle`, `dev.assertions`, and
`dev.report`. `dev.events` carries batches of newly persisted raw runtime events
using the same object shape as `log --json`. `dev.assertions` carries the compact
executable-spec assertion summary; the final `dev.report` line embeds the same
`whipplescript.dev_report.v0` object as `dev --json`.

JSON output includes the instance id, workflow name, `source_metadata`,
per-iteration step reports, worker reports, durable diagnostics for the dev
instance, compact `provider_runs`, `provider_artifacts`, and
`provider_evidence` summaries, an `executable_spec` assertion summary grouped
by source tag, assertion filter counts, and assertion reports.
`provider_artifacts` groups metadata by artifact kind and MIME type and
includes compact artifact item links without exposing artifact paths or
content. `provider_evidence` groups evidence metadata by kind and subject type
and includes compact evidence item links without exposing evidence metadata
payloads.
Assertion reports include `target_id`, source tags, and any assertion
description, plus `event_id` links to the durable assertion event,
`diagnostic_ids` links for failed or errored assertions, and
deterministic fact/effect `reads` so acceptance reports can group checks by
source metadata and the projections they validate. Each read includes
`match_count` and concrete fact/effect match ids where available. Effect
matches include `prompt_content_type` when the effect input preserved an
annotated multiline prompt. Acceptance report assertion-read summaries also
include compact trace/evidence item counts for grouped effect matches.

### `accept`

```sh
whip [--store path] [--json] accept <fixture.json>
```

Runs a test-only acceptance fixture through the same local `dev` control-plane
path and validates the final report. Fixtures use schema
`whipplescript.acceptance_fixture.v0`; reports use
`whipplescript.acceptance_report.v0`, include `observed.summary` totals plus
grouped final fact/effect count summaries, compact observed provider-run,
artifact-link, evidence-link, control-action, source-metadata, assertion-read,
diagnostic, trace, inbox, and executable-spec summaries, and the full
`whipplescript.dev_report.v0` under `dev_report`. Relative `workflow` and
`provider_config_paths` entries resolve from the fixture file directory.
Fixtures can assert diagnostics by code, executable-spec summaries, tagged and
untagged executable-spec groups, deterministic assertion reads and match
metadata such as prompt content type plus trace/evidence link counts, fixture
action counts, final fact/effect totals, grouped final fact/effect counts,
source metadata targets, provider run counts, metadata-only artifact counts,
metadata-only evidence counts, and human inbox item counts. Observed
assertion-read match groups include compact `trace_sequences` and `evidence_ids`
links for drilldown. The observed trace summary reports event totals,
reconstructed abstract trace event groups, compact abstract trace items, and
conformance; fixtures can assert trace summary fields, groups, and stable item
selectors through `expect.trace`. `expect.assertion_reads` entries must include
at least one selector: `source`, `kind`, `head`, or `guard`.
The v0 command accepts one fixture path; external suite runners should isolate
stores per fixture.

Fixtures may also provide `setup.facts` entries with a declared class `name`,
optional stable `key`, and JSON `value`. These setup facts are validated against
the workflow class schema and derived into the started instance before setup
actions and before the normal dev loop. `setup.inbox` can create pre-existing
human review items with a prompt, status/severity, choices, and related link
arrays before the normal dev loop. `setup.effects` and `setup.artifacts` are
rejected in v0; effects and artifacts must be produced through ordinary rules,
workers, and providers. Fixture `actions` can apply real `pause`, `resume`, or
`cancel` control-plane transitions before the dev loop. Fixture and expectation
fields are shape-checked before the workflow starts so wrong-typed expectations
are rejected rather than treated as absent.

### `revise`

```sh
whip [--store path] revise <instance> <workflow.whip> \
  [--root Workflow] \
  [--dry-run] \
  [--cancel keep|queued|running]
```

Checks whether a candidate source bundle can become the active program version
for a non-terminal running instance. With `--dry-run`, it reports compatibility
without changing the store. Without `--dry-run`, it records a revision
activation event and future `step` calls use the new active program version.

Cancellation policy controls old-version effects:

| Policy | Meaning |
| --- | --- |
| `keep` | Keep old-version effects claimable/runnable. |
| `queued` | Terminal-cancel queued, blocked, and claimable old-version effects. |
| `running` | Cancel queued old-version effects and request cancellation for running old-version work. |

Running cancellation requests are not terminal results. Providers still record
the eventual completion, failure, timeout, or cancellation acknowledgement.

JSON dry-run output includes the candidate version, compatibility diagnostics,
agent impact, cancellation impact, and no activation event. Activation output
includes the activated version, revision epoch, cancellation policy, diagnostics,
and evidence links.

### Inspection commands

| Command | Meaning |
| --- | --- |
| `instances` | List all instances in the configured store. |
| `status <instance>` | Show instance status, counts, recent events, pending time effects, any pending human asks (with the `whip inbox answer …` command to unblock them), and workflow invocation links. |
| `log <instance>` | Show append-only event log. |
| `facts <instance>` | Show current unconsumed facts. |
| `effects <instance>` | Show effects, status, target, profile, and block reason. |
| `runs <instance>` | Show provider run attempts. |
| `evidence <instance>` | Show evidence records and evidence links. |
| `diagnostics <instance>` | Show durable diagnostics. |
| `trace <instance> [--check]` | Show trace bundle; with `--check`, reconstruct abstract trace and run conformance checks. |

All inspection commands support `--json`.
Facts seeded from `table` declarations appear in JSON with
`"provenance_class": "table"` and a `source_span` whose `construct` is
`"table_row"`.

### Inbox commands

```sh
whip inbox [<instance>]
whip inbox show <item>
whip inbox answer <item> --choice <value> [--by NAME]
whip inbox answer <item> --text <value> [--by NAME]
```

Inbox commands inspect and answer human review requests created by `human.ask`
effects. Bare `whip inbox` lists pending items; `whip inbox <instance>`
filters pending items to one instance.

### `signal`

```sh
whip [--store path] [--json] signal <instance> \
  --name <signal.name> \
  --data <json> \
  --program <workflow.whip> \
  [--root Workflow]
```

Validates an external signal payload against the source bundle's declared
`signal <name> { ... }` schema, appends the signal fact to the instance log, and
derives the typed fact that rules match with `when <signal.name> as x` or the
general `when fact <signal.name> as x` form. A malformed or undeclared signal is
rejected before an ill-typed fact can land.

Human output prints the signal sequence. JSON output includes:

```json
{
  "instance_id": "inst_...",
  "signal": "deploy.finished",
  "signal_id": "sig_...",
  "fact_id": "fact_..."
}
```

Exit behavior:

| Exit | Meaning |
| --- | --- |
| `0` | Signal accepted and typed fact derived. |
| `1` | Store, source, or payload validation failed. |
| `2` | CLI usage error. |

### Items commands

```sh
whip items add --queue <Q> --title <T> [--body <B>] [--label <L>]
whip items list [--queue <Q>] [--status <S>]
whip items show <id>
```

Items commands operate the builtin work-queue tracker (see
[work queues](language-reference.md#work-queues)). The builtin tracker is
workspace-scoped, stores items in `.whipplescript/items.sqlite` (override with
`WHIPPLESCRIPT_ITEMS_STORE`), and issues sequential ids `WS-1`, `WS-2`, and so
on. `--status` filters on the item status categories `open`, `in_progress`,
`done`, and `cancelled`. When an agent files an item mid-turn through
`whip items add`, the new item carries run-identity provenance taken from the
`WHIPPLESCRIPT_RUN_ID` environment variable.

### Coordination inspection

```sh
whip [--json] leases [<resource>]
whip [--json] ledger [<ledger>] [--partition <value>]
whip [--json] counters [<counter>]
```

These commands inspect workspace-scoped coordination state created by
`lease`, `ledger`, and `counter` declarations and their corresponding
`acquire`, `release`, `append`, and `consume` effects. They read
`.whipplescript/coordination.sqlite` by default, or
`WHIPPLESCRIPT_COORDINATION_STORE` when set. They do not use the instance
`--store`; coordination resources intentionally outlive disposable run stores.

JSON output shapes are documented in [JSON reference](json-reference.md).

### Lifecycle commands

```sh
whip pause <instance>
whip resume <instance>
whip cancel <instance>
whip retry <instance> <effect>
whip recover <instance>
```

| Command | Meaning |
| --- | --- |
| `pause` | Transition a running instance to paused. |
| `resume` | Transition a paused instance back to running. |
| `cancel` | Transition a running or paused instance to terminal cancelled. |
| `retry` | Move an eligible failed or timed-out effect back to queued. |
| `recover <instance>` | Reconcile interrupted native provider runs from persisted provider evidence. |

Terminal instances are absorbing: completed, failed, and cancelled instances do
not accept further public lifecycle transitions or rule commits.

### `otel-export`

```sh
whip [--store path] otel-export <instance> [--dry-run]
```

Tails terminal provider runs from the instance store and emits OTLP/HTTP JSON
spans. The event log and provider-run tables are the buffer; a cursor file next
to the SQLite store records exported run ids so repeated exporter passes are
emit-once. `--dry-run` prints the payload and does not write the cursor.

Configuration:

| Variable | Meaning |
| --- | --- |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | Collector endpoint; defaults to `http://localhost:4318`. |
| `OTEL_SERVICE_NAME` | Resource service name; defaults to `whipplescript`. |

Exit behavior:

| Exit | Meaning |
| --- | --- |
| `0` | Export succeeded, nothing was new, or dry-run printed payload. |
| `1` | Store read, HTTP export, or cursor persistence failed. |
| `2` | CLI usage error. |

### `telemetry`

```sh
whip [--store path] [--json] telemetry status
whip [--store path] [--json] telemetry reset-cursor [<instance>]
```

Manages the `otel-export` emit-once cursor. `status` reports the exporter endpoint
and service name plus the per-instance count of runs already exported.
`reset-cursor` clears the cursor — for one `<instance>` or, with no argument, all of
it — so the next `otel-export` re-sends from the start. Neither command touches
workflow execution.

## Language reference index

For examples and semantics, see [Language Reference](language-reference.md).
This section is a compact index of source constructs.

### Top-level constructs

| Construct | Surface | Meaning |
| --- | --- | --- |
| Workflow | `workflow Name { ... }` or `workflow Name` | Deployable runtime boundary. |
| Contract | `input name Type`, `output name Type`, `failure name Type` | Typed workflow input/output/failure contract. |
| Include | `include "path.whip"` | Source bundle composition. |
| Package/library import | `use memory` | Import package library surface by name. |
| Class | `class Name { field Type }` | Typed fact and payload schema. |
| Enum | `enum Name { A B }` | Finite string domain. |
| Signal | `signal deploy.finished { field Type }` | Typed external signal ingress schema. |
| Harness | `harness coder: codex` | Named provider endpoint family used by `agent ... using coder`. |
| Agent | `agent name { profile "..."; capacity N; skills [...] }` | Logical provider target and policy metadata. |
| Coerce | `coerce fn(args...) -> Type { prompt """markdown ... """ }` | Declared coerce-backed effect. |
| Flow | `flow name when ... { step; step; ... }` | A rule whose body is a multi-step sequence; lowers to `flow.<name>.seg<N>` rules. |
| Queue | `queue name { tracker builtin }` | Declared vendor-neutral work-item backlog. |
| Lease | `lease name { key Type slots N ttl 10m }` | Workspace-scoped bounded mutex/semaphore resource. |
| Ledger | `ledger name { entry Type partition by field retain 90d }` | Workspace-scoped append log partitioned by a typed field. |
| Counter | `counter name { key Type cap N reset daily }` | Workspace-scoped consumable budget with lazy reset. |
| Pattern | `pattern Name<T> { ... }` | Compile-time reusable fragment. |
| Apply | `apply Name<Type> as Alias { ... }` | Pattern specialization. |
| Assertion | `assert expression` | Deterministic projection check in `dev`. |

### Rule constructs

| Construct | Surface | Meaning |
| --- | --- | --- |
| Rule | `rule name ... => { ... }` | Atomic deterministic rewrite. |
| Fact match | `when Class as binding` | Bind an unconsumed fact. |
| Guarded match | `when Class as binding where expr` | Bind fact only when pure guard is true. |
| Started event | `when started` | Match the initial `external.started` event. |
| Readiness | `when Class as item` or `when { ... }` | Match facts and other deterministic rule conditions. |
| Availability | `worker is available` inside a `when` clause/group | Match logical agent capacity/policy availability. |
| Human answer | `when human answered <label> as x` | Match a `human.answer.received` fact created when an inbox item is answered. The binding payload exposes `choice`, `text`, `answered_by`, `prompt`, `inbox_item_id`, and `effect_id`. |
| Agent turn | `when <agent> completed turn ... [as x]` | Match an `agent.turn.completed` fact. A declared agent name filters to that agent's turns; the generic word `worker` matches any agent. |
| Queue readiness | `when <queue> has ready item as x` | Match an item that is ready to be claimed in a work queue. |
| Declared signal | `when deploy.finished as x` | Match a typed external signal fact declared with `signal deploy.finished { ... }`. |
| General fact | `when fact <dotted.name> as x [where ...]` | General readiness form; the English phrases above are sugar over it. |

### Rule body operations

| Operation | Effect/commit output |
| --- | --- |
| `record Class { ... }` | New fact. |
| `record Class from binding { ... }` | New fact with copied fields. |
| `done binding` | Mark matched fact consumed. `consume binding` is a deprecated alias; the checker now emits a warning for it. |
| `done binding -> record ...` | Consume and create replacement fact atomically. |
| `tell agent ... [timeout <dur>] as turn` | `agent.tell` effect. |
| `prompt "..." [using provider] as result` | Provider-backed free-text prompt effect returning a string-shaped result. |
| `coerce fn(...) as result` | `coerce` effect. |
| `decide "..." -> { ... } as result` | Inline typed `coerce` effect. |
| `exec "<command>" as result` | Dev-profile `exec.command` effect (gated by `WHIPPLESCRIPT_EXEC_ALLOW`; exposes `exit_code`, `stdout`). |
| `exec <capability> with <record> -> Type as result` | Hosted `exec.command` effect requiring `script.<capability>`, typed JSON stdin, SHA-256 manifest verification, and typed stdout ingestion. |
| `file item into <queue> { ... }` | `queue.file` effect. |
| `claim <item> [as x]` | `queue.claim` effect (already-claimed is a branchable failure). |
| `release <item>` | `queue.release` effect. |
| `finish <item> [{ summary ... }]` | `queue.finish` effect. |
| `timer <duration> as x` | `timer.wait` effect completed when due. |
| `timer until <time> as x` | Absolute `timer.wait` effect completed at or after a typed instant. |
| `cancel <binding>` | Terminal-cancel a pending effect; request cancellation of a running one. |
| `askHuman ... [choices [...]] ...` | `human.ask` effect. |
| `call capability for value as result` | `capability.call` effect. |
| `emit signal <name> to <instance> { ... } as x` | `signal.emit` effect that injects a typed signal into another instance. |
| `acquire <lease> for <key> as x` | `lease.acquire` effect with `held` / `contended` branch outcomes. |
| `release <lease-binding>` | `lease.release` effect. |
| `append Type { ... } to <ledger> as x` | `ledger.append` effect. |
| `consume <counter> for <key> amount <expr> as x` | `counter.consume` effect with `ok` / `over` branch outcomes. |
| `invoke Workflow { ... } as child` | `workflow.invoke` effect. |
| `after effect succeeds/fails/completes` | Dependency branch scoped by terminal status. |
| `case expr { Pattern => { ... } }` | Deterministic finite-domain branch. |
| `complete output { ... }` | `workflow.completed` event and terminal completed state. |
| `fail failure { ... }` | `workflow.failed` event and terminal failed state. |

## JSON contracts

Status values, event types, inspection output, coordination output, provider metadata, and artifact manifest shapes are documented in [JSON reference](json-reference.md).

## Rust APIs

Rust crate APIs are internal-stability contributor interfaces, not the public CLI or JSON contract. See [Rust API reference](rust-api.md).

## Formal and release checks

Common root checks:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
scripts/check-formal-models.sh
scripts/check-tla-models.sh
scripts/check-e2e.sh
scripts/check-release-readiness.sh
```

`scripts/check-formal-models.sh` runs Maude checks and the TLA check wrapper.
`scripts/check-tla-models.sh` runs Apalache type checking and bounded safety.
`scripts/check-e2e.sh` runs deterministic fixture-provider integration tests.
