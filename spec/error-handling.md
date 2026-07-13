# Error Handling And Diagnostics

Status: implementation-grade target

WhippleScript's diagnostic north star is:

```text
Most users should be able to fix a problem directly from the error message.
```

The model to emulate is Gleam-style helpfulness: precise location, plain
language, local context, and a concrete repair path. WhippleScript has a harder
problem because errors can come from source parsing, type checking, package
composition, construct graph acceptance, lowering, capability policy, runtime
lifecycle, and external providers. The answer is not longer strings. The answer
is structured diagnostics with provenance and package-supplied labels rendered
by the platform.

## Goals

Diagnostics should answer:

```text
what failed
where it failed
why WhippleScript rejected it
which invariant or authority boundary was protected
what concrete edit or operator action is likely to fix it
```

Diagnostics should be:

- source-local when source exists
- stable enough for fixtures and CI
- structured in JSON reports
- rendered consistently in text output
- explainable through package/construct/lowering provenance
- safe: no secret values, prompt payload dumps, or provider credential leaks
- useful for both workflow authors and package/provider authors

## Non-Goals

This spec does not require:

- package-owned custom error renderers
- arbitrary package-emitted compiler strings
- model-generated explanations as primary diagnostics
- exact compatibility for pre-release diagnostic wording
- one diagnostic schema version for every report immediately

The platform may keep multiple report schemas during migration, but every new
diagnostic surface should move toward the same structure.

Lint and editor diagnostics are specified in
[`editor-tooling.md`](editor-tooling.md). They reuse this diagnostic object
model, but they do not decide source validity.

## Diagnostic Object

Every compiler, package, construct, lowering, runtime, and provider diagnostic
should normalize into this conceptual shape:

```text
diagnostic {
  code
  severity
  title
  message
  primary_span?
  secondary_spans[]
  provenance
  expected?
  found?
  invariant?
  labels[]
  suggestions[]
  fixits[]
  explain[]
  docs?
  related[]
  redaction
}
```

Required fields:

```text
code        stable dotted identifier
severity    error | warning | info | hint
title       one-line human summary
message     concise explanation
provenance  owning layer and refs
redaction   whether sensitive material was omitted
```

`primary_span` is required for source diagnostics when source exists. Runtime
and provider diagnostics may have no source span, but should still carry source
provenance when they are caused by a source construct.

## Codes

Diagnostic codes are stable identifiers, not prose. Use dotted namespaces. The
reserved top-level namespaces are: `parse`, `type`, `expr`, `graph`, `effect`,
`construct`, `lowering`, `capability`, `runtime`, `provider`, `package_set`,
`package_lock`, `package_contract`, `security`, and `lint`. The `lint.*`
namespace is owned by the linter; all other namespaces are owned by `check` /
runtime / providers and a lint rule must not reuse them.

```text
parse.expected_declaration
type.unknown_field
expr.optional_without_presence
graph.effectful_cycle                # static analysis: cycle not crossing a boundary
graph.unstratified_recursion         # static analysis: negation/recursion not stratified
effect.implicit_ordering             # static analysis: sibling effects assume order
effect.output_scope_leak             # static analysis: effect output used outside its `after`
effect.unsatisfiable_dependency      # static analysis: dependency edge cannot resolve
package_lock.missing
package_contract.invalid_capability
construct.missing_requirement
construct.ambiguous_resolution
construct.cardinality_conflict
lowering.duplicate_core_object_owner
capability.not_granted
runtime.stale_completion
provider.feature_unavailable
provider.output_validation_failed
security.script_disabled
lint.unused_import                    # lint.* is the advisory namespace
lint.broad_file_grant
lint.internal                         # linter infrastructure failure (severity error)
```

The analyses in [`static-analysis.md`](static-analysis.md) emit codes under
`type`, `expr`, `graph`, and `effect`; that spec and this code list must stay in
sync as analyses land.

### Code Governance

- Codes are stable. A code may be added; it must not be renamed or removed once
  shipped. Replacing a code requires keeping the old code as an alias until a
  major version.
- A new code is allocated under exactly one owning namespace by the subsystem
  that emits it. `lint.*` is the only namespace package/lint authors observe;
  packages never allocate core (`parse`/`type`/…) codes.
