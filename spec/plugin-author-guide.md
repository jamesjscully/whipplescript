# Package Author Guide

Status: draft

Packages extend WhippleScript by registering capabilities, providers, profiles,
bindings, and source-level library contracts. The author-facing boundary is
package/library/provider. Packages do not add new control-flow semantics.
They also do not add retry semantics, durable execution boundaries, direct fact
writes, hidden context injection, or cross-run storage mutation. Package
features must fit an already accepted platform extension class; otherwise the
package is rejected until the platform contract changes.

## Manifest Shape

See `examples/packages/notes.json` (third-party) and
`std/manifests/memory.json` (the embedded `std.memory` seed) for the
checked first-class package shape.

Top-level fields:

```text
schema = whipplescript.package_manifest.v0
package_id
name
version
libraries
capabilities
providers
profiles
bindings
```

Libraries declare importable source contracts and effect contracts.
Capabilities describe authority and input/output schemas. Providers bind a
capability or effect contract to an executable provider kind. Profiles define
allowed capabilities. Bindings grant a program or all programs access to a
provider for a capability.

Validate and pin a package manifest before using it. For one-off development,
the low-level commands are:

```sh
whip package check examples/packages/notes.json
whip package lock --output whip.lock examples/packages/notes.json
```

`package lock` and `package sync` refuse a manifest whose name claims the
reserved `std.*` namespace: std packages ship embedded in the platform and
cannot be provided by a package lock.

For project use, the target workflow is a package set plus sync:

```json
{
  "schema": "whipplescript.package_set.v0",
  "packages": [
    {
      "name": "notes",
      "source": {"type": "path", "path": "examples/packages/notes.json"}
    }
  ]
}
```

```sh
whip package sync
```

Then compile or run workflows against the discovered lock:

```sh
whip --json compile workflow.whip
whip dev workflow.whip
```

The lock records each manifest source, package id, package name/version, and
exact manifest SHA-256. A workflow that imports `use notes` resolves to the
locked `notes` package. (The reserved `std.*` namespace is the exception: std
packages ship embedded in the platform, and a lock claiming a `std.*` name
fails to load.) Runtime commands load the locked manifest into the
store before provider policy checks. See
[`package-management.md`](package-management.md) for the project package-set and
lock discovery contract.

## Effect Design

Use namespaced effect contract ids:

```text
memory.recall
memory.learn
github.comment
notification.send
```

For generic capability calls, the effect contract id is namespaced, but the
core effect kind is `capability.call`:

```json
{
  "id": "memory.recall",
  "effect_kind": "capability.call",
  "source_forms": ["call memory.recall"],
  "input_schema": {"query": "string"},
  "output_schema": {"summary": "string", "target": "string"},
  "validation": "runtime_boundary",
  "required_capabilities": ["memory.recall"]
}
```

Provider runs must produce evidence and terminal facts through the kernel. Do
not mutate workflow state directly from package/provider code.

`validation: runtime_boundary` means the worker validates provider output
against the locked `output_schema` before deriving
`capability.call.succeeded`. Invalid output fails the effect with a provider
output validation diagnostic. If an `output_schema` is present and validation is
omitted, first-class package manifests default to runtime-boundary validation.

The current package schema fragment is intentionally small:

```text
"string", "int", "number", "bool", "null", "json", "any"
{"field": "string", "other": "int"}     exact required object fields
["string"]                              homogeneous array
```

Use explicit repair or coercion effects for loose provider output instead of
quietly widening the package contract.

## Language Surface

Use generic capability calls first:

```whipplescript
call memory.recall for item as context

after context succeeds {
  tell worker "Use {{ context.summary }}"
}
```

`call memory.recall` lowers to the core `capability.call` effect and requires
the package capability `memory.recall`. Do not introduce package-specific
sequencing, hidden retries, direct fact writes, or hidden context injection.

In the current generic-call phase, package effect contracts must use
`effect_kind: "capability.call"`. `whip package check` and `whip package lock`
reject contracts that name undeclared required capabilities, providers whose
capability is not declared by the package, profiles that allow undeclared
capabilities, or bindings whose provider kind does not implement the bound
capability. Non-`capability.call` effect lowering remains outside the accepted
package contract.

