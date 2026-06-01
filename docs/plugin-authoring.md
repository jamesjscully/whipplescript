# Plugin Authoring

Plugins extend WhippleScript by registering capabilities, providers, profiles,
bindings, schemas, resources, and optional skills.

For most users, the important rule is simple: plugins should expose explicit
effects. They should not hide orchestration policy outside the workflow.

## Source Shape

Import a plugin:

```whip
use memory
```

Call a capability:

```whip
call memory.query for item as context

after context succeeds {
  tell worker as turn "Use this context: {{ context.summary }}"
}
```

The `call` creates a durable effect. A provider handles it later.

## What Plugins Define

Plugin manifests may define:

- capability names and schemas
- provider bindings for effect execution
- profiles and authority boundaries
- resources and prompt templates
- optional skills that can be attached to agents or turns

## Current Status

Plugin packaging and provider configuration are still early. Use this page as a
user-facing orientation, then read the design-facing
[Plugin Author Guide](../spec/plugin-author-guide.md) for the current manifest
shape and policy details.