- Code stability matters more than message stability. Tests should generally pin
  codes, spans, labels, and suggestions, while allowing wording to improve.

## Severity

There is **one** severity enum, shared by `check`, `lint`, `lsp`, `test`,
runtime, and provider diagnostics. It is aligned 1:1 with LSP severities so no
remapping is needed:

```text
error    command or runtime transition cannot proceed   (LSP Error)
warning  accepted, but likely unintended or requiring attention (LSP Warning)
info     maintainability/clarity observation; never blocks  (LSP Information)
hint     editor-only guidance; never blocks                 (LSP Hint)
```

`note` is **not** a severity. Supporting context attached to another diagnostic
is carried as related information (a secondary span or a related diagnostic ref),
not as a top-level diagnostic. See Spans And Labels.

Severity is the diagnostic's *intrinsic* level. It is distinct from a lint
rule's *configured action* (`allow` / `warn` / `deny`), which is configuration,
not severity — see [`editor-tooling.md`](editor-tooling.md). `deny`/`allow` never
appear in the `severity` field.

Usage rules:

- `check`, static analysis, runtime, and provider diagnostics use `error`,
  `warning`, and (rarely) `info`. They do not emit `hint`.
- `lint` rules emit only `warning`, `info`, or `hint` — never `error` for valid
  source. A linter *infrastructure* failure (bad config, internal error) is an
  ordinary `error` diagnostic with an infra code (e.g. `lint.internal`), not a
  redefinition of `error`.
- Compiler warnings must not silently change runtime behavior. Runtime warnings
  must be durable evidence or diagnostics when they affect operator decisions.

## Spans And Labels

A source diagnostic should have one primary span and zero or more secondary
spans:

```text
primary span     the source that must change
secondary spans  definitions, candidates, imports, package declarations,
                 prior claims, provider bindings, or related branch sites
```

Labels should be short and specific:

```text
this field does not exist on `Issue`
`project_memory` is declared here
this package also exports a bare `send`
this profile grants `files.read`, but not `files.write`
```

When the source span came through package lowering, the diagnostic should retain
both:

```text
source span      user-authored source
construct ref    normalized construct graph node/port/edge/lowering ref
```

Users see the source. `--explain` and JSON reports expose the construct refs.

## Suggestions And Fixits

Suggestions are human guidance. Fixits are machine-applicable edits.

```text
suggestion {
  message
  applicability: exact | likely | manual
}

fixit {
  title
  edits[]
  applicability: exact | likely
}
```

Rules:

- emit a fixit only when the edit is local and semantics-preserving enough to be
  safe
- prefer one high-confidence suggestion over a list of vague possibilities
- do not suggest granting broader authority unless the diagnostic is explicitly
  an operator-profile problem
- do not suggest disabling a safety check
- for package/import errors, suggest `whip package sync` only when that is
  actually the missing step

## Rendering

Text diagnostics should be concise by default and expandable on demand.

Default shape:

```text
error[type.unknown_field]: `Issue` has no field named `prioirty`
  --> triage.whip:18:24
   |
18 |   when Issue as issue where issue.prioirty == high
   |                            ^^^^^^^^^^^^^^ unknown field
   |
   = did you mean `priority`?
```

For construct graph errors, the default message should use package vocabulary,
not graph vocabulary:

```text
error[construct.missing_requirement]: `recall` needs a memory pool named `project_memory`
  --> triage.whip:31:15
   |
31 |   recall from project_memory for issue as context
   |               ^^^^^^^^^^^^^^ no memory pool with this name is visible
   |
   = declare one with `memory pool project_memory { ... }`
```

`--explain` may add graph detail:

```text
construct node: memory.recall#n42
required port: memory.pool input exactly-one resource=project_memory
candidates considered: 0
```

## Diagnostic Layers

Diagnostics should name the layer that rejected the program or transition.

```text
parse
source_shape
type
expression
static_analysis
package_set
package_lock
package_contract
construct_grammar
construct_graph
lowering
capability
runtime
provider
security
report_admission
```

Layering keeps messages honest. A construct graph failure should not pretend to
be a parser failure, and a provider failure should not pretend to be a source
typing failure.

## Parse And Source Shape

Parse diagnostics can be the closest to Gleam.

Requirements:

