# Runtime Provider Registry

Status: legacy implementation note; public plugin system retired

This file preserves the old plugin-system design context, but the active
extension model is now package/library/provider:

```text
package   locked installable distribution unit
library   compile-time source meaning and construct contracts
provider  runtime implementation behind durable effects
```

There should not be a separate author-facing plugin system. The remaining
implementation pieces historically called "plugin" are the runtime
provider-registration substrate: capability schemas, effect providers, profiles,
bindings, and compatibility manifest loading.

The active boundary is specified in:

- [`decision-records/0006-libraries-packages-providers-and-exec.md`](decision-records/0006-libraries-packages-providers-and-exec.md)
- [`decision-records/0010-package-library-provider-boundary.md`](decision-records/0010-package-library-provider-boundary.md)
- [`decision-records/0011-controlled-library-grammar-extensions.md`](decision-records/0011-controlled-library-grammar-extensions.md)
- [`construct-grammar.md`](construct-grammar.md)
- [`construct-graph-calculus.md`](construct-graph-calculus.md)

Current implementation note: `whip package check` and `whip package lock`
accept first-class `whipplescript.package_manifest.v0` package manifests,
derive contract-registry entries, and pin manifests by SHA-256. Old
plugin-shaped manifests are not part of the package contract. Runtime commands
can load the same lock with `--package-lock`. The store uses package
registration names; legacy plugin-shaped fixture manifests are compatibility
inputs, not a separate extension model.

The emerging construct grammar adds a compile-time layer above this runtime
registry:

```text
construct/library registry = source meaning, interfaces, effect contracts,
                             deterministic lowering
runtime provider registry  = authority, providers, profiles, bindings,
                             execution policy
package                    = installable unit that can contain both
```

The runtime provider registry remains necessary. The plugin system as a public
language-extension layer does not.

## Historical Context

The old WhippleScript plugin model was inspired by Pi's extension and package
architecture: a small host API, package-level resource discovery, lifecycle
hooks, and registered tools/resources. WhippleScript should borrow the
installation and registration shape, but not Pi's full process-authority model.

Pi extensions run as TypeScript modules inside the agent process and can
register hooks, tools, commands, providers, resources, and persistent session
entries. That is excellent for hacker extensibility. WhippleScript's target
includes enterprise and nontechnical users, so providers must expose authority
through declared capabilities and durable effects.

## Design Principle

```text
Pi extension = arbitrary code in the agent process
WhippleScript provider = declared durable-effect runner with explicit authority
```

The host remains small. Packages and providers extend available effects,
resources, and runtime policy; they do not extend the rule language's execution
semantics. Source-level syntax and construct composition belong to the
construct/library registry.

## Package Shape

An WhippleScript package may contain:

```text
package manifest
libraries/
providers/
skills/
prompts/
schemas/
policies/
bin/
docs/
```

First-class manifests are shaped around package/library/provider fields, not a
single plugin bucket:

```json
{
  "schema": "whipplescript.package_manifest.v0",
  "package_id": "memory",
  "name": "whipplescript-memory-sqlite",
  "version": "0.1.0",
  "libraries": [],
  "capabilities": [],
  "providers": [],
  "profiles": [],
  "bindings": []
}
```

The exact executable format is open. Good candidates:

- WASM/WASI module
- constrained sidecar process over stdio
- local CLI capability adapter
- trusted in-process Rust provider, only for first-party components

In-process TypeScript is not the default target because it blurs the authority
boundary.

## Runtime Registrations

Packages and providers may register runtime surfaces:

```text
effect kinds
fact schemas
event schemas
capability bindings
capability schemas
effect providers
profiles
skills
prompt templates
artifact renderers
status panels
verification summaries
```

Source files historically imported plugin-like surfaces with the short form:

```whip
use memory
```

Under the package/library split, `use memory` resolves source library surface
through the package lock. Runtime commands then register the locked package
manifest into the runtime provider registry before provider policy checks.
`use` is not how agent skills are attached and does not grant runtime authority
by itself.

Packages and providers may not:

```text
add new rule-control semantics
mutate instance facts outside a rule commit
execute effects inline during rule evaluation
silently access credentials outside declared policy
silently inject context into agent turns without provenance
```

Runtime registrations extend typed surfaces; they do not grant direct write
access to instance truth. A provider may register fact schemas and produce
observations, but those observations enter an instance only through
kernel-mediated events, effects, or projections.

Hard boundary:

```text
provider code -> registered effect/capability -> kernel event/projection -> facts
```

Never:

```text
provider code -> direct mutation of L, F, Q, or D
```

## Hooks

Lifecycle hooks remain an open runtime-provider design topic, not a package
language-extension power. Any accepted hook must produce durable records or
registered resources rather than hidden runtime mutation.

Initial hook families:

```text
program.loaded
instance.started
instance.paused
instance.completed
effect.queued
effect.run_started
effect.terminal
agent.turn.before
agent.turn.after
status.render
resources.discover
```

Hook outputs are typed:

```text
additional skills/prompts
capability registrations
context bundles
artifact renderers
diagnostics
```

Hooks must not mutate workflow state directly. If a hook discovers information
that should affect a workflow, it must return a typed diagnostic/resource or
request a registered effect/event path that the kernel can record.

## Resource Discovery

Like Pi's `resources_discover`, packages/providers may contribute resources
dynamically:

```text
skills
prompt templates
schemas
status panels
artifact renderers
```

Discovery must be visible in status and future package/provider inspection
commands. A command named `whip plugins` would be legacy compatibility, not the
preferred public shape.

## Provider And Package State

Provider/package state must be explicit:

- instance-local state lives in WhippleScript facts/effects/artifacts
- provider-local cache lives under an explicitly declared runtime store
- committed project state belongs to package-owned or external kernels/providers
  such as tracker systems or hosted integration providers

Packages/providers must declare whether state is authoritative, cache, or local
runtime coordination.

## Security

Every package/provider declares requested authority:

```text
filesystem
network
process execution
credentials
capability dependencies
artifact retention
```

The capability registry decides whether that authority is granted. Governed
environments should fail closed when a provider cannot be sandboxed or audited
at the requested level.
