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
package, check, compile, verify-report, gov, agent, agents, providers, skills,
skill, lint, lsp, fmt, test, run, revise, step, worker, dev, accept, instances,
status, log, facts, effects, runs, artifacts, inbox, signal, ingress, message, issue,
leases, ledger, counters, evidence, diagnostics, trace, otel-export, telemetry,
pause, resume, cancel, checkpoint, restore, fork, retry, recover, auth, deploy,
executor, doctor
```

`branch` also dispatches, but it backs the unreleased v0.4 versioned workspace and
is documented below as experimental. The top-level `whip --help` banner lists a
subset of these commands; the dispatch above is authoritative.

Run `whip <command> --help` or `whip help <command>` to print the usage line
for any command.

### Environment variables

| Variable | Meaning |
| --- | --- |
| `WHIPPLESCRIPT_STORE` | Default store path when `--store` is omitted. |
| `WHIPPLESCRIPT_ITEMS_STORE` | Path for the builtin issue tracker's store (defaults to `.whipplescript/items.sqlite`). |
| `WHIPPLESCRIPT_COORDINATION_STORE` | Path for workspace-scoped lease, ledger, and counter state (defaults to `.whipplescript/coordination.sqlite`). |
| `WHIPPLESCRIPT_EXEC_ALLOW` | Dev-profile raw `exec "<command>"` allow-list: colon-separated glob prefixes such as `scripts/*:bin/ci-*`. Unset/empty, raw exec blocks at admission (`security.script_disabled`); commands outside a non-empty list fail without running. The program must also `use std.script`. |
| `WHIPPLESCRIPT_EXEC_PROFILE` | `dev` (default) or `hosted`. Hosted rejects raw exec strings and requires script capabilities. |
| `WHIPPLESCRIPT_SCRIPT_MANIFEST` | JSON manifest path for hosted script capabilities. Equivalent to `--script-manifest`. |
| `WHIPPLESCRIPT_RUN_ID` | Run identity stamped onto items filed by an agent through `whip issue new`. |
| `WHIPPLESCRIPT_PROVIDER_CONFIGS` | Colon-separated provider binding config paths for the worker. |
| `WHIPPLESCRIPT_WORKER_DIR` | Default worker directory for `whip deploy` when `--worker-dir` is omitted. |
| `WHIP_EXECUTOR_TOKEN` | Bearer token the `whip executor` sidecar requires for non-loopback calls (constant-time compared). |
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
```

The formal/report helper scripts use Python `jsonschema`. From a checkout, use
`nix develop` or install `requirements-dev.txt` before running generated model
searches or report-schema validation outside the packaged CI environment.

With `--provider-config`, JSON output includes `provider_config_checks`. Each
check contains the config path and redacted validation `results`.
With `--providers`, JSON output includes `provider_health_checks`, a
deterministic non-live posture for Codex and Claude. It reports CLI
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
package import (for example `use notes`) or third-party package construct use
fails with a diagnostic that names the blockers and suggests
`whip package sync`; pure `std.` programs are unaffected. A lock entry may
never claim the reserved `std.*` namespace — std packages ship embedded in the
platform, so loading such a lock is an error.
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

With `--package-lock`, imported package libraries such as `use notes` resolve
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
  `class`/`agent`/`enum`/`signal`/`tracker`/`file store` bodies (including a
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
- **`lint.unused_tracker`** — a `tracker` declared but never filed into or claimed.
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
- **`lint.mark_off_consumption_boundary`** — a `mark` whose frozen prefix carries a
  settled effect from a rule that never consumes its trigger: replaying a changed
  candidate re-derives the effect (a refire), so prefix replay refuses pre-flight
  and the pin degrades to input replay. Consume the trigger or move the mark.
- **`lint.tool_grant_requires_owned_harness`** — an agent with a `tools [...]`
  grant (DR-0025) that does not use the owned harness (`provider owned`, or a
  harness of kind `owned`); the grant is dead because sub-workflow tools are only
  resolved and offered in the owned brokered loop.
- **`lint.missing_coercion_import`** / **`lint.missing_coord_import`** /
  **`lint.missing_files_import`** / **`lint.missing_tracker_import`** /
  **`lint.missing_ingress_import`** — the
  program uses `coerce`/`decide`/`prompt` without `use std.coercion`,
  coordination resources (`lease`/`ledger`/`counter` and their verbs) without
  `use std.coord`, file stores (`file store` and the
  `read`/`write`/`import`/`export` verbs) without `use std.files`, the work
  tracker (`tracker` and the `file`/`claim`/`release`/`finish` verbs) without
  `use std.tracker`, or typed signal admission (`signal` declarations,
  external `source` blocks, `emit signal … to`) without `use std.ingress`.
  Advisory only (the graduated import ladder): the program
  still runs, but the import names the std package that owns and configures
  the effects.

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

Supported effect kinds (the canonical dotted kind strings that appear in
`whip effects` and JSON output):

```text
agent.tell
schema.coerce
human.ask
event.emit
signal.emit
workflow.invoke
exec.command
capability.call
lease.acquire
lease.release
ledger.append
counter.consume
tracker.file
tracker.claim
tracker.release
tracker.finish
file.read
file.write
file.import
file.export
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
| `artifacts <run-id>` | Show artifact-manifest metadata for a provider run (`whip artifacts <run-id>`). |
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
  [--root Workflow] \
  [--delivery-id <id>]
```

Validates an external signal payload against the source bundle's declared
`signal <name> { ... }` schema, appends the signal fact to the instance log, and
derives the typed fact that rules match with `when <signal.name> as x` or the
general `when fact <signal.name> as x` form. A malformed or undeclared signal is
rejected before an ill-typed fact can land.

`--delivery-id` supplies the provider/operator delivery identity: it wins over
the derived payload hash as the admission key, so re-running the same id
admits once across process runs — the duplicate is absorbed with a diagnostic
(`"duplicate": true` under `--json`, naming the original event) instead of a
second fact.

Human output prints the signal sequence. JSON output includes:

```json
{
  "instance_id": "inst_...",
  "signal": "deploy.finished",
  "event_id": "evt_...",
  "fact_event_id": "evt_..."
}
```

Exit behavior:

| Exit | Meaning |
| --- | --- |
| `0` | Signal accepted and typed fact derived (or a duplicate delivery absorbed). |
| `1` | Store, source, or payload validation failed. |
| `2` | CLI usage error. |

### `ingress serve`

```sh
whip [--store path] ingress serve --stdio --program <workflow.whip> [--root Workflow]
```

The resident stdio admission driver (`std.ingress.stdio`, a dev/test reference
path): reads JSONL envelopes `{"instance", "signal", "payload",
"delivery_id"?}` from stdin and admits each through the same admission core as
`whip signal` — declared-signal check, payload validation, internal-channel
gate, idempotent delivery key. One JSON result line per envelope on stdout
(`"status"`: `admitted` / `duplicate` / `rejected` with a `reason`); a
malformed line is rejected before any fact; the process exits `0` at EOF. The
HTTP listener driver is deferred (`spec/std-ingress.md`, "Deferred with
cause").

### Issue commands

```sh
whip issue new --tracker <TR> --title <T> [--body <B>] [--label <L>] [--actor <A>]
whip issue list [--tracker <TR>] [--status <S>]
whip issue show <id>
whip issue ready <tracker> [--limit <N>]
whip issue claim <id> [--actor <A>]
whip issue renew <id> [--actor <A>]
whip issue release <id>
whip issue finish <id> [--summary <S>]   # alias: complete
whip issue fail <id> [--actor <A>]
whip issue dep add <blocked> depends-on <blocker>
whip issue rebuild
```

Issue commands operate the builtin issue tracker (see
[trackers](language-reference.md#trackers)). The builtin tracker is
workspace-scoped, stores items in `.whipplescript/items.sqlite` (override with
`WHIPPLESCRIPT_ITEMS_STORE`), and issues sequential ids `WS-1`, `WS-2`, and so
on. `--status` filters on the status categories `open`, `in_progress`,
`closed`, `canceled`, and `archived`. Every mutating command returns the full
issue row (JSON with `--json`), so an agent never needs a follow-up `show`.
Claims are local runtime leases split from durable status: a plain `claim`
overlays `in_progress` and re-projects the issue as not-ready without a durable
write; `release`, `renew` (holder-only heartbeat), and terminal auto-release
manage the lease. `dep add` records a `blocks` edge (the blocked issue is gated
until the blocker closes). Actor identity defaults to the run-identity stamp
(`WHIPPLESCRIPT_RUN_ID` / `WHIPPLESCRIPT_INSTANCE_ID`) and is overridable with
`--actor`, so an item an agent files or claims mid-turn carries run-identity
provenance.

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

### `message`

```sh
whip message <instance> --channel <name> --text <text> \
  [--markdown <md>] [--by <sender>] [--thread <id>] \
  --program <workflow.whip> [--root <workflow>]
```

Injects an inbound channel message into a running instance. It compiles the
program, checks that `<name>` names a declared `channel`, builds the generic
`Message` envelope, and derives a `message.<channel>` fact that rules match with
`when message from <channel> as x`. A message needs at least one of `--text` or
`--markdown`; an undeclared channel or a missing instance is a usage error.
`--by` records the claimed-actor sender identity (`--from` is accepted as an
alias); `received_at` is the injection instant, and the message id is minted
per delivery, so re-sending identical text is a distinct message. JSON output
includes the recorded event and derived fact ids.

### `mailbox`

```sh
whip mailbox outbound [--channel <name>] [--limit <n>]
whip mailbox inbound <channel> [--limit <n>]
```

The read surface over the LOCAL messaging provider's files. `outbound` lists
the rows `send via` a `local` channel delivered to `<store>.mailbox.jsonl`
(optionally filtered by channel); `inbound <channel>` lists the received rows
in `<store>.inbox.<channel>.jsonl` with their admission ordinals. Read-only —
injection stays `whip message`. `--json` emits the full records.

## Cloud deployment and runtime

The same durable rule/effect kernel that runs locally also runs unchanged inside
a Cloudflare Durable Object wasm isolate. `deploy` publishes a workflow to that
edge runtime; `executor` is the optional Class-A compute sidecar.

### `deploy`

```sh
whip deploy [--worker-dir <path>] [--name <worker>] \
  [--dry-run] [--skip-build] [--set-secrets]
```

One-command edge deploy of a workflow to a Cloudflare Worker plus its Durable
Object. `deploy` requires `npm` and `wrangler` on `PATH`; it stages the worker
bundle, optionally builds it, and publishes it. `--worker-dir` selects the
worker directory (or set `WHIPPLESCRIPT_WORKER_DIR`); `--dry-run` validates the
bundle without publishing; `--skip-build` reuses a prior build; `--set-secrets`
pushes locally set provider credentials as Worker secrets (a key that is not set
locally is skipped with a note). On the edge, timers and deadlines fire through
Durable Object alarms and provider credentials come from Durable Object secrets.

### `executor`

```sh
whip executor [--bind <addr:port>]
```

Runs the Class-A exec sidecar that backs real toolchains for the compute plane,
serving the `whip-executor/1` wire. The default bind is loopback only
(`127.0.0.1:8080`); a container entrypoint binds `0.0.0.0:8080`. In-cluster calls
authenticate with a constant-time-compared bearer token
(`WHIP_EXECUTOR_TOKEN`), and the sidecar refuses non-loopback calls that lack it.
The compute plane (this Class-A sidecar and the Class-B per-turn container path)
is built and live-proven; enabling it in production is a follow-on configuration
step, not on by default.

## Restorable context

`checkpoint` and `restore` rewind an agent's work — its files, its transcript,
and the instance's event-log position — to a prior point as one consistent,
coherence-checked cut. File history is captured content-addressed, so restore
reverts to exact prior bytes. Native file I/O only.

### `checkpoint`

```sh
whip [--json] checkpoint <instance> [--cut-id <id>] [--external-positions <json|@file>]
```

Captures a checkpoint of the instance's three planes at the current event
position. `--cut-id` names the cut; when omitted a timestamped id is generated.
Human output reports the cut id, event sequence, and captured file count; JSON
output adds the manifest hash. `--external-positions` records the position
**pair** for cross-store backup/handoff: an external authority's own scope
positions (inline JSON, or `@path` to a file) travel inside the same fenced
`plane.positions` event as the workspace cut id, so both stores restore to one
coherent coordinate. Read the pair back via `whip handles`.

### `handles`

```sh
whip [--json] handles <instance>
```

The referenceable-handles surface (`whipplescript.handles.v0`): the stable
pointers an external policy authority admits decisions against —
one-owner-per-fact, so the authority's log records decisions plus these
pointers while the facts stay whip-owned. Reports the instance's latest
event-log position, its effect ids with status, the workspace binding's line
with head and recent cut ids, and the latest position-pair cut. See
`spec/store-seam-contract-draft.md` for the seam contract these handles serve.

### `restore`

```sh
whip [--json] restore <instance> <cut-id>
```

Restores the instance to the named checkpoint. It coherence-checks the cut up
front and refuses (mutating nothing) rather than applying a partial cut, then
auto-checkpoints the current head so the restore is itself undoable, applies the
file reconcile (writing manifest paths back to cut content and removing post-cut
files), and commits a `context.restored` marker that folds the instance and
transcript planes to the cut. The success line names the `auto-before-<cut-id>`
checkpoint you can restore to in order to undo. Exit `1` if the cut is refused.

### `fork`

```sh
whip [--json] fork <instance> [--agent <name>] [--branch-id <id>]
```

The chat fork: births a **new** instance whose agent thread is seeded from the
source's completed turns and — when the source is branch-bound — whose file
surface is a fresh branch forked at the source line's head, bound at birth.
Both planes are taken from one quiescent coordinate: a running effect refuses
the fork. The fork inherits the source's program version, input, and authority,
but not its event history — it never pretends to have executed the source's
effects. It does not replay the workflow start; drive it with `whip signal` or
`whip message`, and its next `thread continue` turn resumes the seeded
conversation while file effects land on its own line with branch-distinct
effect keys. `--agent` picks the thread when several agents completed turns
(default: the newest completed turn's agent); `--branch-id` names the fork
branch. An unbound source forks thread-only, reported honestly. JSON output
carries the new instance id, the seeded message count, the source coordinate,
and the branch pair (or `null`).

## Improve (experimentation surface)

Gauge evidence, pinned scenarios, and campaign records live in a
workspace-scoped improve store (`.whipplescript/improve.sqlite`, env
`WHIPPLESCRIPT_IMPROVE_STORE`), separate from disposable run stores.

### `pin`

```
whip [--json] pin <instance> [at <mark>] --as <name>
```

Pins a run as a named scenario — the regression corpus grows out of use,
not authoring. A bare pin freezes the run's input (regeneration re-runs
the whole workflow); `at <mark>` freezes the prefix at the first time the
run reached the declared mark — regeneration then replays that prefix
(replayed effects never re-fire) and re-executes only the suffix, pairing
both arms at the cut.

### `suppose`

```
whip [--json] suppose <scenario> [--program <workflow.whip>] [--root <workflow>]
     [--provider <name>] [--provider-config <path>]
```

One what-if regeneration of the pinned scenario under the current (or a
candidate) program — the everyday debugging what-if. Mark pins replay the
frozen prefix; the recorded run is scored in place as the paired control;
output is per-gauge `recorded → regenerated` with pass verdicts, replay
accounting (events replayed, refires), and honesty tags
(`prefix-replay`, `replay-refire`, `clock-sensitive`, `replay-fallback`).
Each gauge line carries `p_better` — the Bayesian sign test over the
(recorded, regenerated) pair under a Jeffreys prior — when both sides
carry bar verdicts (a single continuous delta has no scale, so the
t-family stays silent at N=1). Every suppose lands in the evidence
ledger. A candidate program that is not revision-compatible with the
frozen prefix degrades honestly to input replay.

### `settle`

```
whip [--json] settle <gauge> [--certify] [--threshold <k>] [--spend-cap $<n>]
     [--program <workflow.whip>] [--root <workflow>]
     [--provider <name>] [--provider-config <path>]
```

Names the decision (the gauge's declared bar — a gauge without one is
refused) and lets the system stop itself: regenerations race round-robin
over the pinned scenarios, each bar-passing observation raising the
evidence level and each contrary one lowering it (floored at zero), until
the level crosses the threshold (`bar-cleared`) or a full pass over the
pool adds no net evidence — an honest `undetermined`, never an
operator-chosen sample size. The crossing is anytime-valid (sequential
e-process shape), so stopping at it is sound. `--certify` records the
crossing observation with a `certificate` tag and mints a certificate id.
`--spend-cap` is a guardrail in currency, never a sample size: it stops
the race (an honest `undetermined`, reason `spend-cap-reached`) when the
**priced** cost of regenerations crosses it, rated by the provider
config's `prices` block; unpriced usage cannot bind it. Every settle
regeneration lands in the evidence ledger tagged `settle`. Alongside
the walk, `p_bar_met` reads out the Jeffreys Beta posterior that a
regeneration clears the bar (referenced at the chance bar's own rate) —
a readout, never the stopping rule.

### `gauges`

```
whip [--json] gauges [<gauge>]
```

The accumulated gauge evidence: mean, N with the regen/live decomposition,
and pass counts. Ambient rows land from `whip dev` automatically for
deterministic judges (exec + builtins). The output also carries
**standing-contradiction flags**: for every accepted tradeoff answer,
live evidence under the accepted candidate's program hash folds into a
contradiction posterior against the answer-time operating point, and a ⚠
line (JSON `contradictions`) is raised only when the posterior sits at
≥ 0.8 and has not receded over the last three informative observations —
sustained, never a spike. The flag is advisory and cites the precedent;
revoking the answer stays `whip answer --revoke`.

### `improve`

```
whip [--json] improve [<gauge>[>=<target>] ... [then ...] | <campaign>]
     [--program <workflow.whip>] [--sacrifice <gauge>] [--within <gauge>=<band>%]
     [--spend-cap $<n>] [--proposer fixture|native] [--provider <name>]
     [--redacted-view]
```

Runs an improvement campaign. The partition is expressed by which gauges
you name: named gauges ascend, unnamed gauges are guarded within
indifference bands, `--sacrifice` releases, declared bars are always hard.
`--spend-cap` is enforced against **priced** recorded cost: rates come
from the provider config's `prices` block (USD per million tokens, per
provider/model, input and output separately — config-only, no shipped
defaults); usage with no matching rate records honestly as `unpriced`
with cost 0 and cannot bind the cap. A campaign that crosses its cap
**parks** (`campaign.parked` in the record, `"parked": true` in the
report); `whip improve --resume <campaign-id>` continues it under a
fresh per-invocation allowance (the spec, program, and candidate
numbering come from the record; a changed program refuses on the
baseline-hash guard).
Inline targets (`extract_quality>=0.9`) become reach bounds; `then`
separates lexicographic stages with ratchet semantics: a stage whose
reach targets the baseline already meets advances at invocation time —
its achieved levels become hard guard floors for later stages (recorded
as `stage.advanced` campaign events), and a target-less stage is
open-ended maximization that never auto-advances. A single positional naming a declared `campaign` adopts its
spec. Bare `whip improve` is repair mode. The loop: seal a holdout
(20% / floor 2 of pinned scenarios; below 4 scenarios the campaign runs
tagged `unheld-out`), evaluate the baseline, then propose → static-gate →
evaluate → dominance verdict → sealed promotion gate. A candidate is
proposed only if it improves an ascend gauge, regresses nothing guarded,
and meets every bar; a genuine tradeoff is surfaced as a decision, never
auto-accepted. The proposer never sees sealed scenario contents
(aggregates only); `--redacted-view` (or a declared `proposer redacted`
clause) extends that to ALL scenario content — the campaign's evidence is
tagged `proposer:redacted-view`. Every candidate is checked for verbatim
scenario-payload fragments newly present in its source; matches tag the
card `leakage-overlap` (a flag, never a block — adoption remains the
audited human act). Terminal state is an evidence card per candidate —
propose, don't apply.

### `campaigns` / `campaign`

```
whip [--json] campaigns
whip [--json] campaign <id>
```

The folded campaign records: candidates considered, evidence cards as they
stood, sealing, adoption decisions — program archaeology's data.

### `adopt`

```
whip [--json] adopt <campaign>:<candidate> [--program <workflow.whip>]
```

Writes a candidate's source into the program — the explicit human adoption
act. Reserved for candidates the campaign proposed, or tradeoffs accepted
via `whip answer`. Refuses honestly if the program changed since the
campaign evaluated its baseline.

### `answer`

```
whip [--json] answer <campaign>:<candidate> --accept|--reject|--revoke [--by <who>]
```

Answers a surfaced tradeoff (only `candidate.tradeoff` decisions are
answerable). The answer is a **precedent** — a recorded speech act that
makes an accepted candidate adoptable and, by default, auto-resolves
future tradeoffs by *monotone precedent dominance*: a new tradeoff at
least as good on every gauge as an accepted precedent auto-accepts; one at
most as good as a rejected precedent everywhere auto-rejects; anything
between — or covered by conflicting precedents, or outside the answer-time
operating point's band neighborhood — asks. Every auto-resolution cites
its precedent on the evidence card (`auto-resolved:precedent` /
`auto-rejected:precedent` tags). `--revoke` withdraws the answer and with
it all authority it granted.

## Credentials

### `auth`

```sh
whip auth status
whip auth set <openai|anthropic> <key>
```

Manages native coerce-provider credentials. `status` reports, per provider, where
a credential resolves from and a redacted preview (JSON adds
`whipplescript.auth_status.v0` with a `configured` flag). `set` stores a key for
`openai` or `anthropic`. There is no `login` subcommand — whip runs no OAuth
flow.

An embedding host may own auth entirely by handing whip resolved credentials
inside provider profiles: `WHIPPLESCRIPT_PROVIDER_PROFILES` names a host-written
JSON file mapping profile name (the agent's declared `profile`, with `default`
as the catch-all) to `{ provider, model, api_key | api_key_env, base_url?,
max_tokens?, timeout_secs? }`. When an entry matches, owned turns use it and
whip performs no credential acquisition; the resolver order above becomes the
standalone fallback. A configured-but-broken entry fails the turn honestly
rather than silently falling back. `status` names the active profiles file.

## Introspection and governance

### `agents`

```sh
whip [--json] agents [--root <workflow>] <workflow.whip>
```

Lists every declared agent in a program from the compiled IR, with each agent's
provider, harness, profile, capacity, capabilities, tools, and skills.

### `providers`

```sh
whip [--json] providers [--root <workflow>] <workflow.whip>
```

The operator view of every provider a program needs: aggregates the `provider` of
each declared agent, channel, and source into a distinct list, with what
references each.

### `skills`

```sh
whip [--json] skills [--root <workflow>] <workflow.whip>
```

Lists every skill declared across a program's agents, each with the agents that
declare it.

### `skill`

```sh
whip [--json] skill list
whip [--json] skill validate <SKILL.md|dir>
whip [--json] skill install <SKILL.md|dir>
```

Manages the local owned-harness skill control plane: `list` enumerates installed
skills, `validate` checks a `SKILL.md` (or a directory of them), and `install`
adds one to the local skill store. Skills are content-addressed and never grant
authority.

### `gov`

```sh
whip gov <sign|verify|escalate|escalations|agent> [args]
```

The governance surface (DR-0026/0028). `sign`/`verify` operate on signed
governance envelopes, `escalate <request>` files a low-integrity request to the
`WHIPPLESCRIPT_GOV_ESCALATIONS` log for an admin to review, `escalations` lists
them, and `gov agent` starts the privileged governance agent loop (it refuses to
start without governance privilege).

### `infoflow`

```sh
whip infoflow
```

Starts the unprivileged interactive whip-agent loop (DR-0026/0028). It reads
commands from stdin — `check <file>` runs the information-flow check on a whip
source, `escalate <request>` files a governance escalation, and `quit` exits. It
has no path to signing governance; a `sign` is refused. Renamed one-way from
`whip agent` (no alias; spec/std-agent.md "Operator CLI") — the old spelling
errors with a pointer here.

### `verify-report`

```sh
whip verify-report [--entry-index <n>] \
  [--emit construct-graph|lowered-ir|artifacts] \
  <check-or-compile-or-artifacts-report.json>...
```

Re-verifies the digests inside a previously emitted `check`, `compile`, or
artifacts report and re-derives its verified artifacts. `--entry-index` selects
one entry from a multi-entry report; `--emit` prints a chosen embedded artifact
(the construct graph, the lowered IR report, or the artifact set).

### `branch` (experimental, unreleased)

```sh
whip [--json] branch <create|list|show|write|read|ls|remove|merge|discard|bind> ...
```

`branch` (and its companion `stream`) back the versioned workspace (whip-native
VCS). The surface is present and functional but should be treated as
experimental and subject to change.

## Language reference index

For examples and semantics, see [Language Reference](language-reference.md).
This section is a compact index of source constructs.

### Top-level constructs

| Construct | Surface | Meaning |
| --- | --- | --- |
| Workflow | `workflow Name { ... }` or `workflow Name` | Deployable runtime boundary. |
| Contract | `input name Type`, `output name Type`, `failure name Type` | Typed workflow input/output/failure contract. |
| Include | `include "path.whip"` | Source bundle composition. |
| Package/library import | `use std.memory` | Import package library surface by name. |
| Class | `class Name { field Type }` | Typed fact and payload schema. |
| Enum | `enum Name { A B }` | Finite string domain. |
| Signal | `signal deploy.finished { field Type }` | Typed external signal ingress schema. |
| Harness | `harness coder: codex` | Named provider endpoint family used by `agent ... using coder`. |
| Agent | `agent name { profile "..."; capacity N; skills [...] }` | Logical provider target and policy metadata. |
| Coerce | `coerce fn(args...) -> Type { prompt """markdown ... """ }` | Declared coerce-backed effect. |
| Flow | `flow name when ... { step; step; ... }` | A rule whose body is a multi-step sequence; lowers to `flow.<name>.seg<N>` rules. |
| Channel | `channel name { provider local destination "#ops" }` | Named messaging endpoint for inbound `when message from` and outbound `send via`; the provider must be one of `local`, `desktop`, `stdio`, `fixture`. |
| Source | `source clock\|file\|http as name { ... observe as obs emit <signal> { ... } }` | Ingress source (clock schedule, file `path` lines or `watch` content occurrences, or GET-only http fetch, optionally `dedup <obs>.<field>`) whose `emit` clause admits observed input as a typed signal through the admission core. The provider kind must be contributed by an embedded/locked package manifest. |
| File store | `file store name { root "..." allow read [...] allow write [...] }` | Policy boundary over a provider-backed document root for `read text`/`write text`/`import`/`export`. |
| Tracker | `tracker name { provider builtin }` | Declared vendor-neutral work-item backlog. |
| Lease | `lease name { key Type slots N ttl 10m }` | Workspace-scoped bounded mutex/semaphore resource. |
| Ledger | `ledger name { entry Type partition by field retain 90d }` | Workspace-scoped append log partitioned by a typed field. |
| Counter | `counter name { key Type cap N reset daily }` | Workspace-scoped consumable budget with lazy reset. |
| Pattern | `pattern Name<T> { ... }` | Compile-time reusable fragment. |
| Apply | `apply Name<Type> as Alias { ... }` | Pattern specialization. |
| Assertion | `assert expression` | Deterministic projection check in `dev`. |
| Gauge | `gauge name [on site] { judge via ... expect ... }` | Named quality dimension: judge + optional bar; scored ambiently, optimized by `whip improve`. |
| Campaign | `campaign name { ascend ... reach ... guard ... sacrifice ... }` | Versioned objective intent: the partition of the gauge vector. |

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
| Tracker readiness | `when <tracker> has ready issue as x` | Match an item that is ready to be claimed in a work tracker. |
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
| `coerce fn(...) as result` | `schema.coerce` effect (the source keyword is `coerce`; the effect kind is `schema.coerce`). |
| `decide "..." -> { ... } as result` | Inline typed `schema.coerce` effect. |
| `exec "<command>" as result` | Dev-profile `exec.command` effect (requires `use std.script` + a non-empty `WHIPPLESCRIPT_EXEC_ALLOW`, which seed the `script.raw` capability; exposes `exit_code`, `stdout`). |
| `exec <capability> with <record> -> Type as result` | Hosted `exec.command` effect requiring `script.<capability>`, typed JSON stdin, SHA-256 manifest verification, and typed stdout ingestion. |
| `file issue into <tracker> { ... }` | `tracker.file` effect. |
| `claim <item> [as x]` | `tracker.claim` effect (already-claimed is a branchable failure). |
| `release <item>` | `tracker.release` effect. |
| `finish <item> [{ summary ... }]` | `tracker.finish` effect. |
| `read text from <store> at <path> as x` | `file.read` effect over a declared `file store`. |
| `write text to <store> at <path> { body ... [mode ...] } as x` | `file.write` effect over a declared `file store`. |
| `import <jsonl\|json\|csv> <Schema> from <store> at <path> as x` | `file.import` effect decoding structured rows into typed facts. |
| `export <jsonl\|json\|csv> <Schema> to <store> at <path> { [where ...] mode ... } as x` | `file.export` effect writing typed rows to a file. |
| `redact <binding> keep [..] as <out>` | Project a record-typed binding onto a chosen subset of fields (type-level + runtime field drop with per-field IFC refinement). |
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