- report the construct the parser was trying to parse
- show the expected token or source shape
- include the nearest enclosing declaration when useful
- prefer "I expected..." wording over grammar jargon
- recover far enough to report multiple independent errors when practical
- reject removed historical syntax with targeted diagnostics

Example:

```text
error[parse.expected_as_binding]: expected `as <name>` after this source declaration
  --> workflow.whip:12:14
   |
12 | source clock daily_triage
   |              ^^^^^^^^^^^^ this source needs a binding name
   |
   = write `source clock as daily_triage`
```

## Type And Expression Diagnostics

Type diagnostics should be specific to the authoring domain:

- unknown names should include did-you-mean suggestions
- field errors should name the receiver type
- enum/literal errors should list the valid small domain
- optional access errors should point to the missing presence proof
- projection errors should name the projected fact/effect/schema
- expression errors should say whether the expression evaluated to non-boolean,
  `Missing`, `null`, or an unsupported comparison

When a value came from a package construct, the message should still name the
source-level package concept before naming the lowered core type.

## Construct Grammar Diagnostics

Construct grammar diagnostics occur while parsing and checking package-owned
source forms against platform-owned construct families.

Requirements:

- package constructs must provide display labels for the construct, fields,
  resources, ports, capabilities, examples, and common fixes
- the platform renders the diagnostic
- packages may not emit arbitrary compiler error text
- package-supplied messages are templates over platform-approved fields, not
  free-form control over rendering
- every package-owned construct should include at least one invalid-source
  fixture for each required field, resource reference, and capability boundary

Package diagnostic metadata should include:

```text
construct_display
field_labels
resource_labels
port_labels
cardinality_explanation
missing_requirement_template
ambiguity_template
capability_explanation
provider_feature_explanation
example_declaration
example_operation
docs_anchor
```

The platform may reject a package contract that exports constructs without
enough diagnostic metadata for its declared construct family.

## Construct Graph Diagnostics

Construct graph diagnostics explain package interoperability failures.

Every rejected required port should carry a resolution trace:

```text
required port
source construct
expected kind/type/phase/version/resource/cardinality
visible candidates considered
rejection reason for each candidate
selected edges, if any
final diagnostic code
```

Common construct graph codes:

```text
construct.missing_requirement
construct.ambiguous_resolution
construct.incompatible_type
construct.incompatible_phase
construct.incompatible_version
construct.resource_mismatch
construct.cardinality_conflict
construct.reserved_keyword
construct.namespace_ambiguous
construct.capability_not_declared
construct.lowering_class_unavailable
```

Graph diagnostics should default to source/package vocabulary:

```text
`send` is ambiguous because two visible packages export a channel named `ops`.
```

Graph refs and derived facts belong in JSON and `--explain`, not in the first
line of the default error.

## Lowering Diagnostics

An accepted construct graph that cannot lower is usually one of:

- a platform bug
- a stale package contract
- an unsupported platform catalog feature
- a report/artifact admission failure

User-facing lowering diagnostics should say that the source was accepted but
the platform could not preserve it during lowering. The message should include
the package, construct family, lowering class, and exact unsupported or
contradictory output.

Required codes include:

```text
lowering.unsupported_class
lowering.lifecycle_profile_mismatch
lowering.duplicate_core_object_owner
lowering.unowned_core_object
lowering.forbidden_runtime_object
lowering.signal_source_as_occurrence
```

For package authors, `--explain` should show the construct graph node, lowering
entry, emitted core object refs, and validator-owned facts involved.

## Capability And Security Diagnostics

Authority diagnostics should be blunt and actionable.

They must name:

```text
requested capability
source construct or runtime request
active profile
provider binding, if any
missing grant or denied policy
safe next action
```

Example:

```text
error[capability.not_granted]: this workflow can request `script.run.deploy`, but profile `ci` does not grant it
  --> deploy.whip:44:3
   |
44 |   run script deploy_release with input as result
   |   ^^^^^^^^^^^^^^^^^^^^^^^^^ this requires `script.run.deploy`
   |
   = choose a profile that grants this named script capability, or remove the script call
```

Security diagnostics must not suggest weakening safety casually. For
`std.script` hard-off, the fix is to remove or explicitly authorize the named
capability in operator configuration, not to trust prompt text or provider
output.

## Runtime Diagnostics

Runtime diagnostics are durable records tied to events, effects, runs,
assertions, or provider boundaries.

