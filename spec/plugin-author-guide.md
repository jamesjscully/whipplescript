# Plugin Author Guide

Status: draft

Plugins extend WhippleScript by registering capabilities, providers, profiles, and
bindings. They do not add new control-flow semantics.

## Manifest Shape

See `examples/plugins/memory.json` for a checked example.

Top-level fields:

```text
plugin_id
name
version
capabilities
providers
profiles
bindings
```

Capabilities describe authority and input contracts. Providers bind an effect
kind to an executable provider. Profiles define allowed capabilities. Bindings
grant a program or all programs access to a provider for a capability.

## Effect Design

Use namespaced effect kinds:

```text
memory.query
memory.write
thoth.verify
notification.send
```

Provider runs must produce evidence and terminal facts through the kernel. Do
not mutate workflow state directly from plugin code.

## Language Surface

Use generic capability calls first:

```whipplescript
call memory.query for item as context

after context succeeds {
  tell worker "Use {{ context.summary }}"
}
```

Do not introduce plugin-specific sequencing or hidden context injection.

## Policy

Every plugin capability should have:

- a clear authority name
- a narrow input schema
- a profile with least privilege
- explicit bindings
- evidence explaining what was read, written, or decided

Enterprise deployments should review plugin manifests for authority escalation,
credential handling, filesystem access, network access, and retention policy.
