# Linting And Editor Tooling

Status: implementation-grade target

WhippleScript should have a first-class authoring loop:

```text
whip check   authoritative correctness and safety
whip lint    advisory style/risk/clarity diagnostics
whip fmt     deterministic formatting
whip lsp     editor surface over the same compiler services
```

The linter and language server must not become parallel implementations of the
language. They are product surfaces over the same parser, package resolver,
construct graph validator, type checker, diagnostic renderer, formatter, and
report contracts used by `whip check`.

## Goals

- Make valid WhippleScript easier to write and maintain.
- Surface suspicious but legal patterns before they become operational problems.
- Provide IDE-quality feedback without weakening static analysis or package
  boundaries.
- Let packages contribute declarative editor metadata without running arbitrary
  lint or editor code.
- Reuse the structured diagnostic contract from
  [`error-handling.md`](error-handling.md).

## Non-Goals

This spec does not add:

- a `std.test`, `std.lint`, or editor package
- package-defined lint code
- LSP-only language semantics
- LSP-only parser recovery
- automatic authority grants or package installs
- model-generated fixes as a trusted path
- formal verification of style or helpfulness

Correctness, authority, lifecycle, lowering, and package composition failures
belong to `whip check`, not `whip lint`.

## Layering

The intended layering is:

```text
compiler services
  parse
  package-set / lock discovery
  source bundle resolution
  typed AST / IR
  construct graph
  lowered IR
  static diagnostics
  formatter tree
  symbol index

whip check
  errors for invalid programs
  authoritative JSON reports
  optional model-search

whip lint
  warnings/info/hints for valid programs and package contracts
  optional fixits where safe

whip fmt
  deterministic source formatting

whip lsp
  diagnostics, completion, hover, navigation, code actions, formatting,
  and explain views over the same compiler/lint services
```

No layer above compiler services may reinterpret source or invent additional
runtime behavior.

## `whip check` Boundary

`whip check` owns rejection.

The following must remain check errors, not lint warnings:

- parse failures
- type errors
- expression errors that can produce invalid runtime behavior
- unsatisfied package imports or stale locks
- construct graph rejection
- lowering rejection
- missing capability declarations
- forbidden package semantics
- authority policy mismatches required before execution
- effect-output scope leaks
- unsafe effect cycles
- malformed package manifests/contracts

If accepting the program would threaten correctness, determinism, authority, or
runtime lifecycle invariants, the rule belongs to `check`.

## `whip lint`

`whip lint` is advisory. It runs only after the source reaches a usable checked
state. If `check` fails, `lint` may surface check diagnostics first and skip
advisory rules.

Target command:

```sh
whip lint [<source-or-dir>...] [--json] [--package-lock <path>] [--fix] [--rule <id>] [--allow <id>] [--deny <id>]
```

Default behavior:

- discover `whip.lock` the same way as `check`
- run package resolution and static analysis through compiler services
- emit structured diagnostics with codes under `lint.*`
- exit nonzero only for infrastructure errors, invalid lint configuration, or
  `--deny` rules that fired
- never mutate files unless `--fix` is supplied
- with `--fix`, apply only exact, local fixits

JSON output should use the same diagnostic object model as check reports, with
lint-specific fields for rule id, default severity, configured severity, and
fix applicability.

### Severity And Configured Action

Lint uses the **shared severity enum** from
[`error-handling.md`](error-handling.md). Lint rules emit only:

```text
warning  likely problem, valid source
info     maintainability or clarity issue
hint     editor-only guidance
```

A lint rule never emits `error` for valid source. A linter *infrastructure*
failure (invalid config, internal error) is an ordinary `error` diagnostic with
code `lint.internal` — this is not a redefinition of `error`.

Severity is intrinsic to the diagnostic. It is **distinct from a rule's
configured action**, which is a separate axis set in config or on the CLI:

```text
allow  rule is suppressed (not emitted)
warn   rule is emitted at its default severity
deny   rule is emitted and the CLI exits nonzero
```