They should include:

```text
diagnostic_id
instance_id
event_id?
effect_id?
run_id?
rule_id?
source_span?
code
severity
message
recoverability
retry_hint?
provider?
evidence_refs[]
redacted_fields[]
```

Recoverability:

```text
retryable       same runtime request may succeed later
configuration   operator/profile/provider config must change
source_change    workflow source or package lock must change
external         external system must change
terminal         intentionally terminal for this workflow path
unknown          no safe automated recommendation
```

Runtime diagnostics should distinguish:

- workflow-authored `fail error` terminal outcomes
- assertion failures
- provider failures
- provider output validation failures
- policy denials
- stale completions
- cancellation/revision conflicts
- report/admission failures

## Provider Diagnostics

Provider diagnostics must be normalized before they reach users.

Requirements:

- capture raw provider stderr/SDK errors as bounded evidence, not as the primary
  message
- classify boundary phase and error kind
- redact secrets, paths outside allowed roots when necessary, tokens, prompts
  marked sensitive, and provider credentials
- attach provider feature reports when a requested feature is unavailable
- attach version/probe information for Codex, Claude, and other harnesses
- distinguish "provider cannot do this" from "provider failed while doing this"

Example:

```text
error[provider.feature_unavailable]: Codex provider `codex-main` does not report slash command `/goal`
  --> workflow.whip:52:8
   |
52 |   tell coder with command "/goal Review this migration"
   |        ^^^^^ this turn requires a Codex feature not available in the active provider
   |
   = select a Codex provider variant that reports `/goal`, or remove the command requirement
```

## Package And Extension Contract

Package authors interact with diagnostics through declarative metadata and
fixtures.

They provide:

- construct labels
- field labels
- resource labels
- port labels
- capability explanations
- provider feature explanations
- examples
- docs anchors
- invalid fixtures with expected diagnostic codes

They do not provide:

- custom renderers
- arbitrary compiler strings
- hidden validation code that emits unchecked diagnostics
- source spans fabricated after parsing
- diagnostics that assert graph acceptance, lowering acceptance, or authority

The platform owns:

- parser recovery
- source spans
- diagnostic codes
- rendering style
- JSON report shape
- package diagnostic metadata validation
- construct graph resolution traces
- lowering and lifecycle provenance
- capability and runtime policy classification

## Reports

JSON reports should carry complete diagnostic structure. Text output should be a
rendering of the same object, not a separate path.

Minimum report requirements:

- stable `code`
- `severity`
- human `title` or `message`
- primary source span when available
- secondary spans or related refs when useful
- provenance layer and owner
- package/construct refs for construct errors
- runtime/effect/run refs for runtime errors
- suggestions and safe fixits when available
- redaction metadata

During migration, older report schemas may keep `message`, `suggestion`, and
`source_span` only. New schemas should move toward `diagnostic_v1` or an
equivalent shared definition used by check, compile, dev, accept, construct
graph, lowered IR, package check, and runtime diagnostics.

## Testing Diagnostics

Diagnostics are product behavior and need tests.

Required tests:

- parser invalid fixtures pin codes, primary spans, and suggestions
- type/expression invalid fixtures pin receiver types, expected/found domains,
  and did-you-mean suggestions
- package-manager invalid fixtures pin package lock and path diagnostics
- package-contract invalid fixtures pin missing metadata and authority
  diagnostics
- construct graph invalid fixtures pin resolution traces and user-facing
  messages for missing, ambiguous, incompatible, and cardinality errors
- lowering invalid fixtures pin package-author-facing `--explain` detail
- security fixtures pin hard-off behavior and do not allow prompt/payload
  smuggling
- runtime fixtures pin recoverability, effect/run refs, evidence refs, and
  redaction metadata
- provider smoke tests pin provider-feature-unavailable diagnostics separately
  from provider execution failures

Tests should pin stable fields and avoid freezing every word. Wording can
improve; diagnostic meaning should not drift.

## Formal Diagnostic Adequacy

Formal modeling should cover diagnostic adequacy, not diagnostic helpfulness.
Maude and TLA+ should not try to prove that prose is good. They should prove
that the implementation cannot reject silently, cannot attach diagnostics to
unrelated failures, and cannot let packages fake diagnostic completeness.

