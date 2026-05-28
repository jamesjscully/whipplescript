# Plugin System

Status: draft

Whippletree's plugin model is inspired by Pi's extension and package architecture:
a small host API, package-level resource discovery, lifecycle hooks, and
registered tools/resources. Whippletree should borrow the shape, but not Pi's full
process authority model.

Pi extensions run as TypeScript modules inside the agent process and can
register hooks, tools, commands, providers, resources, and persistent session
entries. That is excellent for hacker extensibility. Whippletree's target includes
enterprise and nontechnical users, so plugins must expose authority through
declared capabilities and durable effects.

## Design Principle

```text
Pi extension = arbitrary code in the agent process
Whippletree plugin = declared capability provider with explicit authority
```

The host remains small. Plugins extend available effects and resources, not the
rule language's execution semantics.

## Package Shape

An Whippletree package may contain:

```text
whippletree.json
plugins/
skills/
prompts/
schemas/
policies/
bin/
docs/
```

Example manifest:

```json
{
  "name": "whippletree-memory-sqlite",
  "version": "0.1.0",
  "whippletree": {
    "plugins": ["plugins/memory.wasm"],
    "skills": ["skills/memory/SKILL.md"],
    "effects": ["memory.query", "memory.write", "memory.consolidate"],
    "facts": ["memory.record", "memory.queryResult"],
    "capabilities": ["memory"],
    "statusPanels": ["memory.summary"]
  }
}
```

The exact executable format is open. Good candidates:

- WASM/WASI module
- constrained sidecar process over stdio
- local CLI capability adapter
- trusted in-process Rust plugin, only for first-party core components

In-process TypeScript is not the default target because it blurs the authority
boundary.

## Plugin Registrations

Plugins may register:

```text
effect kinds
fact schemas
event schemas
capability bindings
skills
prompt templates
artifact renderers
status panels
verification summaries
```

Plugins may not:

```text
add new rule-control semantics
mutate instance facts outside a rule commit
execute effects inline during rule evaluation
silently access credentials outside declared policy
silently inject context into agent turns without provenance
```

Plugin registrations extend typed surfaces; they do not grant direct write
access to instance truth. A plugin may register fact schemas and produce
observations, but those observations enter an instance only through
kernel-mediated events, effects, or projections.

Hard boundary:

```text
plugin code -> registered effect/capability -> kernel event/projection -> facts
```

Never:

```text
plugin code -> direct mutation of L, F, Q, or D
```

## Hooks

Whippletree should expose lifecycle hooks, but hooks must produce durable records
or registered resources rather than hidden runtime mutation.

Initial hook families:

```text
program.loaded
instance.started
instance.paused
instance.completed
effect.queued
effect.claimed
effect.completed
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

Like Pi's `resources_discover`, plugins may contribute resources dynamically:

```text
skills
prompt templates
schemas
status panels
artifact renderers
```

Discovery must be visible in `whip status` and `whip plugins`.

## Plugin State

Plugin state must be explicit:

- instance-local state lives in Whippletree facts/effects/artifacts
- plugin-local cache lives under a plugin-specific runtime store
- committed project state belongs to external kernels such as Loft or Thoth

Plugins must declare whether state is authoritative, cache, or local runtime
coordination.

## Security

Every plugin declares requested authority:

```text
filesystem
network
process execution
credentials
capability dependencies
artifact retention
```

The capability registry decides whether that authority is granted. Governed
environments should fail closed when a plugin cannot be sandboxed or audited at
the requested level.