`allow` / `warn` / `deny` are configured *actions*, not severity values, and
never appear in the diagnostic's `severity` field. The lint JSON report carries
the rule's `default_severity`, its `configured_action`, and (when known) the
effective severity, alongside the shared diagnostic object.

### Suppression

v0 should support suppressions outside workflow semantics.

Target mechanisms:

```text
CLI:       --allow lint.rule_id
project:   whip.lint.json
```

Inline source suppressions are deferred until the comment/attribute syntax is
settled. If added later, suppressions must be source-spanned, rule-specific,
and visible in reports. Blanket hidden suppressions are not allowed.

Project config shape:

```json
{
  "schema": "whipplescript.lint_config.v0",
  "rules": {
    "lint.unused_import": "warn",
    "lint.broad_file_grant": "deny",
    "lint.missing_assertions": "allow"
  }
}
```

`whip.lint.json` must not affect `check`, runtime behavior, provider authority,
package resolution, or lowering.

## Initial Lint Rules

Initial rules should focus on authoring clarity and operational risk.

### Unused And Dead Surface

```text
lint.unused_import
lint.unused_schema
lint.unused_resource
lint.unused_rule
lint.unused_signal
lint.unused_agent
lint.unreachable_rule
```

These rules should avoid false confidence. For example, a rule waiting on an
external signal may be reachable even if no local rule produces the signal.

### Naming And Namespace Clarity

```text
lint.shadowed_binding
lint.ambiguous_resource_name
lint.near_reserved_word
lint.package_alias_too_broad
lint.provider_name_confusing
```

These rules are warnings about maintainability, not correctness. True reserved
keyword misuse remains a check error.

### Capability And Grant Hygiene

```text
lint.broad_file_grant
lint.broad_memory_grant
lint.broad_script_capability
lint.unused_capability_grant
lint.provider_feature_fragile
```

Examples:

- `with access to project_files { read ["**"] }`
- memory turn grants that allow both recall and learn when only recall is used
- script capability patterns that are legal but too wide for the local call
- relying on an agent harness feature that is available only in one provider
  variant

These should suggest narrowing the grant, not weakening policy.

### Workflow Shape

```text
lint.service_without_service_intent
lint.workflow_without_assertions
lint.large_rule_body
lint.deep_after_nesting
lint.effect_sequence_without_dependency
lint.retry_without_terminal_policy
lint.long_running_without_observability
```

The linter may suggest clearer structure, explicit dependencies, source
assertions, or telemetry hooks. It must not imply that source order creates
effect ordering.

### Package And Provider Authoring

```text
lint.package_missing_diagnostic_metadata
lint.package_missing_examples
lint.package_missing_conformance_fixture
lint.provider_missing_feature_probe
lint.provider_unredacted_evidence_risk
```

These apply to package/provider manifests and specs. Missing required metadata
that prevents package acceptance is a `check` error; metadata that is present
but low quality can be a lint warning.

### Prompt And Coercion Hygiene

```text
lint.unstructured_output_used_as_structured
lint.prompt_requests_json_without_coerce
lint.coerce_result_unused
lint.memory_recall_without_context_boundary
```

These rules are advisory. WhippleScript should not force every prompt through
`coerce`, but it should help authors notice when they are informally asking for
structured data and later treating it as if it were typed.

## Formatter

`whip fmt` should be a stable source formatter over the recoverable parse tree.

Target command:

```sh
whip fmt [<source-or-dir>...] [--check] [--write]
```

Rules:

- no semantic changes (the formatted IR must equal the original IR)
- **idempotent**: `fmt(fmt(x)) == fmt(x)` for all inputs
- **stable across versions**: a formatter change that alters output is a
  breaking change requiring a deliberate reformat, not silent drift
- deterministic: same input + same formatter version → byte-identical output
- preserve prompt bodies exactly unless an explicit prompt-formatting mode is
  later designed
- preserve package-owned source forms using platform grammar shapes
- expose formatting through LSP `textDocument/formatting`

Comment handling is a prerequisite: `whip fmt` is not complete until the
comment/attribute model is settled, because a formatter that drops or relocates
comments is not faithful. Until then, `fmt` operates on comment-free source or
preserves comment tokens verbatim at their attach points; the comment model is
tracked with the inline-suppression syntax in Suppression.

