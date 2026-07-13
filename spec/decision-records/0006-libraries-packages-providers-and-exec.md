# 0006: Libraries, Packages, Providers, and `exec`

Status: accepted vocabulary baseline; implementation cleanup active

## Decision

WhippleScript should not present `exec`, plugins, and libraries as three equal
authoring concepts. The cleaner model is:

```text
library   source-level reusable WhippleScript surface
provider  executable implementation behind an effect
package   distribution and registration unit
exec      the default provider implementation for pinned local scripts
plugin    legacy/internal term for runtime provider-registration code
```

Ordinary authors should mostly see libraries and providers. "Plugin" should
not be a distinct language concept.

This record defines the vocabulary. The formal ABI and authority boundary are
specified in
[`0010-package-library-provider-boundary.md`](0010-package-library-provider-boundary.md).

## Definitions

### Library

A library is analyzable WhippleScript surface:

```text
types
events
fact schemas
workflow declarations
rules / flows / patterns
capability requirements
provider declarations
skills / prompts / examples / docs
```

Libraries may introduce names and reusable orchestration policy. They do not
execute code by themselves.

### Provider

A provider implements an effect contract:

```text
script capability
HTTP/API adapter
agent harness adapter
tracker provider
artifact/status renderer with explicit hooks
```

Provider execution is always mediated by the runtime through durable effects,
events, evidence, and the capability registry.

### Package

A package is the installable distribution unit. A package may contain:

```text
libraries
providers
script manifests
skills
prompts
schemas
docs
examples
native binaries or sidecars
```

### `exec`

Hosted `exec <capability> with <record> -> Type` is a provider implementation
for pinned local scripts. It should be the default implementation path for
small custom integrations because it already gives:

```text
typed JSON stdin
typed JSON/JSONL stdout
content pinning
capability gating
hash evidence
failure routing through ordinary effect lifecycle
```

## Rationale

The existing plugin language is too broad. It currently covers package layout,
source imports, capability registration, executable code, hooks, skills, and
status rendering. That makes it sound like WhippleScript needs a new execution
model for every extension.

Most extensions do not need that. A large class of integrations can be ordinary
libraries plus pinned script providers. Native provider code is justified only
when the integration needs capabilities that `exec` cannot provide.

## When `exec` Is Enough

Use a pinned script provider for request/response capabilities:

```text
run tests
classify a repo path set
call a simple API
transform files
produce a report
perform one tracker mutation
backup or deploy through a vetted script
```

The rule of thumb: if typed input, typed output, process exit status, and
captured evidence are enough, use `exec`.

## When Native Providers Are Needed

Use a native provider or sidecar when the capability needs:

```text
streaming events
long-lived sessions
bidirectional protocols
native cancellation or renewal
incremental artifact capture
credential/session lifecycle management
resource discovery hooks
high-volume watch/sync
custom status panels
provider-specific health checks
```

Agent provider adapters are the clearest example. Codex and Claude should
not be treated as plain `exec` wrappers because WhippleScript needs structured
turn lifecycle, session identifiers, streaming observations, approvals,
cancellation, and artifacts. Existing harness terminology should be treated as
the advanced endpoint-binding vocabulary, not the main package concept.

## Authoring Implications

Source should import stable semantic surfaces:

```whip
use std.tracker
use std.agent
```

Provider selection and authority should live in declarations and operator
configuration:

```whip
tracker backlog {
  provider github
}

agent coder {
  provider codex
  profile repo-writer
}
```

The environment decides whether those providers are installed, configured,
credentialed, and authorized.

## Relationship To Existing Plugin System

The plugin system should be dissolved as an author-facing extension path. The
remaining implementation substrate should be reframed as package/provider
registration:

```text
old "plugin imports"     -> library imports resolved through a package lock
old "plugin code"        -> provider implementation
old "plugin manifest"    -> package manifest
old "plugin capability"  -> package capability / effect contract
```

Providers and package runtime code may not:

```text
add rule-control semantics
mutate facts directly
bypass the event log
execute effects inline
silently inject context
silently access credentials
```

## Consequences

- The public docs should de-emphasize "plugin" as an author-facing concept.
- `use` should import libraries/packages, not imply executable authority.
- Hosted `exec` becomes the preferred custom-provider path.
- Native providers remain available for serious integrations.
- Package manifests should distinguish source libraries from executable
  providers and from script capabilities.
- Existing code paths named `plugin_*` are implementation debt, not a fourth
  extension mechanism.
- Project package management should start with a local package-set workflow:
  `whip.packages.json` plus `whip package sync` producing a portable
  `whip.lock`. Registry, resolver, install/update, publishing, and native
  provider artifact installation remain deferred.

## Open Questions

- Should the source keyword remain `use`, or should library imports get a more
  explicit form such as `import`?
- How should a package expose both a library and multiple providers without
  giving authors the impression that import grants authority?
- What is the minimal package manifest schema that cleanly separates
  libraries, providers, script capabilities, skills, and docs?
