# Core Scope

Status: draft, terminology under revision

Direction note: this file predates the standard-library split proposed in
[`decision-records/0007-core-standard-libraries-and-providers.md`](decision-records/0007-core-standard-libraries-and-providers.md).
The current direction is a smaller kernel plus first-party standard libraries
such as `std.tracker`, `std.coord`, `std.ingress`, and `std.messaging`.

WhippleScript should keep a small core in the Pi style: enough stable machinery to
compose serious agent systems, without baking every useful integration into the
language or daemon.

## Core

The core includes:

```text
rule language
compiler / analyzer / verifier
program and instance control plane
runtime store
event log
fact projection
durable effect outbox
agent harness interface
capability registry
skill registry
typed schema-coercion effects (`coerce` / `decide`) and schema-coercion backend ABI
artifact/evidence records
status views
```

These are core because they define the trust boundary, static-analysis target,
or minimum useful local workflow.

## Package/Provider By Default

Most domain-specific systems should be packages, libraries, or providers:

```text
memory systems
standard tracker providers (GitHub / Linear / Jira)
browser automation
web research
communication channels and notification systems
custom dashboards
specialized evaluators
repo-specific tools
```

Packages may register library contracts, capabilities, providers, fact schemas,
skills, status panels, and artifact renderers. They must not add arbitrary
control flow to the restricted rule language, and providers must not directly
mutate instance facts.

## Core Integration Standard

A feature earns core integration only if at least one is true:

1. The compiler/analyzer must understand it to preserve safety.
2. The runtime must enforce it to prevent duplicated effects or lost work.
3. Every practical agent workflow needs it.
4. It defines a stable substrate other packages/providers depend on.

Everything else should begin as a package/provider.

## OpenClaw-Lite

"OpenClaw-lite" is an example capability composition, not a separate product
mode and not a language feature. A small WhippleScript script plus core registries
should be able to provide:

```text
skills
scheduled heartbeat events
agent harness turns
memory package access
tracker claims
typed signals
messaging channels
artifact/evidence tracing
```

without a large gateway process.