`whip fmt --check` should be suitable for CI (idempotency makes `--check` a fixed
point: a formatted file always passes).

## LSP

Target command:

```sh
whip lsp --stdio
```

The language server should implement standard JSON-RPC LSP over stdio. Other
transports are deferred.

The LSP must use the same compiler services as `check`, `lint`, and `fmt`.
There should be no LSP-only parser, resolver, package loader, type checker, or
construct graph validator.

### Project Model

The language server should:

- discover the workspace root from the opened file, `whip.packages.json`,
  `whip.lock`, or VCS root
- discover `whip.lock` using the same rules as CLI commands
- watch `.whip` files, package set files, package manifests, `whip.lock`,
  lint config, and provider feature report files
- re-check incrementally when possible, but preserve full-check semantics
  (see Incremental Analysis)
- never auto-sync packages, install packages, or grant authority without an
  explicit user command

### Incremental Analysis

The dependency-graph, recursion-stratification, and construct-graph analyses are
whole-program by nature, so "incremental" must not mean a weaker check. The v0
model is conservative:

- **v0 baseline**: on a change, debounce briefly, cancel any in-flight check,
  and re-run the full check for the affected program. Cache parse/IR per file
  keyed by content digest so unchanged files are not re-parsed; the analysis
  phase still runs whole-program. This trades some latency for correctness and
  is acceptable for v0 program sizes.
- **invalidation**: editing a file invalidates that file's parse/IR cache and any
  program whose lock or imports transitively include it; editing `whip.lock`,
  a package manifest, or a provider feature report invalidates all programs that
  resolve through it.
- **no LSP-only incrementality**: any incremental cache is a shared compiler
  service, not LSP-private logic, so CLI `check` and the LSP produce identical
  diagnostics for the same state (the no-LSP-only-semantics rule).
- finer-grained incremental analysis (per-rule, per-node) is an optional later
  optimization and must still yield full-check-equivalent diagnostics.

When the package lock is missing or stale, the LSP should surface the same
diagnostic as `check` and offer a code action to run `whip package sync`.

### LSP Features

Required v0 features:

```text
publishDiagnostics
  check diagnostics
  lint diagnostics
  package-lock diagnostics
  construct graph diagnostics

completion
  keywords and grammar shapes
  package constructs
  resources
  schemas and fields
  enum/literal variants
  signals
  agents and AgentRef domains
  capabilities
  provider/profile names

hover
  types
  schema fields
  resource declarations
  package construct docs
  capability requirements
  provider feature requirements
  lowering summary

definition
  schemas
  fields
  rules
  resources
  signals
  package manifests/contracts
  providers/profiles

references
  schema/resource/rule/signal/agent references

codeAction
  safe diagnostic fixits
  package sync command action
  add missing import where unambiguous
  narrow grant where exact
  apply formatter edits

documentSymbol / workspaceSymbol
  workflows, rules, schemas, resources, signals, providers

formatting
  `whip fmt` edits
```

Optional later features:

```text
semantic tokens
rename
call hierarchy / rule dependency graph
inlay hints for types and capabilities
construct graph explain view
lowered IR preview
background model-search
```

Background model-search must be opt-in. Editors should not require Maude,
Python bridge dependencies, or external formal tooling for normal feedback.

### Diagnostics In LSP

LSP diagnostics should preserve WhippleScript diagnostic codes:

```text
source: "whip"
code: "construct.missing_requirement"
severity: Error | Warning | Information | Hint
data: full WhippleScript diagnostic object
```

The user-visible LSP message can be shorter than CLI output, but the `data`
field should retain provenance, related refs, suggestions, and fixits for code
actions and explain views.

### Code Actions

Code actions must come from structured fixits or safe command actions.

Allowed v0 code actions:

- apply exact local fixit
- run `whip package sync`
- run `whip fmt`
- add a missing import when exactly one package exports the construct
- rename a misspelled field or enum variant when there is one high-confidence
  suggestion