Packages can register metadata-only library-owned syntax for tooling, or the
accepted executable `capability_call` lowering target for fixed rule-body
forms. A `capability_call` form must name a declared `target_capability` with a
matching `capability.call` effect contract:

```json
{
  "id": "memory.recall",
  "keyword": "recall",
  "scope": "rule_body",
  "grammar": {
    "shape": "effect_operation",
    "keyword": "recall",
    "slots": [
      {"name": "pool", "kind": "identifier"},
      {"name": "query", "kind": "expression", "connective": "for"}
    ],
    "payload": null,
    "binding": "required",
    "target_capability": "memory.recall"
  },
  "lowering_target": "capability_call",
  "target_capability": "memory.recall"
}
```

The `grammar` object (spec/construct-grammar.md, DR-0011) is the construct's
parse shape; the older flat `fields[]` array is now derived from it (slots,
then payload fields, then the binding) and may not be declared alongside it.
Grammar-less constructs may still declare `fields[]` directly.

With a package lock that imports `memory`, the source form is valid:

```whip
recall project_memory for item as context
```

The compiler records construct-use metadata but lowers the executable work to
ordinary `capability.call` requiring `memory.recall`. Without a package lock
that authorizes the form, `check`, `compile`, and workflow start reject the
source. The checker also rejects reserved core keywords, unknown scopes,
unknown field kinds, `metadata_only` forms with a target capability, and
`capability_call` forms without a declared target.

Package authors do not get a bespoke approval path for new semantics. New
lowering targets, non-`capability.call` effect kinds, scheduler behavior,
custom retry behavior, or custom durable storage admission require a
platform-owned extension class with updated specs, validators, tests, and model
coverage before a package can use them.

## Diagnostics

Package diagnostics are declarative metadata rendered by the platform. See
[`error-handling.md`](error-handling.md) for the shared diagnostic contract.
See [`editor-tooling.md`](editor-tooling.md) for completion, hover, code action,
and lint metadata.

For every exported source construct, package authors should provide:

- construct, field, resource, port, and capability labels
- provider feature labels when the construct depends on provider support
- one short example declaration or operation
- common fix templates for missing requirements and ambiguity
- docs anchors
- invalid fixtures with expected diagnostic codes
- completion snippets and hover summaries for exported constructs

Packages must not provide custom diagnostic renderers, arbitrary compiler
strings, hidden validation code that emits unchecked diagnostics, fabricated
source spans, or diagnostics that claim graph acceptance, lowering acceptance,
runtime authority, editor plugins, arbitrary lint rules, or language-server
code. If a package construct cannot be explained through the shared diagnostic
and editor metadata model, the package contract is incomplete.

## Test Fixtures And Risk Utilities

Runtime-facing package surfaces must expose deterministic fixture outcomes for
user workflow tests. See [`workflow-testing.md`](workflow-testing.md) for the
standard risk utility vocabulary.

The platform derives required outcomes from the declared surface class:

```text
effect_operation  succeeds, fails, times_out
signal_source     valid, duplicated, malformed, unauthorized
resource_read     exists, missing, permission_denied
message_send      sent, delivery_failed, unsupported_feature
agent_turn        succeeds, fails, times_out, cancelled,
                  returns_invalid_output, feature_unavailable
```

Packages may add domain-specific aliases, but they must still map back to the
platform vocabulary so tests remain portable across packages.

For each runtime-facing surface, package authors should declare:

- surface id and surface class
- required fixture outcomes implemented
- deterministic fixture response shape
- terminal, retryable, or branchable status for each outcome
- diagnostic code for failure, denial, or malformed-input outcomes
- projection changes produced by each outcome
- conformance fixtures for standard-package surfaces

`whip package check` should reject standard package contracts whose executable
surfaces do not provide the required fixture outcomes. Third-party packages may
start with warning-level enforcement during migration, but the target contract
is strict: if workflow tests can call or stub the surface, its fixture behavior
must be declared.

## Policy

Every package capability should have:

- a clear authority name
- a narrow input schema
- a declared output schema when successful output is consumed by rules
- runtime-boundary validation for provider output
- a profile with least privilege
- explicit bindings
- evidence explaining what was read, written, or decided

Enterprise deployments should review package manifests for authority escalation,
credential handling, filesystem access, network access, and retention policy.
