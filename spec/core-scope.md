# Core Scope

Status: draft

Armature should keep a small core in the Pi style: enough stable machinery to
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
BAML-backed coerce
Docket integration
human review inbox
artifact/evidence records
observability/status views
```

These are core because they define the trust boundary, static-analysis target,
or minimum useful local workflow.

## Plugin By Default

Most domain-specific systems should be plugins:

```text
memory systems
Thoth governance
GitHub / Linear / Jira integrations
browser automation
web research
notification systems
custom dashboards
specialized evaluators
repo-specific tools
```

Plugins may register capabilities, effects, fact schemas, skills, status
panels, and artifact renderers. They must not add arbitrary control flow to the
restricted rule language, and they must not directly mutate instance facts.

## Core Integration Standard

A feature earns core integration only if at least one is true:

1. The compiler/analyzer must understand it to preserve safety.
2. The runtime must enforce it to prevent duplicated effects or lost work.
3. Every practical agent workflow needs it.
4. It defines a stable substrate other plugins depend on.

Everything else should begin as a plugin.

## OpenClaw-Lite

"OpenClaw-lite" is an example capability composition, not a separate product
mode and not a language feature. A small Armature script plus core registries
should be able to provide:

```text
skills
scheduled heartbeat events
agent harness turns
memory plugin access
Docket work claims
human review
artifact/evidence tracing
```

without a large gateway process.