- insert a missing presence proof when the compiler has an exact dominating
  location

Disallowed v0 code actions:

- grant broader authority
- install packages
- change provider credentials
- silence safety errors
- rewrite prompts based on model judgment
- introduce new package dependencies without explicit user confirmation

## Package Editor Metadata

Packages may contribute declarative editor metadata through their manifest or
contract:

```text
construct labels
completion snippets
hover summaries
docs anchors
field/resource labels
capability explanations
provider feature explanations
example declarations and operations
safe fix templates
```

Packages may not contribute:

```text
editor plugins
language-server code
arbitrary lint rules
custom parser recovery
custom code actions
custom diagnostic rendering
background processes
```

The platform validates package editor metadata as part of package-contract
validation, against the same `package_contract_v0` schema that carries the
construct contract. Required-vs-optional metadata is defined **per construct
family**, not globally:

```text
declaration_block (resource)  required: construct label, one hover summary,
                              one example declaration
effect_operation              required: construct label, per-field labels,
                              capability explanation, one example operation,
                              missing-requirement fix template
source_declaration            required: construct label, observation/emit field
                              labels, one example block
projection_read (projection)  required: construct label, projected-type hover
metadata-only constructs      required: construct label only
```

`completion snippets`, `docs anchors`, and `safe fix templates` are optional for
every family. A construct that omits its family's required metadata is rejected
by package-contract validation (code `package_contract.insufficient_metadata`)
rather than silently shipping unhelpful diagnostics. This required set is the
single source of truth shared by `check`, `lint`, and `lsp`; none of them defines
its own metadata-adequacy rule.

## JSON Reports

`whip lint --json` should emit a report envelope:

```json
{
  "schema": "whipplescript.lint_report.v0",
  "path": "workflow.whip",
  "status": "ok",
  "source_hash": "...",
  "package_lock_digest": "...",
  "diagnostics": [],
  "summary": {
    "warnings": 0,
    "info": 0,
    "hints": 0,
    "denied": 0
  }
}
```

The report should reuse the shared diagnostic object from
[`error-handling.md`](error-handling.md). A later schema file should be added
when implementation begins.

The LSP does not need a separate report schema, but tests should be able to
drive the LSP and compare emitted diagnostics, completions, hovers, and code
actions as JSON fixtures.

## Testing

Required tests:

- CLI lint fixtures for every initial lint rule
- `--deny`, `--allow`, and config precedence tests
- `--fix` applies only exact local fixits
- linter never suppresses or downgrades `check` errors
- package editor metadata validates and appears in completions/hovers
- package fixture outcome metadata appears in `test`-scenario `stub` completions
- LSP can discover and run top-level `test` scenarios through the same services
  as `whip test` (the platform scenario runner defined in
  [`workflow-testing.md`](workflow-testing.md); this is distinct from the
  `std.test` *package*, which remains a Non-Goal)
- missing/stale package lock diagnostics match CLI check behavior
- LSP diagnostics match `check` and `lint` diagnostics for the same source
- LSP completion fixtures for package constructs, fields, resources, enum
  variants, signals, agents, and capabilities
- LSP hover fixtures for types, capabilities, provider features, and package
  docs
- LSP code action fixtures for safe fixits and command actions
- LSP does not auto-sync, auto-install, or grant authority
- formatting through CLI and LSP produces identical edits

## Acceptance Criteria

This design is implemented when:

- `whip lint` runs through compiler services and emits shared diagnostics
- initial advisory rules cover unused/dead surface, broad grants, workflow
  shape, package metadata, provider feature fragility, and prompt/coercion
  hygiene
- lint configuration can allow or deny individual rules without affecting
  `check`
- `whip fmt --check` and `whip fmt --write` are available for `.whip` source
- `whip lsp --stdio` provides diagnostics, completion, hover, definition,
  references, code actions, symbols, and formatting
- packages contribute editor metadata declaratively and cannot run editor code
- LSP and CLI diagnostics share codes, spans, provenance, suggestions, and
  fixits
- no LSP behavior changes runtime authority, package resolution, construct
  graph acceptance, lowering, or provider execution