Diagnostic adequacy has four obligations:

```text
completeness  every modeled rejection path emits or requires a diagnostic
soundness     every modeled diagnostic corresponds to a real failed invariant
provenance    every modeled diagnostic cites the rejected object or transition
ownership     only checker/runtime-owned facts can satisfy diagnostic adequacy
```

### Static Maude Obligations

Use Maude for static pipeline diagnostics where acceptance already depends on
finite construct, package-contract, and lowering facts.

Required modeled cases:

```text
missing required port
  -> construct.missing_requirement

two providers for exactly-one or optional-one
  -> construct.ambiguous_resolution

wrong resource identity
  -> construct.resource_mismatch

wrong type, phase, or version
  -> construct.incompatible_type | construct.incompatible_phase |
     construct.incompatible_version

reserved keyword without platform privilege
  -> construct.reserved_keyword

unsupported lowering class
  -> lowering.unsupported_class

duplicate lowered core-object owner
  -> lowering.duplicate_core_object_owner

lowering emits forbidden runtime state
  -> lowering.forbidden_runtime_object

event source lowered as event occurrence
  -> lowering.signal_source_as_occurrence

package-supplied acceptance or diagnostic-complete fact
  -> rejected or ignored
```

Useful searches:

```text
search rejectedGraph(G) /\ noDiagnostic(G) .
  expected: no solution

search graphAccepted(G) /\ errorDiagnostic(G) .
  expected: no solution for graph-acceptance diagnostics

search diagnostic(G, D) /\ noFailedInvariant(G, D) .
  expected: no solution

search diagnosticComplete(G) supplied only by package facts .
  expected: no solution
```

The model should track diagnostic codes and provenance classes, not rendered
text. Human wording belongs to fixtures and snapshot tests.

### Runtime And TLA+/Trace Obligations

Runtime diagnostic adequacy is about durable lifecycle evidence.

Required modeled or trace-checked cases:

```text
capability denial
  -> diagnostic, no provider run

assertion failure or error
  -> diagnostic/evidence, no user fact/effect mutation

provider failure
  -> terminal failure diagnostic or terminal evidence

provider output validation failure
  -> failed effect and validation diagnostic

stale completion
  -> diagnostic/evidence, no authoritative completion

script disabled
  -> security.script_disabled diagnostic, no exec boundary crossed

cancel/revision conflict
  -> diagnostic/evidence, no new unauthorized work
```

TLA+ should model the durable lifecycle obligations: if a transition is rejected
or classified as non-authoritative, the event log records the diagnostic or
evidence needed to explain it. Trace conformance should check concrete runtime
artifacts for the same relationship.

### Fixture Obligations

Formal checks prove adequacy. Fixtures prove quality.

Invalid-source and report fixtures should assert:

- diagnostic code
- primary source span when source exists
- relevant secondary spans or related refs
- package-domain labels for construct errors
- resolution trace for construct graph errors
- recoverability for runtime/provider errors
- redaction metadata for sensitive provider/runtime errors
- safe suggestion or fixit when the repair is local enough

Text snapshots may check representative rendered messages, but they should not
freeze every word of every diagnostic.

## Acceptance Criteria

This strategy is implemented when:

- all new diagnostics use stable codes
- every source diagnostic has a primary span when source exists
- check/compile/dev/package/accept/report-admission diagnostics share one
  conceptual object model
- construct graph errors include resolution traces in JSON and package-domain
  messages in text
- package contracts include diagnostic metadata for exported constructs
- package authors cannot bypass platform rendering
- runtime/provider diagnostics include recoverability classification and
  redaction metadata
- formal/static diagnostic adequacy checks cover completeness, soundness,
  provenance, and ownership for construct graph and lowering rejection paths
- runtime trace/lifecycle checks cover diagnostic/evidence emission for denied
  or non-authoritative transitions
- security diagnostics include malicious negative fixtures
- acceptance fixtures can assert diagnostic codes and counts without relying on
  fragile prose
- `--explain` exposes provenance without overwhelming default output

## Relationship To User Docs

[`../docs/diagnostics.md`](../docs/diagnostics.md) is the user-facing repair
guide. This spec defines the platform contract that should make that guide
shorter over time: each diagnostic should carry enough context that the guide
explains categories and uncommon cases rather than compensating for poor
messages.
